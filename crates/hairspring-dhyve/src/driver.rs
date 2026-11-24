//! The session driver's logic: a probe, one prepare, then restores in a fixed literal order, some given the checkpointed
//! run's own input files (group C) and some given another seed file (group S), each read and counted. It runs the backend's
//! own prepare, restore and judge functions and adds only counting, the run order, the start rule, the gates and stops the
//! session's rules name, and its two private records: a steps log in the evaluator's form (with a row for each root command
//! the backend runs) and a record for each run. It compares nothing but equality of serial output and of event lines among
//! the C restores.
use crate::error::{InfraError, InfraKind};
use crate::fwcfg::{Inputs, GUEST_JOB};
use crate::judge::{judge_trial, observe, printed_seed, Judged, Observed};
use crate::prepare::Prepared;
use crate::session::Session;
use crate::spawn::SpawnLog;
use crate::steps::{error_text, Steps};
use crate::trial::TrialRun;
use crate::vmname::VmName;
use hairspring_engine::probe::{ProbeReport, ProbeStatus};
use hairspring_engine::verdict::Verdict;
use hairspring_props::limits::EVENT_PREFIX;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The cap of one restore in a session.
pub const TRIAL_CAP: Duration = Duration::from_secs(70);
/// What the start rule allows for a prepare and for a restore: the cap, the kill grace and the destroys around it.
const PREPARE_WORST: Duration = Duration::from_secs(445);
const RUN_WORST: Duration = Duration::from_secs(95);
/// The seed of the group that is given another seed file: one past the checkpointed run's.
const OTHER_SEED_STEP: u64 = 1;
const RECORDS_DIR: &str = "records";
/// What bhyve prints when the CPU cannot trap the random-number instruction.
const RDRAND_WARNING: &str = "RDRAND exit not supported";
/// The most serial lines a record keeps.
const MAX_SERIAL_SUFFIX: usize = 20_000;
/// A line is cut to this before it is kept, the longest line the event protocol takes.
const MAX_KEPT_LINE: usize = 4_096;

/// What to run.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub seed: u64,
    pub control_restores: u32,
    pub other_seed_restores: u32,
    /// A run starts only if it can end by this time.
    pub deadline: Option<Instant>,
    /// Go on although the prepare's standard error holds the RDRAND warning.
    pub rdrand_accepted: bool,
    /// The machine check, run before anything starts. `None` runs none.
    pub probe: Option<fn() -> ProbeReport>,
    pub prepare_worst: Duration,
    pub run_worst: Duration,
}

impl Plan {
    pub fn new(seed: u64, control_restores: u32, other_seed_restores: u32) -> Plan {
        Plan {
            seed,
            control_restores,
            other_seed_restores,
            deadline: None,
            rdrand_accepted: false,
            probe: None,
            prepare_worst: PREPARE_WORST,
            run_worst: RUN_WORST,
        }
    }
}

/// The counts of one group of restores.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Group {
    pub count: u32,
    pub counted: u32,
    pub resume_seen: u32,
    pub exit_reported: u32,
    pub verdicts: Vec<String>,
    pub errors: Vec<String>,
    serial_digests: Vec<String>,
    events_digests: Vec<String>,
}

/// What a session saw.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub prepare: Result<PrepareSummary, String>,
    pub control: Group,
    pub other_seed: Group,
    /// How the session ended: `DONE`, or `END:` and the reason.
    pub end: String,
    /// The names of the runs, in the order they ran.
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrepareSummary {
    pub stamp: String,
    pub checkpoint_sha256: Vec<String>,
    pub restored_pointer: String,
    pub seconds: f64,
}

/// The literal order of the restores: C, C, S, repeated, with what is left of either at the end. Ten and five give
/// `CCSCCSCCSCCSCCS`.
pub fn order(control: u32, other: u32) -> String {
    let (mut c, mut s) = (control, other);
    let mut out = String::new();
    while c > 0 || s > 0 {
        for _ in 0..2 {
            if c > 0 {
                out.push('C');
                c -= 1;
            }
        }
        if s > 0 {
            out.push('S');
            s -= 1;
        }
    }
    out
}

/// The code of the stop a fault of this kind ends a session with, or `None` when the fault is only that run's.
pub fn end_code(e: &InfraError) -> Option<&'static str> {
    match e.kind {
        InfraKind::CheckpointChanged => Some("END:CKPT-CHANGED"),
        InfraKind::InputFiles => Some("END:FWCFG-MISMATCH"),
        InfraKind::Refused => Some("END:ARGV-REFUSED"),
        _ => None,
    }
}

fn verdict_label(v: &Verdict) -> &'static str {
    match v {
        Verdict::Pass => "pass",
        Verdict::Violation(_) | Verdict::Deadlock(_) => "finding",
        Verdict::Hang(_) => "hang",
        Verdict::Inconclusive(_) => "unusable",
        Verdict::Infra(_) => "infra",
    }
}

fn equality(digests: &[String]) -> &'static str {
    match digests {
        [] | [_] => "not_compared",
        [first, rest @ ..] if rest.iter().all(|d| d == first) => "equal",
        _ => "differed",
    }
}

impl Group {
    fn record(&mut self, observed: &Observed, verdict: &str, counted: bool) {
        self.count += 1;
        self.counted += u32::from(counted);
        self.resume_seen += u32::from(observed.resume_seen);
        self.exit_reported += u32::from(observed.exit_reported);
        self.verdicts.push(verdict.to_string());
        self.serial_digests.extend(observed.serial_digest.clone());
        self.events_digests.extend(observed.events_digest.clone());
    }

    fn failed(&mut self, e: &InfraError) {
        self.count += 1;
        self.errors.push(e.to_string());
    }

    fn json(&self, compare: bool) -> Value {
        let mut v = json!({
            "count": self.count,
            "counted": self.counted,
            "resume_seen": self.resume_seen,
            "exit_reported": self.exit_reported,
            "verdicts": self.verdicts,
            "errors": self.errors,
        });
        if compare {
            v["serial"] = json!(equality(&self.serial_digests));
            v["events"] = json!(equality(&self.events_digests));
        }
        v
    }
}

impl Summary {
    pub fn to_json(&self) -> String {
        let prepare = match &self.prepare {
            Ok(p) => json!({
                "ok": true,
                "stamp": p.stamp,
                "checkpoint_sha256": p.checkpoint_sha256,
                "restored_pointer": p.restored_pointer,
                "seconds": p.seconds,
            }),
            Err(e) => json!({"ok": false, "error": e}),
        };
        json!({
            "prepare": prepare,
            "control": self.control.json(true),
            "other_seed": self.other_seed.json(false),
            "end": self.end,
            "names": self.names,
        })
        .to_string()
    }

    /// Whether the prepare worked, no restore met an infrastructure error, and the session ran to its end.
    pub fn clean(&self) -> bool {
        self.prepare.is_ok()
            && self.end == "DONE"
            && self.control.errors.is_empty()
            && self.other_seed.errors.is_empty()
    }
}

/// Where the session's private records are.
pub fn records_dir(session: &Session) -> PathBuf {
    session.layout.root().join(RECORDS_DIR)
}

/// The path of the steps log in `dir`.
pub fn steps_log(dir: &Path) -> PathBuf {
    dir.join("steps.tsv")
}

fn write_record(dir: &Path, name: &str, record: &Value) {
    let _ = std::fs::write(dir.join(format!("{name}.json")), record.to_string());
}

/// A line of serial text as a record keeps it: printable ASCII, whole up to the longest line the event protocol takes.
fn kept(line: &[u8]) -> String {
    line.iter()
        .take(MAX_KEPT_LINE)
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

fn serial_lines(run: &TrialRun) -> Vec<Vec<u8>> {
    run.serial
        .as_ref()
        .map_or_else(Vec::new, |s| s.lines.clone())
}

/// The scan outcome as a stable word: `seen`, `not_sent`, or `unseen:` and the reason.
fn scan_word(run: &TrialRun) -> String {
    use crate::director::ResumeResult::{NotSent, Seen, Unseen};
    match &run.control.resume {
        Seen => "seen".to_string(),
        NotSent => "not_sent".to_string(),
        Unseen(why) => format!("unseen:{}", why.replace(' ', "_")),
    }
}

fn hashes_object(pairs: &[(String, String)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(k, v)| (k.clone(), json!(v)))
            .collect::<Map<_, _>>(),
    )
}

fn keys_object(prepared: &Prepared) -> Value {
    let k = &prepared.record.keys;
    json!({
        "stop_seq": k.stop_seq, "branch_count": k.branch_count, "vtime_ns": k.vtime_ns,
        "probe_id": k.probe_id, "probe_arg0": k.probe_arg0, "vcpu_id": k.vcpu_id,
    })
}

fn state_files(run: &TrialRun) -> Value {
    let mut out = Map::new();
    for path in [
        &run.run.state_manifest,
        &run.run.state_regs,
        &run.run.state_mem,
        &run.run.state_devices,
    ] {
        if let (Some(name), Ok(meta)) = (path.file_name(), std::fs::metadata(path)) {
            out.insert(name.to_string_lossy().into_owned(), json!(meta.len()));
        }
    }
    Value::Object(out)
}

/// The per-run record: verdict and reason, scan outcome, exit report, the stored events and the serial suffix whole,
/// sizes, masked argument hash, input file hashes, restore keys, the restored pointer and the state files' sizes.
fn run_record(
    prepared: &Prepared,
    run: &TrialRun,
    group: char,
    position: usize,
    judged: &Result<Judged, InfraError>,
) -> Value {
    let (verdict, reason) = match judged {
        Ok(j) => (
            verdict_label(&j.verdict),
            match &j.verdict {
                Verdict::Pass => String::new(),
                Verdict::Violation(r)
                | Verdict::Deadlock(r)
                | Verdict::Hang(r)
                | Verdict::Inconclusive(r)
                | Verdict::Infra(r) => kept(r.as_bytes()),
            },
        ),
        Err(e) => ("infra", kept(e.to_string().as_bytes())),
    };
    let lines = serial_lines(run);
    let exit = run.serial.as_ref().and_then(|s| s.exit);
    let events: Vec<String> = lines
        .iter()
        .filter(|l| l.starts_with(EVENT_PREFIX.as_bytes()))
        .map(|l| kept(l))
        .collect();
    let hs_seed = lines
        .iter()
        .find_map(|l| l.strip_prefix(b"HS_SEED "))
        .map(kept);
    json!({
        "name": run.name.as_str(),
        "group": group.to_string(),
        "position": position,
        "verdict": verdict,
        "reason": reason,
        "scan": scan_word(run),
        "skipped_lines": run.control.skipped,
        "first_line": run.control.first_line,
        "hs_exit": exit.map(|e| json!({"code": e.code, "signal": e.signal})),
        "hs_seed": hs_seed,
        "events": events,
        "serial_suffix": lines.iter().take(MAX_SERIAL_SUFFIX).map(|l| kept(l)).collect::<Vec<_>>(),
        "serial_lines": lines.len(),
        "run_log_bytes": run.run_log_len,
        "uart_log_bytes": run.uart_len,
        "state_files": state_files(run),
        "argv_masked_sha256": run.argv_masked_sha256,
        "fwcfg_sha256": hashes_object(&run.input_hashes),
        "restore_keys": keys_object(prepared),
        "restored_pointer": format!("{:#x}", prepared.record.rip),
    })
}

/// The record of a restore that returned an error before a trial existed.
fn error_record(
    prepared: &Prepared,
    name: &str,
    group: char,
    position: usize,
    e: &InfraError,
) -> Value {
    json!({
        "name": name,
        "group": group.to_string(),
        "position": position,
        "verdict": "infra",
        "reason": kept(e.to_string().as_bytes()),
        "scan": "none",
        "restore_keys": keys_object(prepared),
        "restored_pointer": format!("{:#x}", prepared.record.rip),
    })
}

fn prepare_record(prepared: &Prepared) -> Value {
    json!({
        "name": prepared.name.as_str(),
        "stamp": prepared.stamp.as_str(),
        "checkpoint_sha256": prepared.record.hashes,
        "restored_pointer": format!("{:#x}", prepared.record.rip),
        "restore_keys": keys_object(prepared),
        "fwcfg_sha256": hashes_object(&prepared.input_hashes),
        "argv_masked_sha256": prepared.argv_masked_sha256,
        "seconds": prepared.elapsed.as_secs_f64(),
    })
}

/// Whether a restore is counted: it passed, its resume was seen, the guest reported its exit and printed the seed its arm
/// gives, its name has the session's form, and no log passed its bound. (That its input files read back as written is
/// settled before it starts: a restore whose files do not is an error, not a trial.)
fn counted(
    session: &Session,
    run: &TrialRun,
    observed: &Observed,
    label: &str,
    arm_seed: u64,
    position: usize,
) -> bool {
    let seed_ok = run.serial.as_ref().and_then(printed_seed) == Some(arm_seed);
    // A restore is run number position + 1 under the session's stamp (a restore past the stamp's names has a stamp of its own).
    let name_ok = VmName::parse(run.name.as_str()).is_some_and(|n| {
        n.run_number() as usize == position + 1 || n.stamp() != prepared_stamp(run)
    });
    let bounds_ok = session
        .bounds
        .breach(run.run_log_len, run.uart_len)
        .is_none();
    observed.resume_seen
        && observed.exit_reported
        && label == "pass"
        && seed_ok
        && name_ok
        && bounds_ok
}

fn prepared_stamp(run: &TrialRun) -> crate::vmname::Stamp {
    run.stamp.clone()
}

fn seed_for(plan: &Plan, group: char) -> u64 {
    if group == 'C' {
        plan.seed
    } else {
        plan.seed.wrapping_add(OTHER_SEED_STEP)
    }
}

fn start_allowed(plan: &Plan, worst: Duration) -> bool {
    plan.deadline.is_none_or(|d| Instant::now() + worst <= d)
}

/// The row of the probe: the gate and the rows it does not gate on.
fn probe_row(report: &ProbeReport) -> (bool, String) {
    let failing: Vec<&str> = report
        .rows
        .iter()
        .filter(|r| r.status == ProbeStatus::Fail)
        .map(|r| r.label.as_str())
        .collect();
    let pass = report.is_ready() && failing.is_empty();
    let status = |label: &str| {
        report
            .rows
            .iter()
            .find(|r| r.label == label)
            .map_or("na", |r| match r.status {
                ProbeStatus::Ok => "ok",
                ProbeStatus::Fail => "fail",
                ProbeStatus::Note => "note",
            })
    };
    let gate = if pass {
        "PASS".to_string()
    } else {
        format!("FAIL:{}", failing.join(","))
    };
    (
        pass,
        format!(
            "gate={gate} cpu={} module_kind={} access={} nmi={}",
            status("cpu"),
            status("module kind"),
            status("access"),
            status("nmi")
        ),
    )
}

/// Whether the prepare's standard error holds the warning; `None` when it cannot be read.
fn rdrand_in_prepare(prepared: &Prepared) -> Option<bool> {
    let bytes = std::fs::read(&prepared.prepare_run.run_log).ok()?;
    Some(String::from_utf8_lossy(&bytes).contains(RDRAND_WARNING))
}

struct Ended {
    prepare: Result<PrepareSummary, String>,
    control: Group,
    other_seed: Group,
    names: Vec<String>,
}

impl Ended {
    fn early(why: &str) -> Ended {
        Ended {
            prepare: Err(why.to_string()),
            control: Group::default(),
            other_seed: Group::default(),
            names: Vec::new(),
        }
    }
}

fn summary(steps: &Steps, ended: Ended, end: &str) -> Summary {
    steps.driver("END", "row", "-", "-", end);
    Summary {
        prepare: ended.prepare,
        control: ended.control,
        other_seed: ended.other_seed,
        end: end.to_string(),
        names: ended.names,
    }
}

/// Runs the plan. The session should be one that allows a single prepare and keeps everything; the driver binary makes it
/// so.
pub fn run_session(session: &Session, plan: &Plan) -> Summary {
    let dir = records_dir(session);
    let _ = std::fs::create_dir_all(&dir);
    let steps = Arc::new(Steps::new(&steps_log(&dir), session.layout.root()));
    let log: Arc<dyn SpawnLog> = steps.clone();
    let session = session
        .with_trial_cap(TRIAL_CAP.min(session.timings().trial_cap))
        .with_spawn_log(log);
    if let Some(probe) = plan.probe {
        if !start_allowed(plan, Duration::from_secs(35)) {
            return summary(
                &steps,
                Ended::early("not started: the session's time is used"),
                "END:WCAP",
            );
        }
        let (pass, detail) = probe_row(&probe());
        steps.driver("PROBE", "row", "-", "-", &detail);
        if !pass {
            return summary(
                &steps,
                Ended::early("not started: the machine check failed"),
                "END:PROBE",
            );
        }
    }
    if !start_allowed(plan, plan.prepare_worst) {
        return summary(
            &steps,
            Ended::early("not started: the session's time is used"),
            "END:WCAP",
        );
    }
    steps.driver("PREPARE", "start", "-", "-", "-");
    let inputs = Inputs {
        seed: plan.seed,
        job: GUEST_JOB,
    };
    let prepared = match session.prepare(inputs) {
        Ok(p) => p,
        Err(e) => {
            steps.driver(
                "PREPARE",
                "done",
                "-",
                "-",
                &format!("error={}", error_text(&e.to_string())),
            );
            let end = if e.kind == InfraKind::SecondPrepare {
                "END:REPREPARE"
            } else {
                end_code(&e).unwrap_or("END:PREPARE")
            };
            return summary(&steps, Ended::early(&e.to_string()), end);
        }
    };
    steps.driver(
        "PREPARE",
        "done",
        "-",
        "-",
        &format!(
            "name={} stamp={}",
            prepared.name.as_str(),
            prepared.stamp.as_str()
        ),
    );
    write_record(&dir, "prepare", &prepare_record(&prepared));
    let mut ended = Ended {
        prepare: Ok(PrepareSummary {
            stamp: prepared.stamp.as_str().to_string(),
            checkpoint_sha256: prepared.record.hashes.to_vec(),
            restored_pointer: format!("{:#x}", prepared.record.rip),
            seconds: prepared.elapsed.as_secs_f64(),
        }),
        control: Group::default(),
        other_seed: Group::default(),
        names: Vec::new(),
    };
    let warning = rdrand_in_prepare(&prepared);
    let (word, stop) = match warning {
        Some(false) => ("none", false),
        Some(true) => ("warning", !plan.rdrand_accepted),
        None => ("unreadable", !plan.rdrand_accepted),
    };
    let flag = if plan.rdrand_accepted { "yes" } else { "no" };
    steps.driver(
        "GR",
        "row",
        "-",
        "-",
        &format!("rdrand_p={word} flag={flag}"),
    );
    if stop {
        return summary(&steps, ended, "END:RDRAND");
    }
    let end = run_restores(&session, plan, &prepared, &steps, &dir, &mut ended);
    summary(&steps, ended, end)
}

fn run_restores(
    session: &Session,
    plan: &Plan,
    prepared: &Prepared,
    steps: &Steps,
    dir: &Path,
    ended: &mut Ended,
) -> &'static str {
    for (index, group) in order(plan.control_restores, plan.other_seed_restores)
        .chars()
        .enumerate()
    {
        if !start_allowed(plan, plan.run_worst) {
            return "END:WCAP";
        }
        let position = index + 1;
        let reference = format!("{group}{position}");
        let arm_seed = seed_for(plan, group);
        let ordinal = u32::try_from(index).unwrap_or(u32::MAX);
        let target = if group == 'C' {
            &mut ended.control
        } else {
            &mut ended.other_seed
        };
        steps.driver("TRIAL", "start", &reference, "-", "-");
        let trial = session.restore_trial(prepared, ordinal, prepared.inputs.with_seed(arm_seed));
        let (is_counted, stop) = match trial {
            Err(e) => {
                let name = VmName::new(&prepared.stamp, ordinal + 2)
                    .map_or_else(|_| "-".to_string(), |n| n.as_str().to_string());
                steps.driver(
                    "TRIAL",
                    "done",
                    &reference,
                    "-",
                    &format!("error={}", error_text(&e.to_string())),
                );
                write_record(
                    dir,
                    &name,
                    &error_record(prepared, &name, group, position, &e),
                );
                target.failed(&e);
                (false, end_code(&e))
            }
            Ok(trial) => {
                ended.names.push(trial.name.as_str().to_string());
                let observed = observe(&trial);
                let judged = judge_trial(&trial, session.timings().trial_cap.as_secs(), false);
                let label = judged
                    .as_ref()
                    .map_or("infra", |j| verdict_label(&j.verdict));
                write_record(
                    dir,
                    trial.name.as_str(),
                    &run_record(prepared, &trial, group, position, &judged),
                );
                steps.driver(
                    "TRIAL",
                    "done",
                    &reference,
                    "-",
                    &format!("name={} verdict={label}", trial.name.as_str()),
                );
                let ok = counted(session, &trial, &observed, label, arm_seed, position);
                match &judged {
                    Err(e) => target.failed(e),
                    Ok(_) => target.record(&observed, label, ok),
                }
                session.clean_trial(&trial, label != "pass");
                (ok, None)
            }
        };
        if let Some(code) = stop {
            return code;
        }
        if position == 1 && !is_counted {
            return "END:FIRST-NOT-OK";
        }
    }
    "DONE"
}

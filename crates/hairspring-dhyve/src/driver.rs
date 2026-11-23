//! The session driver's logic: one prepare, then restores in a fixed literal order, some given the checkpointed run's own
//! input files (group C) and some given another seed file (group S), each read and counted. It runs the backend's own
//! prepare, restore and judge functions and adds only counting, the run order, the start rule, the stop after a first
//! restore that is not counted, and two private records: a steps log and a record for each run. It compares nothing but
//! equality of serial output and of event lines among the C restores.
use crate::error::{InfraError, InfraKind};
use crate::fwcfg::Inputs;
use crate::judge::{judge_trial, observe, Observed};
use crate::prepare::Prepared;
use crate::runlog::shown;
use crate::session::Session;
use crate::trial::TrialRun;
use hairspring_engine::verdict::Verdict;
use hairspring_props::limits::EVENT_PREFIX;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The cap of one restore in a session, and what a restore is allowed on top of it in the start rule.
pub const TRIAL_CAP: Duration = Duration::from_secs(70);
const PER_RUN_WORST: Duration = Duration::from_secs(95);
/// The seed of the group that is given another seed file: one past the checkpointed run's.
const OTHER_SEED_STEP: u64 = 1;
const RECORDS_DIR: &str = "records";

/// What to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub seed: u64,
    pub control_restores: u32,
    pub other_seed_restores: u32,
    /// A restore starts only if it can end by this time.
    pub deadline: Option<Instant>,
}

/// The counts of one group of restores.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Group {
    pub count: u32,
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
    fn record(&mut self, observed: &Observed, verdict: &str) {
        self.count += 1;
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

/// The two private records: a steps log and a record for each run.
struct Records {
    dir: PathBuf,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Records {
    fn new(session: &Session) -> Records {
        let dir = session.layout.root().join(RECORDS_DIR);
        let _ = std::fs::create_dir_all(&dir);
        Records { dir }
    }

    /// One row: UTC seconds, the writer, the kind, a detail and a result.
    fn step(&self, kind: &str, detail: &str, result: &str) {
        let row = format!(
            "{}\tdriver\t{kind}\t{}\t{result}\n",
            now_secs(),
            shown(detail)
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("steps.tsv"))
        {
            let _ = f.write_all(row.as_bytes());
        }
    }

    fn run(&self, name: &str, record: &Value) {
        let _ = std::fs::write(self.dir.join(format!("{name}.json")), record.to_string());
    }
}

fn event_lines(run: &TrialRun) -> Vec<String> {
    run.serial.as_ref().map_or_else(Vec::new, |s| {
        s.lines
            .iter()
            .filter(|l| l.starts_with(EVENT_PREFIX.as_bytes()))
            .map(|l| shown(&String::from_utf8_lossy(l)))
            .collect()
    })
}

/// The per-run record: verdict and reason, scan outcome, exit report, stored events, sizes, masked argument hash, input
/// file hashes, restore keys and the restored pointer.
fn run_record(
    prepared: &Prepared,
    run: &TrialRun,
    group: &str,
    position: usize,
    judged: &Result<crate::judge::Judged, InfraError>,
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
                | Verdict::Infra(r) => shown(r),
            },
        ),
        Err(e) => ("infra", shown(&e.to_string())),
    };
    let exit = run.serial.as_ref().and_then(|s| s.exit);
    let hs_seed = run.serial.as_ref().and_then(|s| {
        s.lines
            .iter()
            .filter_map(|l| std::str::from_utf8(l).ok())
            .find_map(|l| l.strip_prefix("HS_SEED ").map(shown))
    });
    let k = &prepared.record.keys;
    json!({
        "name": run.name.as_str(),
        "group": group,
        "position": position,
        "verdict": verdict,
        "reason": reason,
        "scan": format!("{:?}", run.control.resume),
        "skipped_lines": run.control.skipped,
        "first_line": run.control.first_line,
        "hs_exit": exit.map(|e| json!({"code": e.code, "signal": e.signal})),
        "hs_seed": hs_seed,
        "events": event_lines(run),
        "run_log_bytes": run.run_log_len,
        "uart_log_bytes": run.uart_len,
        "argv_masked_sha256": run.argv_masked_sha256,
        "fwcfg_sha256": run.input_hashes,
        "restore_keys": {
            "stop_seq": k.stop_seq, "branch_count": k.branch_count, "vtime_ns": k.vtime_ns,
            "probe_id": k.probe_id, "probe_arg0": k.probe_arg0, "vcpu_id": k.vcpu_id,
        },
        "restored_pointer": format!("{:#x}", prepared.record.rip),
    })
}

/// Whether a restore is counted: it passed, its resume was seen and the guest reported its exit.
fn counted(observed: &Observed, label: &str) -> bool {
    observed.resume_seen && observed.exit_reported && label == "pass"
}

fn seed_for(plan: &Plan, group: char) -> u64 {
    if group == 'C' {
        plan.seed
    } else {
        plan.seed.wrapping_add(OTHER_SEED_STEP)
    }
}

fn start_allowed(plan: &Plan) -> bool {
    plan.deadline
        .is_none_or(|d| Instant::now() + PER_RUN_WORST <= d)
}

fn finish(
    prepare: Result<PrepareSummary, String>,
    control: Group,
    other_seed: Group,
    end: &str,
    names: Vec<String>,
) -> Summary {
    Summary {
        prepare,
        control,
        other_seed,
        end: end.to_string(),
        names,
    }
}

/// Runs the plan. The session should be one that allows a single prepare and keeps everything; the driver binary makes it
/// so.
pub fn run_session(session: &Session, plan: &Plan) -> Summary {
    let session = session.with_trial_cap(TRIAL_CAP.min(session.timings().trial_cap));
    let records = Records::new(&session);
    records.step("prepare", &plan.seed.to_string(), "start");
    let prepared = match session.prepare(Inputs {
        seed: plan.seed,
        job: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            records.step("prepare", "", &e.to_string());
            let end = if e.kind == InfraKind::SecondPrepare {
                "END:REPREPARE"
            } else {
                "END:PREPARE"
            };
            return finish(
                Err(e.to_string()),
                Group::default(),
                Group::default(),
                end,
                Vec::new(),
            );
        }
    };
    records.step("prepare", prepared.name.as_str(), "ok");
    let mut control = Group::default();
    let mut other_seed = Group::default();
    let mut names = Vec::new();
    let mut end = "DONE";
    for (position, group) in order(plan.control_restores, plan.other_seed_restores)
        .chars()
        .enumerate()
    {
        if !start_allowed(plan) {
            end = "END:WCAP";
            break;
        }
        let ordinal = u32::try_from(position).unwrap_or(u32::MAX);
        let inputs = prepared.inputs.with_seed(seed_for(plan, group));
        let target = if group == 'C' {
            &mut control
        } else {
            &mut other_seed
        };
        records.step("trial", &format!("{group}{}", position + 1), "start");
        let first_not_ok = match session.restore_trial(&prepared, ordinal, inputs) {
            Err(e) => {
                records.step("trial", &format!("{group}{}", position + 1), &e.to_string());
                target.failed(&e);
                true
            }
            Ok(trial) => {
                names.push(trial.name.as_str().to_string());
                let observed = observe(&trial);
                let judged = judge_trial(&trial, session.timings().trial_cap.as_secs(), false);
                let label = judged
                    .as_ref()
                    .map_or("infra", |j| verdict_label(&j.verdict));
                records.run(
                    trial.name.as_str(),
                    &run_record(&prepared, &trial, &group.to_string(), position + 1, &judged),
                );
                records.step("trial", trial.name.as_str(), label);
                let ok = counted(&observed, label);
                match &judged {
                    Err(e) => target.failed(e),
                    Ok(_) => target.record(&observed, label),
                }
                session.clean_trial(&trial, label != "pass");
                !ok
            }
        };
        if position == 0 && first_not_ok {
            end = "END:FIRST-NOT-OK";
            break;
        }
    }
    records.step("end", "", end);
    finish(
        Ok(PrepareSummary {
            stamp: prepared.stamp.as_str().to_string(),
            checkpoint_sha256: prepared.record.hashes.to_vec(),
            restored_pointer: format!("{:#x}", prepared.record.rip),
            seconds: prepared.elapsed.as_secs_f64(),
        }),
        control,
        other_seed,
        end,
        names,
    )
}

/// Where the session's private records are.
pub fn records_dir(session: &Session) -> PathBuf {
    session.layout.root().join(RECORDS_DIR)
}

/// The path of the steps log in `dir`.
pub fn steps_log(dir: &Path) -> PathBuf {
    dir.join("steps.tsv")
}

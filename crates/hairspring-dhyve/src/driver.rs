//! The session driver's logic: one prepare, then restores with the checkpointed run's own input files (the control
//! group) and restores with another seed file, each read and counted. It runs the backend's own prepare, restore and judge
//! functions, and adds nothing but counting. The summary says what was seen and compares nothing but equality of
//! serial output and of event lines among the control restores.
use crate::error::InfraError;
use crate::fwcfg::Inputs;
use crate::judge::{judge_trial, observe, Observed};
use crate::session::Session;
use hairspring_engine::verdict::Verdict;
use serde_json::{json, Value};
use std::time::Duration;

/// What to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub seed: u64,
    pub control_restores: u32,
    pub other_seed_restores: u32,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrepareSummary {
    pub stamp: String,
    pub checkpoint_sha256: Vec<String>,
    pub restored_pointer: String,
    pub seconds: f64,
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
        })
        .to_string()
    }

    /// Whether the prepare worked and no restore met an infrastructure error.
    pub fn clean(&self) -> bool {
        self.prepare.is_ok() && self.control.errors.is_empty() && self.other_seed.errors.is_empty()
    }
}

fn restore_group(
    session: &Session,
    prepared: &crate::prepare::Prepared,
    first_ordinal: u32,
    seeds: &[u64],
    cap: Duration,
) -> Group {
    let mut group = Group::default();
    for (i, seed) in seeds.iter().enumerate() {
        let ordinal = first_ordinal + u32::try_from(i).unwrap_or(0);
        let inputs: Inputs = prepared.inputs.with_seed(*seed);
        match session.restore_trial(prepared, ordinal, inputs) {
            Err(e) => group.failed(&e),
            Ok(trial) => {
                let observed = observe(&trial);
                let judged = judge_trial(&trial, cap.as_secs(), false);
                let label = judged
                    .as_ref()
                    .map_or("infra", |j| verdict_label(&j.verdict));
                match &judged {
                    Err(e) => group.failed(e),
                    Ok(_) => group.record(&observed, label),
                }
                session.clean_trial(&trial, label != "pass");
            }
        }
    }
    group
}

/// Runs the plan.
pub fn run_session(session: &Session, plan: &Plan) -> Summary {
    let cap = session.timings().trial_cap;
    let prepared = match session.prepare(Inputs {
        seed: plan.seed,
        job: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            return Summary {
                prepare: Err(e.to_string()),
                control: Group::default(),
                other_seed: Group::default(),
            }
        }
    };
    let control_seeds = vec![plan.seed; plan.control_restores as usize];
    let other_seeds: Vec<u64> = (1..=u64::from(plan.other_seed_restores))
        .map(|n| plan.seed.wrapping_add(n))
        .collect();
    let control = restore_group(session, &prepared, 0, &control_seeds, cap);
    let other_seed = restore_group(session, &prepared, plan.control_restores, &other_seeds, cap);
    Summary {
        prepare: Ok(PrepareSummary {
            stamp: prepared.stamp.as_str().to_string(),
            checkpoint_sha256: prepared.record.hashes.to_vec(),
            restored_pointer: format!("{:#x}", prepared.record.rip),
            seconds: prepared.elapsed.as_secs_f64(),
        }),
        control,
        other_seed,
    }
}

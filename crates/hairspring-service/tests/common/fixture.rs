//! A finished run with a violation, an UNMET property, an unrated property and degraded trials, for the read tests.
#![allow(dead_code)]
use super::*;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::TrialReport;
use hairspring_props::AssertKind::{Always, Sometimes};
use hairspring_store::Store;
use std::sync::Arc;

pub const HOSTILE: &str = "<script>alert(1)</script>";
/// A seed above 2^53, which a JSON number cannot hold exactly.
pub const BIG_SEED: u64 = (1 << 60) + 7;

pub struct Fixture {
    pub svc: Service,
    pub store: Arc<dyn Store>,
    pub run_id: String,
}

/// Seeds 1-6: seed 3 fails `b.broken` (whose message is hostile text); `s.gap` is never true; `r.ok` is true in
/// seeds 2 and 4; seed 5 is degraded.
pub fn trial_set(rec: &mut Recorder) -> Vec<TrialReport> {
    let base = |r_true: u64, b_false: u64| {
        props_multi(&[
            ("a.fine", Always, "fine", 4, 0),
            ("b.broken", Always, HOSTILE, 4 - b_false, b_false),
            ("s.gap", Sometimes, "gap", 0, 4),
            ("r.ok", Sometimes, "ok", r_true, 2),
        ])
    };
    let mut reports = vec![
        rec.pass(1, &base(0, 0)),
        rec.pass(2, &base(1, 0)),
        rec.trial(3, &base(0, 1), ReportedVerdict::Violation, false),
        rec.pass(4, &base(1, 0)),
        rec.pass(5, &base(0, 0).with_host_fault()),
        rec.pass(6, &base(0, 0)),
    ];
    reports[2].reason = format!("{HOSTILE} property failed");
    reports
}

/// A run of six trials that ended having used its whole budget, reported by `worker`.
pub fn finished(worker: &str) -> Fixture {
    let (svc, store, engine) = setup();
    let job = lease_with_budget(&svc, &engine, worker, 6);
    let reports = trial_set(&mut Recorder::new());
    svc.ingest_trials(&job.job_id, worker, &reports).unwrap();
    finish(&svc, &job, worker, passed(6, Some(6)));
    Fixture {
        svc,
        store,
        run_id: job.run_id,
    }
}

pub fn failed_outcome(trials: u64) -> RunOutcome {
    RunOutcome {
        trials_reported: Some(trials),
        ..RunOutcome::failed(
            trials,
            trials,
            5,
            hairspring_engine::report::FailureReport {
                target: "lock-lease".into(),
                backend: "sim".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                steps: vec![],
                bundle: hairspring_core::bundle::Bundle::new(1, "t", vec![]).encode(),
            },
        )
    }
}

/// Every string in `value` that contains `needle`, with the path of object keys it was found under.
pub fn paths_containing(value: &serde_json::Value, needle: &str) -> Vec<String> {
    fn walk(v: &serde_json::Value, path: &str, needle: &str, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) if s.contains(needle) => out.push(path.to_owned()),
            serde_json::Value::Array(a) => a
                .iter()
                .for_each(|x| walk(x, &format!("{path}[]"), needle, out)),
            serde_json::Value::Object(o) => o
                .iter()
                .for_each(|(k, x)| walk(x, &format!("{path}.{k}"), needle, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, "", needle, &mut out);
    out
}

//! A run through a fake Hermit reports one record per judged trial, and those records fold to the run's statistics.
mod common;

use common::events::rig;
use common::props::*;
use hairspring_engine::config::TargetKind;
use hairspring_engine::report::{RunOutcome, RunStatus};
use hairspring_engine::trial_log::ReportedVerdict;
use hairspring_props::{RunCatalog, RunProps, TrialDigest};

fn fold_log(o: &RunOutcome) -> RunProps {
    let mut catalog = RunCatalog::new();
    let mut run = RunProps::new();
    for r in &o.trial_log {
        for u in &r.catalog {
            catalog = catalog.apply(u, 16384).expect("valid update");
        }
        let d = TrialDigest::decode(&r.digest).expect("valid digest");
        run = run.add_digest(r.seed, &d, &catalog);
    }
    run
}

#[test]
fn a_passing_run_reports_every_trial_and_the_records_fold_to_its_statistics() {
    let script = emit(&report("wal.ckpt", "always", 4, 0, true));
    let o = run_script("log-pass", &script, 5);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    let seqs: Vec<_> = o.trial_log.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
    let seeds: Vec<_> = o.trial_log.iter().map(|r| r.seed).collect();
    assert_eq!(seeds, vec![1, 2, 3, 4, 5]);
    assert!(o
        .trial_log
        .iter()
        .all(|r| r.verdict == ReportedVerdict::Pass));
    let folded = fold_log(&o);
    assert_eq!(Some(folded.summary()), o.property_run);
    assert_eq!(folded.finish(), o.properties);
}

#[test]
fn a_failing_run_reports_the_trials_up_to_the_failure_with_the_failing_one_marked() {
    let script = by_seed(
        &[(3, report("wal.ckpt", "always", 1, 2, true))],
        &report("wal.ckpt", "always", 4, 0, true),
    );
    let o = run_script("log-fail", &script, 6);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    let verdicts: Vec<_> = o.trial_log.iter().map(|r| r.verdict).collect();
    assert_eq!(
        verdicts,
        vec![
            ReportedVerdict::Pass,
            ReportedVerdict::Pass,
            ReportedVerdict::Violation
        ]
    );
    let bad = &o.trial_log[2];
    assert_eq!(bad.seed, 3);
    assert!(bad.reason.contains("wal.ckpt"), "{}", bad.reason);
    assert!(
        bad.detail.is_some() && bad.output.is_some(),
        "a finding keeps its detail"
    );
    assert_eq!(Some(fold_log(&o).summary()), o.property_run);
}

#[test]
fn a_run_whose_trials_time_out_reports_them_as_inconclusive() {
    let mut r = rig("log-hang", "sleep 30", false, vec![]);
    configure(&mut r, 2, 1);
    if let TargetKind::Process {
        trial_timeout_secs, ..
    } = &mut r.spec.kind
    {
        *trial_timeout_secs = Some(1);
    }
    let o = run(r);
    assert_eq!(o.inconclusive, 2, "{o:?}");
    assert!(o
        .trial_log
        .iter()
        .all(|t| t.verdict == ReportedVerdict::Inconclusive));
}

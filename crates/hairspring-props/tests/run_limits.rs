//! Run-level limits and the handling of inconclusive (timed out) trials.
mod common;

use common::*;
use hairspring_props::limits::MAX_IDS_PER_RUN;
use hairspring_props::*;

/// A complete one-process report cataloguing the given ids as `always`, each evaluated once.
fn report_of_ids(ids: &[String]) -> String {
    let mut out = hello("p1");
    for id in ids {
        out.push_str(&catalog_min("p1", id, AssertKind::Always));
        out.push_str(&assert_line("p1", id, 1, 0, None, Some(("t", 1))));
    }
    out.push_str(&ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{},"evals":{},"dropped":0,"threads":0}}"#,
        ids.len(),
        ids.len()
    )));
    out
}

fn unique_ids(trial: usize, count: usize) -> Vec<String> {
    (0..count).map(|i| format!("s{trial}.id{i}")).collect()
}

#[test]
fn distinct_ids_across_trials_stay_under_the_run_cap_and_the_overflow_is_counted() {
    let per_trial = 3000;
    let trials = 7;
    let mut run = RunProps::new();
    for t in 0..trials {
        run = run.add(t as u64, &fold(&report_of_ids(&unique_ids(t, per_trial))));
    }
    let summary = run.summary();
    let stats = run.finish();
    assert_eq!(stats.len(), MAX_IDS_PER_RUN);
    assert_eq!(
        summary.ids_dropped,
        (per_trial * trials - MAX_IDS_PER_RUN) as u64
    );
    assert!(
        summary.degraded_trials > 0,
        "a run that dropped ids is degraded"
    );
}

#[test]
fn a_run_under_the_cap_drops_nothing() {
    let run = RunProps::new().add(1, &fold(&report_of_ids(&unique_ids(0, 50))));
    assert_eq!(run.summary().ids_dropped, 0);
    assert_eq!(run.summary().degraded_trials, 0);
}

#[test]
fn ids_already_tracked_keep_accumulating_after_the_cap_is_reached() {
    let ids = unique_ids(0, 3);
    let mut run = RunProps::with_id_cap(2).add(1, &fold(&report_of_ids(&ids[..2])));
    run = run.add(2, &fold(&report_of_ids(&ids)));
    let summary = run.summary();
    let stats = run.finish();
    assert_eq!(summary.ids_dropped, 1);
    assert_eq!(stats.len(), 2);
    assert!(stats.iter().all(|s| s.trials == 2));
}

#[test]
fn an_inconclusive_trial_decides_no_kind() {
    let timed_out = fold(&single(AssertKind::Always, 1, 0));
    let clean = fold(&single(AssertKind::Sometimes, 2, 0));
    let run = RunProps::new().add_inconclusive(1, &timed_out).add(2, &clean);
    let summary = run.summary();
    let stat = run.finish().remove(0);
    assert_eq!(stat.kind, AssertKind::Sometimes);
    assert_eq!(summary.kind_mismatches, 0);
    assert_eq!(stat.trials_claimable, 1);
    assert_eq!(stat.status, PropertyStatus::Pass);
}

#[test]
fn a_later_inconclusive_trial_of_another_kind_is_not_a_mismatch_against_a_kind_it_decided_nothing_about() {
    let a = fold(&single(AssertKind::Always, 1, 0));
    let b = fold(&single(AssertKind::Sometimes, 1, 0));
    let run = RunProps::new().add_inconclusive(1, &a).add_inconclusive(2, &b);
    assert_eq!(run.summary().kind_mismatches, 0);
}

fn conflicting() -> TrialProps {
    fold(
        &[
            hello("p1"),
            catalog("p1", "x.id", AssertKind::Always),
            hello("p2"),
            catalog("p2", "x.id", AssertKind::Sometimes),
        ]
        .concat(),
    )
}

#[test]
fn a_conflict_in_an_inconclusive_trial_is_not_counted() {
    let t = conflicting();
    assert!(!t.conflicts.is_empty());
    let run = RunProps::new().add_inconclusive(1, &t);
    assert_eq!(run.summary().conflict_trials, 0);
    let counted = RunProps::new().add(1, &t);
    assert_eq!(counted.summary().conflict_trials, 1);
}

#[test]
fn inconclusive_trials_are_not_counted_as_missing_an_end_report() {
    let crashed = fold(&[hello("p1"), catalog("p1", "x.id", AssertKind::Always)].concat());
    let run = RunProps::new()
        .add(1, &crashed)
        .add_inconclusive(2, &crashed)
        .add_inconclusive(3, &fold(&single(AssertKind::Always, 1, 0)));
    let summary = run.summary();
    assert_eq!(summary.report_missing_trials, 1);
    assert_eq!(summary.inconclusive_trials, 2);
}

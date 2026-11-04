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
    let run = RunProps::new()
        .add_inconclusive(1, &timed_out)
        .add(2, &clean);
    let summary = run.summary();
    let stat = run.finish().remove(0);
    assert_eq!(stat.kind, AssertKind::Sometimes);
    assert_eq!(summary.kind_mismatches, 0);
    assert_eq!(stat.trials_claimable, 1);
    assert_eq!(stat.status, PropertyStatus::Pass);
}

#[test]
fn a_later_inconclusive_trial_of_another_kind_is_not_a_mismatch_against_a_kind_it_decided_nothing_about(
) {
    let a = fold(&single(AssertKind::Always, 1, 0));
    let b = fold(&single(AssertKind::Sometimes, 1, 0));
    let run = RunProps::new()
        .add_inconclusive(1, &a)
        .add_inconclusive(2, &b);
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

#[test]
fn an_inconclusive_failure_is_kept_when_a_later_trial_agrees_on_the_kind() {
    let failing = fold(&single(AssertKind::Always, 1, 1));
    let clean = fold(&single(AssertKind::Always, 2, 0));
    let stat = RunProps::new()
        .add_inconclusive(1, &failing)
        .add(2, &clean)
        .finish()
        .remove(0);
    assert_eq!(stat.trials_inconclusive_with_failures, 1);
}

#[test]
fn an_inconclusive_failure_under_a_kind_that_lost_is_forgotten() {
    let failing = fold(&single(AssertKind::Always, 1, 1));
    let clean = fold(&single(AssertKind::Sometimes, 2, 0));
    let stat = RunProps::new()
        .add_inconclusive(1, &failing)
        .add(2, &clean)
        .finish()
        .remove(0);
    assert_eq!(stat.trials_inconclusive_with_failures, 0);
}

#[test]
fn a_timed_out_trial_of_another_kind_leaves_no_mismatch_residue_against_a_decided_kind() {
    let clean = fold(&single(AssertKind::Always, 1, 0));
    let other = fold(&single(AssertKind::Sometimes, 1, 1));
    let run = RunProps::new().add(1, &clean).add_inconclusive(2, &other);
    let summary = run.summary();
    assert_eq!(summary.kind_mismatches, 0);
    assert!(summary.kind_mismatch_details.is_empty());
    let stat = run.finish().remove(0);
    assert_eq!(stat.kind, AssertKind::Always);
    assert_eq!(stat.trials_inconclusive_with_failures, 0);
}

#[test]
fn a_timed_out_trial_of_another_kind_is_still_a_trial_while_the_kind_is_undecided() {
    let a = fold(&single(AssertKind::Always, 1, 0));
    let b = fold(&single(AssertKind::Sometimes, 1, 1));
    let stat = RunProps::new()
        .add_inconclusive(1, &a)
        .add_inconclusive(2, &b)
        .finish()
        .remove(0);
    assert_eq!(stat.trials, 2);
    assert_eq!(stat.trials_inconclusive_with_failures, 0);
}

#[test]
fn the_summary_reports_the_cap_the_accumulator_actually_uses() {
    assert_eq!(RunProps::with_id_cap(2).summary().id_cap, 2);
    assert_eq!(RunProps::new().summary().id_cap, MAX_IDS_PER_RUN);
}

/// A complete one-process report in which each given id failed once.
fn report_of_failing_ids(ids: &[&str]) -> String {
    let mut out = hello("p1");
    for id in ids {
        out.push_str(&catalog_min("p1", id, AssertKind::Always));
        out.push_str(&assert_line("p1", id, 1, 1, Some(("t", 2)), Some(("t", 1))));
    }
    out.push_str(&ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{},"evals":{},"dropped":0,"threads":0}}"#,
        ids.len(),
        ids.len() * 2
    )));
    out
}

#[test]
fn the_summary_counts_inconclusive_trials_that_showed_a_failure_once_per_trial() {
    let two_failing = fold(&report_of_failing_ids(&["a", "b"]));
    let clean = fold(&single(AssertKind::Always, 3, 0));
    let conclusive_failure = fold(&single(AssertKind::Always, 4, 1));
    let summary = RunProps::new()
        .add_inconclusive(1, &two_failing)
        .add_inconclusive(2, &clean)
        .add(3, &conclusive_failure)
        .summary();
    assert_eq!(summary.inconclusive_trials, 2);
    assert_eq!(summary.inconclusive_with_failure_trials, 1);
}

#[test]
fn a_summary_stored_before_the_failure_count_existed_reads_it_as_zero() {
    let json = r#"{"trials":3,"perturbing_trials":0,"degraded_trials":0,"inconclusive_trials":2,"report_missing_trials":0,"conflict_trials":0,"kind_mismatches":0,"kind_mismatch_details":[]}"#;
    let summary: RunSummary = serde_json::from_str(json).expect("an older summary still reads");
    assert_eq!(summary.inconclusive_trials, 2);
    assert_eq!(summary.inconclusive_with_failure_trials, 0);
}

#[test]
fn a_failure_of_a_kind_the_run_did_not_keep_is_not_counted_as_shown() {
    let kept = fold(&single(AssertKind::Sometimes, 3, 0));
    let timed_out = fold(&single(AssertKind::Always, 0, 2));
    let summary = RunProps::new()
        .add(1, &kept)
        .add_inconclusive(2, &timed_out)
        .summary();
    assert_eq!(summary.inconclusive_trials, 1);
    assert_eq!(summary.inconclusive_with_failure_trials, 0);
}

/// A complete one-process report of `always` ids, each evaluated once; the id is given with its count of falses.
fn report_of_ids_with_falses(ids: &[(&str, u64)]) -> String {
    let mut out = hello("p1");
    for (id, falses) in ids {
        out.push_str(&catalog_min("p1", id, AssertKind::Always));
        let first_false = (*falses > 0).then_some(("t", 2));
        out.push_str(&assert_line(
            "p1",
            id,
            1,
            *falses,
            first_false,
            Some(("t", 1)),
        ));
    }
    out.push_str(&ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{},"evals":{},"dropped":0,"threads":0}}"#,
        ids.len(),
        ids.len() * 2
    )));
    out
}

#[test]
fn a_failing_id_is_still_counted_when_a_passing_id_follows_it_in_the_same_trial() {
    let trial = fold(&report_of_ids_with_falses(&[("a", 1), ("b", 0)]));
    let summary = RunProps::new().add_inconclusive(1, &trial).summary();
    assert_eq!(summary.inconclusive_with_failure_trials, 1);
}

/// Pins what `RunCatalog` does with an id past the cap (it is never catalogued, so the digest has no entry for it); the
/// fold cannot see the dropped id, so this cannot fail from a change in the fold alone.
#[test]
fn a_failure_on_an_id_dropped_by_the_cap_is_not_counted() {
    let kept = fold(&report_of_ids_with_falses(&[("a", 0)]));
    let dropped = fold(&report_of_ids_with_falses(&[("b", 1)]));
    let summary = RunProps::with_id_cap(1)
        .add(1, &kept)
        .add_inconclusive(2, &dropped)
        .summary();
    assert_eq!(summary.ids_dropped, 1);
    assert_eq!(summary.inconclusive_with_failure_trials, 0);
}

/// A complete one-process report of ids with their own kind and counts: `(id, kind, trues, falses)`.
fn report_of_kinds(ids: &[(&str, AssertKind, u64, u64)]) -> String {
    let mut out = hello("p1");
    for (id, kind, trues, falses) in ids {
        out.push_str(&catalog_min("p1", id, *kind));
        let first_false = (*falses > 0).then_some(("t", 2));
        let first_true = (*trues > 0).then_some(("t", 1));
        out.push_str(&assert_line(
            "p1",
            id,
            *trues,
            *falses,
            first_false,
            first_true,
        ));
    }
    out.push_str(&ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{},"evals":{},"dropped":0,"threads":0}}"#,
        ids.len(),
        ids.len() * 2
    )));
    out
}

#[test]
fn a_failure_seen_first_under_a_kind_that_a_later_trial_replaces_is_not_counted() {
    let timed_out = fold(&single(AssertKind::Always, 0, 2));
    let decided = fold(&single(AssertKind::Sometimes, 3, 0));
    let run = RunProps::new()
        .add_inconclusive(1, &timed_out)
        .add(2, &decided);
    let summary = run.summary();
    let stat = run.finish().remove(0);
    assert_eq!(stat.kind, AssertKind::Sometimes);
    assert_eq!(stat.trials_inconclusive_with_failures, 0);
    assert_eq!(summary.inconclusive_trials, 1);
    assert_eq!(summary.inconclusive_with_failure_trials, 0);
}

#[test]
fn a_failure_seen_first_is_still_counted_when_a_later_trial_keeps_the_kind() {
    let timed_out = fold(&single(AssertKind::Always, 0, 2));
    let decided = fold(&single(AssertKind::Always, 3, 0));
    let run = RunProps::new()
        .add_inconclusive(1, &timed_out)
        .add(2, &decided);
    let summary = run.summary();
    let stat = run.finish().remove(0);
    assert_eq!(stat.trials_inconclusive_with_failures, 1);
    assert_eq!(summary.inconclusive_with_failure_trials, 1);
}

#[test]
fn a_trial_still_counts_while_one_of_its_ids_keeps_its_failure() {
    use AssertKind::{Always, Sometimes};
    let timed_out = fold(&report_of_kinds(&[
        ("a", Always, 0, 2),
        ("b", Always, 0, 2),
    ]));
    let decides = fold(&report_of_kinds(&[
        ("a", Sometimes, 1, 0),
        ("b", Always, 1, 0),
    ]));
    let summary = RunProps::new()
        .add_inconclusive(1, &timed_out)
        .add(2, &decides)
        .summary();
    assert_eq!(summary.inconclusive_with_failure_trials, 1);
}

#[test]
fn a_trial_stops_counting_when_every_id_that_showed_the_failure_changes_kind() {
    use AssertKind::{Always, Sometimes};
    let timed_out = fold(&report_of_kinds(&[
        ("a", Always, 0, 2),
        ("b", Always, 0, 2),
    ]));
    let decides = fold(&report_of_kinds(&[
        ("a", Sometimes, 1, 0),
        ("b", Sometimes, 1, 0),
    ]));
    let other_timed_out = fold(&report_of_kinds(&[("a", Sometimes, 0, 0)]));
    let summary = RunProps::new()
        .add_inconclusive(1, &timed_out)
        .add_inconclusive(3, &other_timed_out)
        .add(2, &decides)
        .summary();
    assert_eq!(summary.inconclusive_with_failure_trials, 0);
}

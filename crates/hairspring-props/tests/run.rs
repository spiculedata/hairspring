mod common;

use common::*;
use hairspring_props::stats::wilson;
use hairspring_props::stats::Z_95;
use hairspring_props::*;

fn run_of(trials: &[(u64, String)]) -> (RunSummary, Vec<PropertyStat>) {
    let mut run = RunProps::new();
    for (seed, out) in trials {
        run = run.add(*seed, &fold(out));
    }
    (run.summary(), run.finish())
}

fn status_for(kind: AssertKind, trials: &[(u64, u64)]) -> PropertyStatus {
    let t: Vec<_> = trials
        .iter()
        .enumerate()
        .map(|(i, (tr, fa))| (i as u64, single(kind, *tr, *fa)))
        .collect();
    run_of(&t).1[0].status
}

#[test]
fn run_level_truth_table() {
    use AssertKind::*;
    use PropertyStatus::*;
    // (kind, per-trial (trues, falses), expected)
    assert_eq!(status_for(Always, &[(5, 0), (3, 0)]), Pass);
    assert_eq!(status_for(Always, &[(5, 0), (0, 1)]), Violation);
    assert_eq!(status_for(Always, &[(0, 0), (0, 0)]), Unmet);
    assert_eq!(status_for(AlwaysOrUnreachable, &[(0, 0), (0, 0)]), Pass);
    assert_eq!(
        status_for(AlwaysOrUnreachable, &[(2, 0), (0, 3)]),
        Violation
    );
    assert_eq!(status_for(Sometimes, &[(0, 4), (0, 2)]), Unmet);
    assert_eq!(status_for(Sometimes, &[(0, 4), (1, 2)]), Pass);
    assert_eq!(status_for(Sometimes, &[(0, 0), (0, 0)]), Unmet);
    assert_eq!(status_for(Reachable, &[(0, 0), (0, 0)]), Unmet);
    assert_eq!(status_for(Reachable, &[(0, 0), (1, 0)]), Pass);
    assert_eq!(status_for(Unreachable, &[(0, 0), (0, 0)]), Pass);
    assert_eq!(status_for(Unreachable, &[(0, 0), (1, 0)]), Violation);
}

#[test]
fn a_violation_outranks_unmet() {
    // `always` that was false in one trial has been evaluated, so it can only be Violation, never Unmet.
    assert_eq!(
        status_for(AssertKind::Always, &[(0, 1)]),
        PropertyStatus::Violation
    );
}

#[test]
fn always_stat_counts_trials_and_reports_wilson_and_first_failure() {
    let mut trials = Vec::new();
    for seed in 0..88u64 {
        let falses = if [10, 40, 41, 50, 60, 70, 80].contains(&seed) {
            1
        } else {
            0
        };
        trials.push((seed, single(AssertKind::Always, 4, falses)));
    }
    let (summary, stats) = run_of(&trials);
    assert_eq!(summary.trials, 88);
    let s = &stats[0];
    assert_eq!(
        (s.trials, s.trials_hit, s.trials_true, s.trials_false),
        (88, 88, 88, 7)
    );
    assert_eq!(
        (s.evals, s.evals_true, s.evals_false),
        (88 * 4 + 7, 88 * 4, 7)
    );
    let rate = s.rate.unwrap();
    assert_eq!(rate.basis, RateBasis::TrialsFalse);
    assert_eq!((rate.k, rate.n), (7, 88));
    assert!((rate.lower - 0.039).abs() < 5e-4 && (rate.upper - 0.155).abs() < 5e-4);
    assert_eq!(rate.rule_of_three, None);
    let ff = s.first_failure.as_ref().unwrap();
    assert_eq!(ff.seed, 10);
    assert_eq!(ff.witness.as_ref().unwrap().ord, 5);
}

#[test]
fn zero_failures_report_the_rule_of_three() {
    let trials: Vec<_> = (0..3000u64)
        .map(|i| (i, single(AssertKind::Always, 1, 0)))
        .collect();
    let (_, stats) = run_of(&trials);
    let rate = stats[0].rate.unwrap();
    assert_eq!(rate.k, 0);
    assert!((rate.rule_of_three.unwrap() - 1e-3).abs() < 1e-12);
    assert_eq!(rate.lower, 0.0);
    assert_eq!(stats[0].status, PropertyStatus::Pass);
    assert!(stats[0].first_failure.is_none());
}

#[test]
fn sometimes_rate_is_of_trials_true_and_reachable_of_trials_hit() {
    let trials: Vec<_> = (0..10u64)
        .map(|i| (i, single(AssertKind::Sometimes, (i % 2) * 3, 1)))
        .collect();
    let rate = run_of(&trials).1[0].rate.unwrap();
    assert_eq!(rate.basis, RateBasis::TrialsTrue);
    assert_eq!((rate.k, rate.n), (5, 10));
    let w = wilson(5, 10, Z_95).unwrap();
    assert_eq!((rate.lower, rate.upper), w);

    let trials: Vec<_> = (0..4u64)
        .map(|i| (i, single(AssertKind::Reachable, i % 2, 0)))
        .collect();
    let rate = run_of(&trials).1[0].rate.unwrap();
    assert_eq!(rate.basis, RateBasis::TrialsHit);
    assert_eq!((rate.k, rate.n), (2, 4));
}

#[test]
fn unreachable_violation_records_the_first_seed() {
    let trials = vec![
        (7, single(AssertKind::Unreachable, 0, 0)),
        (8, single(AssertKind::Unreachable, 1, 0)),
        (9, single(AssertKind::Unreachable, 2, 0)),
    ];
    let s = &run_of(&trials).1[0];
    assert_eq!(s.status, PropertyStatus::Violation);
    assert_eq!(s.first_failure.as_ref().unwrap().seed, 8);
}

#[test]
fn trials_denominator_counts_only_trials_that_catalogued_the_id() {
    let only_catalog = [hello("p1"), catalog("p1", "x.id", AssertKind::Always)].concat();
    let (summary, stats) = run_of(&[
        (1, single(AssertKind::Always, 2, 0)),
        (2, only_catalog),
        (3, "no events at all\n".to_string()),
    ]);
    assert_eq!(summary.trials, 3);
    assert_eq!(summary.report_missing_trials, 1);
    assert_eq!(stats[0].trials, 2);
    assert_eq!(stats[0].trials_hit, 1);
}

#[test]
fn summary_counts_perturbing_degraded_and_conflicting_trials() {
    let eval = ev(r#"{"v":1,"kind":"eval","id":"e.id","type":"reachable","ok":true}"#);
    let bad = "HAIRSPRING_EVENT {oops\n".to_string();
    let conflict = [
        catalog("p1", "c", AssertKind::Always),
        catalog("p2", "c", AssertKind::Reachable),
    ]
    .concat();
    let (summary, stats) = run_of(&[
        (1, eval),
        (2, bad),
        (3, conflict),
        (4, single(AssertKind::Always, 1, 0)),
    ]);
    assert_eq!(summary.trials, 4);
    assert_eq!(summary.perturbing_trials, 1);
    assert_eq!(
        summary.degraded_trials, 2,
        "the malformed trial and the conflicting one"
    );
    assert_eq!(summary.conflict_trials, 1);
    let ids: Vec<_> = stats.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["c", "e.id", "x.id"]);
}

#[test]
fn a_kind_that_changes_between_trials_is_counted_and_the_first_kind_wins() {
    let (summary, stats) = run_of(&[
        (1, single(AssertKind::Always, 1, 0)),
        (2, single(AssertKind::Sometimes, 0, 5)),
    ]);
    assert_eq!(summary.kind_mismatches, 1);
    assert_eq!(stats[0].kind, AssertKind::Always);
    assert_eq!(stats[0].trials, 1);
}

#[test]
fn counters_saturate_instead_of_overflowing() {
    let big = |seed| {
        (
            seed,
            [
                hello("p1"),
                catalog("p1", "x.id", AssertKind::Always),
                ev(r#"{"v":1,"kind":"assert","proc":"p1","id":"x.id","hits":18446744073709551615,"trues":18446744073709551615,"falses":0}"#),
            ]
            .concat(),
        )
    };
    let (_, stats) = run_of(&[big(1), big(2)]);
    assert_eq!(stats[0].evals, u64::MAX);
    assert_eq!(stats[0].trials_hit, 2);
}

#[test]
fn an_empty_run_has_no_stats() {
    let (summary, stats) = run_of(&[]);
    let expected = RunSummary {
        id_cap: hairspring_props::limits::MAX_IDS_PER_RUN,
        ..RunSummary::default()
    };
    assert_eq!(summary, expected);
    assert!(stats.is_empty());
}

#[test]
fn add_leaves_the_trial_untouched() {
    let trial = fold(&single(AssertKind::Always, 1, 1));
    let before = trial.clone();
    let _ = RunProps::new().add(1, &trial);
    assert_eq!(trial, before);
}

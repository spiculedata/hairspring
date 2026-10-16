//! Must-hit claims are made over claimable trials only: rated (clean) and not perturbing.
mod common;

use common::*;
use hairspring_props::*;

fn stat_of(trials: &[(u64, TrialProps)]) -> PropertyStat {
    let mut run = RunProps::new();
    for (seed, t) in trials {
        run = run.add(*seed, t);
    }
    run.finish().remove(0)
}

#[test]
fn a_hit_seen_only_in_a_degraded_trial_does_not_satisfy_a_sometimes() {
    let degraded = fold(&single(AssertKind::Sometimes, 3, 0)).with_host_fault();
    assert!(degraded.is_degraded());
    let clean_miss = fold(&single(AssertKind::Sometimes, 0, 4));
    let stat = stat_of(&[(1, clean_miss), (2, degraded)]);
    assert_eq!(stat.status, PropertyStatus::Unmet);
    assert_eq!(stat.trials_claimable, 1);
    assert_eq!(stat.trials_degraded, 1);
}

#[test]
fn a_hit_seen_only_in_a_perturbing_trial_does_not_satisfy_a_reachable() {
    let mut perturbing = fold(&single(AssertKind::Reachable, 1, 0));
    perturbing.perturbing = true;
    let clean_miss = fold(&single(AssertKind::Reachable, 0, 0));
    let stat = stat_of(&[(1, clean_miss), (2, perturbing)]);
    assert_eq!(stat.status, PropertyStatus::Unmet);
    assert_eq!(stat.trials_perturbing, 1);
}

#[test]
fn with_no_claimable_trial_neither_pass_nor_unmet_is_claimed() {
    let degraded = fold(&single(AssertKind::Sometimes, 0, 0)).with_host_fault();
    assert_eq!(stat_of(&[(1, degraded)]).status, PropertyStatus::Unrated);
}

#[test]
fn a_false_in_a_degraded_trial_is_still_a_violation() {
    let degraded = fold(&single(AssertKind::Always, 1, 1)).with_host_fault();
    let stat = stat_of(&[(7, degraded)]);
    assert_eq!(stat.status, PropertyStatus::Violation);
    assert_eq!(stat.first_failure.map(|f| f.seed), Some(7));
}

#[test]
fn property_stats_round_trip_through_json() {
    let stat = stat_of(&[(1, fold(&single(AssertKind::Always, 2, 1)))]);
    let json = serde_json::to_string(&stat).unwrap();
    assert_eq!(serde_json::from_str::<PropertyStat>(&json).unwrap(), stat);
}

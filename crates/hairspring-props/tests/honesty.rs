//! How a rate and a run headline are worded: always with `k/n`, never a bare percentage, and never a rate for a run
//! whose trial count was decided by its outcome.
mod common;

use common::*;
use hairspring_props::honesty::{headline, rate_cell, unmet_text, RateCell};
use hairspring_props::*;

fn stat_of(trials: &[(u64, TrialProps)]) -> PropertyStat {
    let mut run = RunProps::new();
    for (seed, t) in trials {
        run = run.add(*seed, t);
    }
    run.finish().remove(0)
}

fn clean(trues: u64, falses: u64) -> TrialProps {
    fold(&single(AssertKind::Always, trues, falses))
}

#[test]
fn zero_failures_show_k_over_n_and_the_exact_bound_for_few_trials() {
    let stat = stat_of(&[(1, clean(3, 0)), (2, clean(3, 0)), (3, clean(3, 0))]);
    let cell = rate_cell(&stat, false);
    assert!(matches!(cell, RateCell::ZeroEvents { n: 3, .. }));
    let text = cell.text();
    assert!(text.starts_with("0/3"), "{text}");
    assert!(
        text.contains("exact one-sided 95% upper bound 63.16%"),
        "{text}"
    );
}

#[test]
fn some_failures_show_k_over_n_and_a_wilson_interval() {
    let stat = stat_of(&[(1, clean(3, 0)), (2, clean(1, 2)), (3, clean(3, 0))]);
    let cell = rate_cell(&stat, false);
    let RateCell::Rate {
        k, n, lower, upper, ..
    } = cell
    else {
        panic!("expected a rate, got {cell:?}");
    };
    assert_eq!((k, n), (1, 3));
    assert!(lower > 0.0 && lower < upper && upper < 1.0);
    let text = cell.text();
    assert!(text.starts_with("1/3"), "{text}");
    assert!(text.contains('['), "an interval is always shown: {text}");
}

#[test]
fn every_trial_failing_still_shows_k_over_n_with_an_interval() {
    let stat = stat_of(&[(1, clean(0, 1)), (2, clean(0, 2))]);
    let RateCell::Rate { k, n, upper, .. } = rate_cell(&stat, false) else {
        panic!("expected a rate");
    };
    assert_eq!((k, n), (2, 2));
    assert_eq!(upper, 1.0);
}

#[test]
fn nothing_rated_is_unrated_and_not_zero_percent() {
    let degraded = fold(&single(AssertKind::Always, 3, 0)).with_host_fault();
    let stat = stat_of(&[(1, degraded)]);
    let cell = rate_cell(&stat, false);
    assert_eq!(cell, RateCell::Unrated);
    assert_eq!(cell.text(), "unrated (0 rated trials)");
}

#[test]
fn a_run_that_stopped_at_its_first_failure_has_no_rate_for_any_property() {
    let failing = stat_of(&[(1, clean(3, 0)), (2, clean(1, 1))]);
    let passing = stat_of(&[(1, clean(3, 0)), (2, clean(3, 0))]);
    for stat in [&failing, &passing] {
        let cell = rate_cell(stat, true);
        assert!(matches!(cell, RateCell::NotARate { .. }), "{cell:?}");
        let text = cell.text();
        assert!(
            text.starts_with("not a rate (stopped at first failure)"),
            "{text}"
        );
        assert!(text.contains("in 2 rated trials before stopping"), "{text}");
        assert!(!text.contains('['), "no interval: {text}");
        assert!(!text.contains("upper bound"), "no 3/N: {text}");
    }
    assert!(rate_cell(&failing, true).text().contains("1 failure in"));
}

#[test]
fn a_rate_cell_never_renders_a_bare_percentage() {
    let stats = [
        stat_of(&[(1, clean(3, 0))]),
        stat_of(&[(1, clean(0, 1)), (2, clean(3, 0))]),
    ];
    for stat in &stats {
        for stopped in [false, true] {
            let text = rate_cell(stat, stopped).text();
            if text.contains('%') {
                assert!(text.contains('/'), "a percentage comes with k/n: {text}");
            }
        }
    }
}

#[test]
fn the_headline_never_says_no_violations_when_nothing_was_rated() {
    assert_eq!(
        headline(Some(0), 0, None),
        "Nothing was rated: 0 rated trials"
    );
    assert_eq!(
        headline(Some(4790), 0, None),
        "No trial failed a property in 4790 rated trials"
    );
}

#[test]
fn the_headline_for_a_failure_is_an_observation_in_a_seed() {
    assert_eq!(
        headline(Some(16), 1, Some(1016)),
        "1 property failed; first failing seed 1016"
    );
    assert_eq!(
        headline(Some(16), 2, Some(7)),
        "2 properties failed; first failing seed 7"
    );
}

#[test]
fn the_headline_does_not_invent_a_trial_count_it_does_not_have() {
    let text = headline(None, 0, None);
    assert!(!text.contains("0 rated"), "{text}");
    assert!(text.contains("rated-trial count is not recorded"), "{text}");
}

#[test]
fn unmet_is_stated_for_the_trials_that_were_run_and_is_not_a_finding() {
    let text = unmet_text(3);
    assert!(text.contains("in 3 claimable trials"), "{text}");
    assert!(text.contains("coverage gap, not a finding"), "{text}");
    assert!(unmet_text(1).contains("in 1 claimable trial"));
    assert!(!unmet_text(1).contains("trials"));
}

#[test]
fn nothing_the_headline_says_uses_the_words_the_explorer_never_generates() {
    for text in [
        headline(Some(0), 0, None),
        headline(Some(5), 0, None),
        headline(Some(5), 3, Some(1)),
        headline(None, 0, None),
        unmet_text(4),
    ] {
        let lower = text.to_lowercase();
        for banned in ["unassisted", "found", "discovered", "flaky"] {
            assert!(!lower.contains(banned), "{text} contains {banned}");
        }
    }
}

fn zero_failure_text(trials: u64) -> String {
    let all_clean: Vec<(u64, TrialProps)> = (1..=trials).map(|seed| (seed, clean(1, 0))).collect();
    rate_cell(&stat_of(&all_clean), false).text()
}

#[test]
fn a_single_clean_trial_is_not_given_a_bound_above_one() {
    let text = zero_failure_text(1);
    assert!(text.starts_with("0/1"), "{text}");
    assert!(!text.contains("at most"), "{text}");
    assert!(!text.contains("3/1"), "{text}");
    assert!(
        text.contains("95.00%"),
        "the exact bound for one trial: {text}"
    );
}

#[test]
fn a_small_zero_failure_cell_states_the_exact_bound() {
    let text = zero_failure_text(3);
    assert!(text.contains("63.16%"), "1 - 0.05^(1/3) is 63.16%: {text}");
    assert!(!text.contains("at most") && !text.contains("3/3"), "{text}");
}

#[test]
fn a_large_zero_failure_cell_states_the_exact_bound_and_gives_three_over_n_only_as_a_note() {
    let text = zero_failure_text(300);
    assert!(
        text.contains("exact one-sided 95% upper bound 1.00%"),
        "the bound is 0.9936%, rounded up: {text}"
    );
    assert!(text.contains("binomial, 0 of 300"), "{text}");
    assert!(text.contains("about 3/300"), "{text}");
    assert!(!text.contains("at most") && !text.contains("<="), "{text}");
}

#[test]
fn a_small_zero_failure_cell_has_no_three_over_n_note() {
    let text = zero_failure_text(3);
    assert!(!text.contains("3/3"), "{text}");
    assert!(
        text.contains("exact one-sided 95% upper bound 63.16% (binomial, 0 of 3)"),
        "{text}"
    );
}

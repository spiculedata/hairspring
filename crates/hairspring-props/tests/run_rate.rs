use hairspring_props::run_rate::{figures, rate_lines, sensitivity, HitsExceedRated, RunCounts};

fn counts(hits: u64, rated: u64, inconclusive: u64, crashed: u64) -> RunCounts {
    RunCounts::new(hits, rated, inconclusive, crashed).expect("hits never exceed rated")
}

fn lines(hits: u64, rated: u64, inconclusive: u64, crashed: u64, stopped: bool) -> Vec<String> {
    rate_lines(&counts(hits, rated, inconclusive, crashed), stopped)
}

const BANNED: [&str; 7] = [
    "found",
    "discovered",
    "unassisted",
    "flaky",
    "bug rate",
    "failure rate",
    "reproduced",
];

#[test]
fn hits_cannot_exceed_rated_runs() {
    assert_eq!(
        RunCounts::new(3, 2, 0, 0),
        Err(HitsExceedRated { hits: 3, rated: 2 })
    );
}

#[test]
fn some_hits_show_k_over_n_the_point_and_the_wilson_interval() {
    assert_eq!(
        lines(3, 200, 0, 0, false),
        ["hit runs: 3/200 rated (1.50%), Wilson 95% interval [0.51%, 4.32%]"]
    );
}

#[test]
fn zero_hits_show_the_interval_the_exact_bound_and_the_rule_of_three_from_thirty_runs() {
    assert_eq!(
        lines(0, 200, 0, 0, false),
        ["hit runs: 0/200 rated, Wilson 95% interval [0.00%, 1.88%]; exact one-sided 95% upper bound 1.49% (rule of three: 1.50%)"]
    );
}

#[test]
fn zero_hits_in_few_runs_omit_the_rule_of_three_which_overshoots_there() {
    let out = lines(0, 3, 0, 0, false);
    assert_eq!(
        out,
        ["hit runs: 0/3 rated, Wilson 95% interval [0.00%, 56.15%]; exact one-sided 95% upper bound 63.16%"]
    );
}

#[test]
fn all_runs_hit_is_stated_with_its_interval() {
    assert_eq!(
        lines(10, 10, 0, 0, false),
        ["hit runs: 10/10 rated (100.00%), Wilson 95% interval [72.25%, 100.00%]"]
    );
}

#[test]
fn nothing_rated_is_zero_over_zero_and_never_a_percentage() {
    let out = lines(0, 0, 0, 0, false);
    assert_eq!(
        out,
        ["hit runs: 0/0 rated; nothing was rated, so there is no rate"]
    );
    assert!(!out.concat().contains('%'));
}

#[test]
fn inconclusive_runs_are_listed_apart_and_a_sensitivity_line_follows() {
    assert_eq!(
        lines(0, 200, 3, 0, false),
        [
            "hit runs: 0/200 rated, Wilson 95% interval [0.00%, 1.88%]; exact one-sided 95% upper bound 1.49% (rule of three: 1.50%)",
            "not rated: 3 inconclusive; reported separately, never in the denominator and never counted as passes",
            "sensitivity: if every non-rated run had been a hit: 3/203 (1.48%), Wilson 95% interval [0.50%, 4.25%]",
        ]
    );
}

#[test]
fn crashed_and_inconclusive_runs_are_both_listed() {
    let out = lines(2, 100, 4, 1, false);
    assert_eq!(
        out[1],
        "not rated: 4 inconclusive, 1 crashed; reported separately, never in the denominator and never counted as passes"
    );
    assert!(out[2].contains("7/105"), "{out:?}");
}

#[test]
fn an_inconclusive_only_run_has_no_rate_and_a_sensitivity_that_says_anything_is_possible() {
    assert_eq!(
        lines(0, 0, 5, 0, false),
        [
            "hit runs: 0/0 rated; nothing was rated, so there is no rate",
            "not rated: 5 inconclusive; reported separately, never in the denominator and never counted as passes",
            "sensitivity: if every non-rated run had been a hit: 5/5 (100.00%), Wilson 95% interval [56.55%, 100.00%]",
        ]
    );
}

#[test]
fn a_run_that_stopped_at_its_first_hit_has_counts_and_no_rate() {
    let out = lines(1, 40, 2, 0, true);
    assert_eq!(
        out,
        [
            "hit runs: 1 in 40 rated runs before the run stopped at its first hit; not a rate",
            "not rated: 2 inconclusive; reported separately, never in the denominator and never counted as passes",
        ]
    );
    assert!(!out.concat().contains('%'));
}

#[test]
fn figures_are_none_without_rated_runs() {
    assert_eq!(figures(0, 0), None);
    assert!(figures(0, 1).is_some());
}

#[test]
fn sensitivity_is_none_when_nothing_was_excluded() {
    assert_eq!(sensitivity(&counts(1, 10, 0, 0)), None);
}

#[test]
fn sensitivity_adds_every_excluded_run_to_hits_and_to_the_denominator() {
    let s = sensitivity(&counts(2, 200, 1, 1)).expect("two runs were excluded");
    assert_eq!((s.k, s.n), (4, 202));
}

#[test]
fn counts_are_unchanged_by_reporting() {
    let before = counts(2, 50, 3, 1);
    let copy = before;
    let _ = rate_lines(&before, false);
    let _ = sensitivity(&before);
    assert_eq!(before, copy);
}

#[test]
fn every_small_combination_keeps_the_rules() {
    for rated in 0..=40u64 {
        for hits in 0..=rated {
            for (inc, crash) in [(0, 0), (1, 0), (0, 2), (3, 3)] {
                for stopped in [false, true] {
                    check(hits, rated, inc, crash, stopped);
                }
            }
        }
    }
}

fn check(hits: u64, rated: u64, inc: u64, crash: u64, stopped: bool) {
    let ctx = format!("{hits}/{rated} +{inc} +{crash} stopped={stopped}");
    let out = lines(hits, rated, inc, crash, stopped);
    let first = &out[0];
    assert!(first.starts_with("hit runs: "), "{ctx}: {first}");
    if rated == 0 {
        assert!(first.contains("0/0 rated"), "{ctx}: {first}");
        assert!(!first.contains('%'), "{ctx}: {first}");
    } else if stopped {
        assert!(first.contains("not a rate"), "{ctx}: {first}");
        assert!(!first.contains('%'), "{ctx}: {first}");
    } else {
        assert!(first.contains(&format!("{hits}/{rated} rated")), "{ctx}");
        assert!(first.contains("Wilson 95% interval ["), "{ctx}: {first}");
    }
    let excluded = inc + crash;
    assert_eq!(
        out.len(),
        1 + usize::from(excluded > 0) * (1 + usize::from(!stopped)),
        "{ctx}"
    );
    if excluded > 0 && !stopped {
        let want = format!("{}/{}", hits + excluded, rated + excluded);
        assert!(out[2].contains(&want), "{ctx}: {}", out[2]);
    }
    for line in &out {
        let lower = line.to_lowercase();
        for word in BANNED {
            assert!(!lower.contains(word), "{ctx}: {word} in {line}");
        }
        if line.contains('%') {
            assert!(
                line.contains('/') || line.contains(" in "),
                "{ctx}: bare percentage in {line}"
            );
        }
    }
}

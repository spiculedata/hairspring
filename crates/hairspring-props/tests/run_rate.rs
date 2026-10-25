use hairspring_props::run_rate::{figures, rate_lines, sensitivity, HitsExceedRated, RunCounts};

fn counts(hits: u64, rated: u64, inconclusive: u64, crashed: u64) -> RunCounts {
    RunCounts::new(hits, rated, inconclusive, crashed).expect("hits never exceed rated")
}

fn lines(hits: u64, rated: u64, inconclusive: u64, crashed: u64, stopped: bool) -> Vec<String> {
    rate_lines(&counts(hits, rated, inconclusive, crashed), stopped)
}

fn first(hits: u64, rated: u64) -> String {
    lines(hits, rated, 0, 0, false).remove(0)
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

const NOT_RATED_TAIL: &str =
    "reported separately, never in the denominator and never counted as passes";

#[test]
fn hits_cannot_exceed_rated_trials() {
    assert_eq!(
        RunCounts::new(3, 2, 0, 0),
        Err(HitsExceedRated { hits: 3, rated: 2 })
    );
}

// Reference fixtures: Wilson 95% (z = 1.959964), lower rounded down, upper rounded up, computed independently.
#[test]
fn some_hits_show_k_over_n_the_point_and_the_wilson_interval() {
    let cases = [
        (
            3,
            200,
            "hits: 3/200 rated trials (1.50%), Wilson 95% interval [0.51%, 4.32%]",
        ),
        (
            1,
            10,
            "hits: 1/10 rated trials (10.00%), Wilson 95% interval [1.78%, 40.42%]",
        ),
        (
            2,
            10,
            "hits: 2/10 rated trials (20.00%), Wilson 95% interval [5.66%, 50.99%]",
        ),
        (
            1,
            90,
            "hits: 1/90 rated trials (1.11%), Wilson 95% interval [0.19%, 6.03%]",
        ),
        (
            5,
            97,
            "hits: 5/97 rated trials (5.15%), Wilson 95% interval [2.22%, 11.51%]",
        ),
        (
            7,
            105,
            "hits: 7/105 rated trials (6.67%), Wilson 95% interval [3.26%, 13.13%]",
        ),
        (
            1,
            1,
            "hits: 1/1 rated trials (100.00%), Wilson 95% interval [20.65%, 100.00%]",
        ),
        (
            10,
            10,
            "hits: 10/10 rated trials (100.00%), Wilson 95% interval [72.24%, 100.00%]",
        ),
    ];
    for (k, n, want) in cases {
        assert_eq!(first(k, n), want, "{k}/{n}");
    }
}

#[test]
fn zero_hits_show_one_labelled_bound_and_no_wilson_interval() {
    let cases = [
        (200, "hits: 0/200 rated trials, exact one-sided 95% upper bound 1.49% (binomial, 0 of 200); about 3/200"),
        (30, "hits: 0/30 rated trials, exact one-sided 95% upper bound 9.51% (binomial, 0 of 30); about 3/30"),
        (29, "hits: 0/29 rated trials, exact one-sided 95% upper bound 9.82% (binomial, 0 of 29)"),
        (3, "hits: 0/3 rated trials, exact one-sided 95% upper bound 63.16% (binomial, 0 of 3)"),
        (1, "hits: 0/1 rated trials, exact one-sided 95% upper bound 95.00% (binomial, 0 of 1)"),
    ];
    for (n, want) in cases {
        let line = first(0, n);
        assert_eq!(line, want, "0/{n}");
        assert!(!line.contains("Wilson"), "{line}");
    }
}

#[test]
fn the_zero_hit_bound_is_rounded_up_never_down() {
    // 0.9936%: rounding to nearest would print 0.99%, below the bound.
    assert!(
        first(0, 300).contains("upper bound 1.00% "),
        "{}",
        first(0, 300)
    );
}

#[test]
fn a_tiny_zero_hit_bound_is_stated_as_less_than_a_hundredth_of_a_percent() {
    let line = first(0, 600_000);
    assert!(line.contains("upper bound <0.01% "), "{line}");
}

#[test]
fn nothing_rated_is_zero_over_zero_and_never_a_percentage() {
    let out = lines(0, 0, 0, 0, false);
    assert_eq!(
        out,
        ["hits: 0/0 rated trials; nothing was rated, so there is no rate"]
    );
    assert!(!out.concat().contains('%'));
}

#[test]
fn inconclusive_trials_are_listed_apart_and_a_sensitivity_line_follows() {
    assert_eq!(
        lines(0, 200, 3, 0, false),
        [
            "hits: 0/200 rated trials, exact one-sided 95% upper bound 1.49% (binomial, 0 of 200); about 3/200".to_owned(),
            format!("not rated trials: 3 inconclusive; {NOT_RATED_TAIL}"),
            "sensitivity: if every non-rated trial had been a hit: 3/203 (1.48%), Wilson 95% interval [0.50%, 4.26%]".to_owned(),
        ]
    );
}

#[test]
fn crashed_and_inconclusive_trials_are_both_listed() {
    let out = lines(2, 100, 4, 1, false);
    assert_eq!(
        out[1],
        format!("not rated trials: 4 inconclusive, 1 failed in the host; {NOT_RATED_TAIL}")
    );
    assert!(out[2].contains("7/105 (6.67%)"), "{out:?}");
}

#[test]
fn an_inconclusive_only_run_has_no_rate_and_no_sensitivity_line() {
    assert_eq!(
        lines(0, 0, 5, 0, false),
        [
            "hits: 0/0 rated trials; nothing was rated, so there is no rate".to_owned(),
            format!("not rated trials: 5 inconclusive; {NOT_RATED_TAIL}"),
        ]
    );
}

#[test]
fn a_run_that_stopped_at_its_first_hit_has_counts_and_no_rate() {
    let out = lines(1, 40, 2, 0, true);
    assert_eq!(
        out,
        [
            "hits: 1 in 40 rated trials before the run stopped at its first hit; not a rate"
                .to_owned(),
            format!("not rated trials: 2 inconclusive; {NOT_RATED_TAIL}"),
        ]
    );
    assert!(!out.concat().contains('%'));
}

#[test]
fn figures_are_none_without_rated_trials() {
    assert_eq!(figures(0, 0), None);
    assert!(figures(0, 1).is_some());
}

#[test]
fn sensitivity_is_none_when_nothing_was_excluded_or_nothing_was_rated() {
    assert_eq!(sensitivity(&counts(1, 10, 0, 0)), None);
    assert_eq!(sensitivity(&counts(0, 0, 5, 0)), None);
}

#[test]
fn sensitivity_adds_every_excluded_trial_to_hits_and_to_the_denominator() {
    let s = sensitivity(&counts(2, 200, 1, 1)).expect("two trials were excluded");
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
    let head = &out[0];
    assert!(head.starts_with("hits: "), "{ctx}: {head}");
    if rated == 0 {
        assert!(head.contains("0/0 rated trials"), "{ctx}: {head}");
        assert!(!head.contains('%'), "{ctx}: {head}");
    } else if stopped {
        assert!(head.contains("not a rate"), "{ctx}: {head}");
        assert!(!head.contains('%'), "{ctx}: {head}");
    } else {
        assert!(
            head.starts_with(&format!("hits: {hits}/{rated} rated trials")),
            "{ctx}: {head}"
        );
        // Zero hits get the exact one-sided bound and no two-sided interval; any hit gets the interval.
        assert_eq!(
            head.contains("Wilson 95% interval ["),
            hits > 0,
            "{ctx}: {head}"
        );
        assert_eq!(
            head.contains("exact one-sided 95% upper bound"),
            hits == 0,
            "{ctx}"
        );
    }
    let excluded = inc + crash;
    let sensitivity_shown = excluded > 0 && !stopped && rated > 0;
    let expected_len = 1 + usize::from(excluded > 0) + usize::from(sensitivity_shown);
    assert_eq!(out.len(), expected_len, "{ctx}: {out:?}");
    if sensitivity_shown {
        let want = format!(": {}/{} (", hits + excluded, rated + excluded);
        assert!(out[2].contains(&want), "{ctx}: {}", out[2]);
    }
    for line in &out {
        let lower = line.to_lowercase();
        for word in BANNED {
            assert!(!lower.contains(word), "{ctx}: {word} in {line}");
        }
        // Every figure is a count over a denominator, or sits in a line that is explicitly not a rate.
        if line.contains('%') {
            let counted = line.split_whitespace().any(|w| {
                w.split_once('/').is_some_and(|(a, b)| {
                    a.parse::<u64>().is_ok()
                        && b.trim_end_matches([',', ';']).parse::<u64>().is_ok()
                })
            });
            assert!(counted, "{ctx}: percentage without k/n in {line}");
        }
    }
}

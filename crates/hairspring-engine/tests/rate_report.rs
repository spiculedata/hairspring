use hairspring_engine::format::to_text;
use hairspring_engine::report::{FailureReport, RunOutcome};

fn failure() -> FailureReport {
    FailureReport {
        target: "svc".to_owned(),
        backend: "process".to_owned(),
        message: "a property failed".to_owned(),
        original_len: 1,
        shrunk_len: 1,
        steps: Vec::new(),
        bundle: "bundle".to_owned(),
    }
}

/// A failed outcome as an older stored run reads back: `stopped_at_first_failure` defaults to false.
fn legacy_failed(trials: u64, inconclusive: u64) -> RunOutcome {
    RunOutcome {
        stopped_at_first_failure: false,
        ..RunOutcome::failed(trials, trials, 5, failure()).with_inconclusive(inconclusive)
    }
}

/// What the Hermit backend returns when every trial was inconclusive (`tally.rs`): an error that ran trials.
fn nothing_tested(trials: u64) -> RunOutcome {
    RunOutcome::error("all trials timed out; nothing was tested")
        .with_trials_attempted(trials)
        .with_inconclusive(trials)
}

#[test]
fn a_passed_run_states_its_rate_with_n_k_and_a_labelled_bound() {
    let text = to_text("svc", &RunOutcome::passed(200, 200, 5));
    assert!(
        text.contains("  hits: 0/200 rated trials, exact one-sided 95% upper bound 1.49% (binomial, 0 of 200); about 3/200\n"),
        "{text}"
    );
}

#[test]
fn zero_hits_show_no_two_sided_interval_beside_the_one_sided_bound() {
    let text = to_text("svc", &RunOutcome::passed(200, 200, 5));
    assert!(!text.contains("Wilson"), "{text}");
}

#[test]
fn timeouts_are_listed_apart_and_left_out_of_the_denominator() {
    let text = to_text("svc", &RunOutcome::passed(200, 200, 5).with_inconclusive(8));
    assert!(text.contains("  hits: 0/192 rated trials,"), "{text}");
    assert!(
        text.contains("  not rated trials: 8 inconclusive; reported separately, never in the denominator and never counted as passes\n"),
        "{text}"
    );
    assert!(
        text.contains("  sensitivity: if every non-rated trial had been a hit: 8/200 (4.00%), Wilson 95% interval [2.04%, 7.70%]\n"),
        "{text}"
    );
}

#[test]
fn a_run_that_tested_nothing_is_an_error_and_still_shows_zero_over_zero() {
    let text = to_text("svc", &nothing_tested(4));
    assert!(text.starts_with("ERROR   svc"), "{text}");
    assert!(
        text.contains("  hits: 0/0 rated trials; nothing was rated, so there is no rate\n"),
        "{text}"
    );
    assert!(
        text.contains("  not rated trials: 4 inconclusive;"),
        "{text}"
    );
    assert!(!text.contains("sensitivity"), "{text}");
    assert!(!text.contains('%'), "{text}");
}

#[test]
fn an_error_that_ran_no_trials_has_no_rate_section() {
    let text = to_text("svc", &RunOutcome::error("boom"));
    assert!(!text.contains("hits:"), "{text}");
    assert!(!text.contains("not rated"), "{text}");
}

#[test]
fn a_failed_run_has_no_rate_because_it_stopped_at_its_first_hit() {
    let text = to_text(
        "svc",
        &RunOutcome::failed(7, 7, 5, failure()).with_inconclusive(2),
    );
    assert!(
        text.contains(
            "  hits: 1 in 5 rated trials before the run stopped at its first hit; not a rate\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("  not rated trials: 2 inconclusive;"),
        "{text}"
    );
    assert!(!text.contains("sensitivity"), "{text}");
    assert!(!text.contains("Wilson"), "{text}");
}

#[test]
fn a_failed_run_read_back_without_the_stopped_flag_still_has_no_rate() {
    let text = to_text("svc", &legacy_failed(7, 2));
    assert!(
        text.contains("before the run stopped at its first hit; not a rate"),
        "{text}"
    );
    assert!(!text.contains('%'), "{text}");
    assert!(
        !text.contains("Wilson") && !text.contains("sensitivity"),
        "{text}"
    );
}

#[test]
fn a_failed_run_with_no_recorded_trials_prints_no_rate_and_no_zero_over_zero() {
    let text = to_text("svc", &legacy_failed(0, 0));
    assert!(!text.contains("hits:"), "{text}");
    assert!(!text.contains("0/0"), "{text}");
}

#[test]
fn a_failed_run_with_nothing_excluded_adds_no_rate_section() {
    let text = to_text("svc", &RunOutcome::failed(7, 7, 5, failure()));
    assert!(!text.contains("hits:"), "{text}");
}

#[test]
fn a_stored_inconclusive_count_above_the_trials_is_capped_and_says_so() {
    let text = to_text("svc", &RunOutcome::passed(2, 2, 5).with_inconclusive(9));
    assert!(text.contains("  hits: 0/0 rated trials;"), "{text}");
    assert!(
        text.contains("  note: the stored inconclusive count (9) is above the trial count (2) and was capped; this record may be corrupt\n"),
        "{text}"
    );
    assert!(
        text.contains("  not rated trials: 2 inconclusive;"),
        "{text}"
    );
}

#[test]
fn a_consistent_record_has_no_corruption_note() {
    let text = to_text("svc", &RunOutcome::passed(10, 10, 5).with_inconclusive(10));
    assert!(!text.contains("note:"), "{text}");
}

#[test]
fn the_report_never_uses_the_words_the_wording_rules_forbid() {
    for outcome in [
        RunOutcome::passed(50, 50, 1),
        RunOutcome::passed(50, 50, 1).with_inconclusive(5),
        RunOutcome::passed(5, 5, 1).with_inconclusive(5),
        RunOutcome::failed(7, 7, 1, failure()).with_inconclusive(1),
        nothing_tested(3),
        legacy_failed(4, 1),
    ] {
        let text = to_text("svc", &outcome).to_lowercase();
        let rate_part: String = text
            .lines()
            .filter(|l| {
                l.starts_with("  hits")
                    || l.starts_with("  not rated")
                    || l.starts_with("  sensitivity")
                    || l.starts_with("  note")
            })
            .collect();
        assert!(!rate_part.is_empty() || outcome.trials == 0);
        for word in ["found", "discovered", "unassisted", "bug rate", "flaky"] {
            assert!(!rate_part.contains(word), "{word}: {rate_part}");
        }
    }
}

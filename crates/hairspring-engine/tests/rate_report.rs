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

#[test]
fn a_passed_run_states_its_rate_with_n_k_and_an_interval() {
    let text = to_text("svc", &RunOutcome::passed(200, 200, 5));
    assert!(
        text.contains("  hit runs: 0/200 rated, Wilson 95% interval [0.00%, 1.88%]; exact one-sided 95% upper bound 1.49% (rule of three: 1.50%)\n"),
        "{text}"
    );
}

#[test]
fn timeouts_are_listed_apart_and_left_out_of_the_denominator() {
    let text = to_text("svc", &RunOutcome::passed(200, 200, 5).with_inconclusive(8));
    assert!(text.contains("  hit runs: 0/192 rated,"), "{text}");
    assert!(
        text.contains("  not rated: 8 inconclusive; reported separately, never in the denominator and never counted as passes\n"),
        "{text}"
    );
    assert!(
        text.contains("  sensitivity: if every non-rated run had been a hit: 8/200 (4.00%), Wilson 95% interval ["),
        "{text}"
    );
}

#[test]
fn a_run_where_every_trial_was_inconclusive_is_zero_over_zero_not_zero_percent() {
    let text = to_text("svc", &RunOutcome::passed(4, 4, 5).with_inconclusive(4));
    assert!(
        text.contains("  hit runs: 0/0 rated; nothing was rated, so there is no rate\n"),
        "{text}"
    );
    assert!(!text.contains("hit runs: 0/0 rated (0"), "{text}");
}

#[test]
fn a_failed_run_has_no_rate_because_it_stopped_at_its_first_hit() {
    let text = to_text(
        "svc",
        &RunOutcome::failed(7, 7, 5, failure()).with_inconclusive(2),
    );
    assert!(
        text.contains(
            "  hit runs: 1 in 5 rated runs before the run stopped at its first hit; not a rate\n"
        ),
        "{text}"
    );
    assert!(text.contains("  not rated: 2 inconclusive;"), "{text}");
    assert!(!text.contains("sensitivity"), "{text}");
    assert!(!text.contains("Wilson"), "{text}");
}

#[test]
fn a_failed_run_with_nothing_excluded_adds_no_rate_section() {
    let text = to_text("svc", &RunOutcome::failed(7, 7, 5, failure()));
    assert!(!text.contains("hit runs"), "{text}");
}

#[test]
fn an_errored_run_has_no_rate_section() {
    let text = to_text("svc", &RunOutcome::error("boom").with_inconclusive(3));
    assert!(!text.contains("hit runs"), "{text}");
    assert!(!text.contains("sensitivity"), "{text}");
}

#[test]
fn a_stored_outcome_with_more_inconclusive_than_trials_is_not_rated() {
    let text = to_text("svc", &RunOutcome::passed(2, 2, 5).with_inconclusive(9));
    assert!(text.contains("  hit runs: 0/0 rated;"), "{text}");
}

#[test]
fn the_report_never_uses_the_words_the_wording_rules_forbid() {
    for outcome in [
        RunOutcome::passed(50, 50, 1),
        RunOutcome::passed(50, 50, 1).with_inconclusive(5),
        RunOutcome::passed(5, 5, 1).with_inconclusive(5),
        RunOutcome::failed(7, 7, 1, failure()).with_inconclusive(1),
    ] {
        let text = to_text("svc", &outcome).to_lowercase();
        let rate_part: String = text
            .lines()
            .filter(|l| {
                l.starts_with("  hit runs")
                    || l.starts_with("  not rated")
                    || l.starts_with("  sensitivity")
            })
            .collect();
        for word in ["found", "discovered", "unassisted", "bug rate", "flaky"] {
            assert!(!rate_part.contains(word), "{word}: {rate_part}");
        }
    }
}

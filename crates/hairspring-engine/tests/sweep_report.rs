//! The text report of a sweep: a rate with its interval, the trials left out of it, and the failing seeds.
use hairspring_engine::format::to_text;
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::sweep::{assemble, SweepEnd, MAX_FIRST_FAILING_SEEDS};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use ReportedVerdict::{Inconclusive, Infra, Pass, Unusable, Violation};

fn failure() -> FailureReport {
    FailureReport {
        target: "svc".into(),
        backend: "hermit".into(),
        message: "property violated at sched-seed 3".into(),
        original_len: 1,
        shrunk_len: 1,
        steps: Vec::new(),
        bundle: "bundle".into(),
    }
}

fn sweep(verdicts: &[ReportedVerdict]) -> RunOutcome {
    let rows: Vec<TrialReport> = verdicts
        .iter()
        .enumerate()
        .map(|(i, v)| TrialReport::without_events(i as u64, 1 + i as u64, *v, "", 1))
        .collect();
    let failure = verdicts.contains(&Violation).then(failure);
    assemble(SweepEnd {
        steps_executed: rows.len() as u64,
        rows,
        failure,
        elapsed_ms: 9,
    })
}

#[test]
fn a_sweep_with_violations_states_its_rate_with_k_n_and_an_interval() {
    let text = to_text(
        "svc",
        &sweep(&[
            Pass, Pass, Violation, Pass, Violation, Pass, Pass, Pass, Pass,
        ]),
    );
    assert!(
        text.contains("  hits: 2/9 rated trials (22.22%), Wilson 95% interval ["),
        "{text}"
    );
    assert!(!text.contains("first hit"), "{text}");
    assert!(!text.contains("not a rate"), "{text}");
}

#[test]
fn the_report_lists_the_trials_by_verdict_and_the_failing_seeds() {
    let text = to_text(
        "svc",
        &sweep(&[Pass, Violation, Inconclusive, Pass, Violation]),
    );
    assert!(
        text.contains(
            "  sweep: 5 trials: 2 pass, 2 violation, 1 inconclusive, 0 unusable, 0 infra\n"
        ),
        "{text}"
    );
    assert!(text.contains("  failing seeds: 2, 5\n"), "{text}");
}

#[test]
fn timeouts_unusable_and_infra_trials_are_out_of_the_denominator_and_listed_apart() {
    let text = to_text(
        "svc",
        &sweep(&[Pass, Violation, Inconclusive, Unusable, Infra, Pass]),
    );
    assert!(text.contains("  hits: 1/3 rated trials ("), "{text}");
    assert!(
        text.contains("  not rated trials: 2 inconclusive, 1 crashed;"),
        "{text}"
    );
    assert!(text.contains("never counted as passes"), "{text}");
}

#[test]
fn a_sweep_without_a_violation_states_its_upper_bound_and_no_interval() {
    let text = to_text("svc", &sweep(&[Pass, Pass, Pass, Pass]));
    assert!(text.starts_with("PASSED  svc"), "{text}");
    assert!(
        text.contains("  hits: 0/4 rated trials, exact one-sided 95% upper bound"),
        "{text}"
    );
    assert!(!text.contains("Wilson"), "{text}");
    assert!(!text.contains("failing seeds"), "{text}");
}

#[test]
fn a_sweep_that_rated_nothing_shows_zero_over_zero_and_no_percentage() {
    let text = to_text("svc", &sweep(&[Inconclusive, Unusable]));
    assert!(text.starts_with("ERROR   svc"), "{text}");
    assert!(
        text.contains("  hits: 0/0 rated trials; nothing was rated, so there is no rate\n"),
        "{text}"
    );
    assert!(!text.contains('%'), "{text}");
}

#[test]
fn a_long_list_of_failing_seeds_is_cut_and_says_how_many_there_were() {
    let verdicts = vec![Violation; MAX_FIRST_FAILING_SEEDS + 4];
    let text = to_text("svc", &sweep(&verdicts));
    assert!(
        text.contains(&format!(
            "(first {MAX_FIRST_FAILING_SEEDS} of {} violations)",
            MAX_FIRST_FAILING_SEEDS + 4
        )),
        "{text}"
    );
}

#[test]
fn a_search_run_is_reported_exactly_as_before() {
    let text = to_text("svc", &RunOutcome::failed(7, 7, 5, failure()));
    assert!(!text.contains("sweep:"), "{text}");
    assert!(!text.contains("failing seeds"), "{text}");
}

#[test]
fn the_report_never_uses_the_forbidden_wording() {
    for verdicts in [
        vec![Pass; 10],
        vec![Pass, Violation, Inconclusive],
        vec![Violation; 12],
        vec![Inconclusive, Infra],
    ] {
        let text = to_text("svc", &sweep(&verdicts)).to_lowercase();
        for word in ["found", "discovered", "unassisted", "bug rate", "flaky"] {
            assert!(!text.contains(word), "{word}: {text}");
        }
    }
}

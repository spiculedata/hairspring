//! Verdicts that depend on particular lines (violation markers, Hermit's own failures) are decided from the true
//! lines of the stream, not from the head and tail the capture keeps.
mod common;

use common::events::rig;
use common::props::{configure, run};
use hairspring_engine::report::{RunOutcome, RunStatus};

/// `n` bytes of `c`, with no newline.
fn filler(n: usize, c: &str) -> String {
    format!("head -c {n} /dev/zero | tr '\\0' '{c}'; ")
}

/// Half of the default 1 MiB capture cap: how much of each end is kept.
const HALF: usize = 512 * 1024;

fn outcome(name: &str, script: &str) -> RunOutcome {
    let mut r = rig(name, script, false, vec![]);
    configure(&mut r, 1, 1);
    run(r)
}

fn violation_message(o: &RunOutcome) -> String {
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    o.failure.as_ref().unwrap().message.clone()
}

#[test]
fn a_marker_in_the_omitted_middle_of_long_output_is_still_a_violation() {
    // Between 512 KiB and 1 MiB in: kept before the head/tail change, omitted after it.
    let script = format!(
        "{}echo; echo 'HAIRSPRING_VIOLATION: lost in the middle'; {}echo; exit 0",
        filler(700_000, "x"),
        filler(1_500_000, "y")
    );
    let o = outcome("marker-middle", &script);
    assert!(violation_message(&o).contains("lost in the middle"));
}

#[test]
fn a_marker_cut_by_the_end_of_the_head_is_still_a_violation() {
    // The marker starts ten bytes before the head's last byte, so the cut falls inside it.
    let script = format!(
        "{}echo; echo 'HAIRSPRING_VIOLATION: straddles the cut'; {}echo",
        filler(HALF - 11, "x"),
        filler(1_500_000, "y")
    );
    let o = outcome("marker-straddle", &script);
    assert!(violation_message(&o).contains("straddles the cut"));
}

#[test]
fn a_marker_on_stderr_in_the_omitted_middle_is_still_a_violation() {
    let script = format!(
        "{{ {}echo; echo 'ANTHILL_VIOLATION: on stderr'; {}echo; }} >&2",
        filler(700_000, "x"),
        filler(1_500_000, "y")
    );
    let o = outcome("marker-stderr", &script);
    assert!(violation_message(&o).contains("on stderr"));
}

#[test]
fn a_fragment_at_the_start_of_the_tail_window_is_not_a_marker() {
    // One long line that merely contains the marker text; the kept tail begins exactly at the `H`.
    let marker = "HAIRSPRING_VIOLATION: not a line start";
    let script = format!(
        "{}printf '%s' '{marker}'; {}",
        filler(1_500_000, "a"),
        filler(HALF - marker.len(), "b")
    );
    let o = outcome("marker-fragment", &script);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
}

#[test]
fn markers_in_short_output_are_still_found() {
    let o = outcome(
        "marker-short",
        "echo noise; echo '  HAIRSPRING_VIOLATION: short'; exit 0",
    );
    assert!(violation_message(&o).contains("short"));
}

#[test]
fn a_marker_that_does_not_start_its_line_is_not_a_violation_in_short_output() {
    let o = outcome("marker-inline", "echo 'see HAIRSPRING_VIOLATION: no'");
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
}

#[test]
fn hermits_own_failure_in_the_omitted_middle_of_stderr_is_still_infrastructure() {
    let script = format!(
        "{{ {}echo; echo 'HERMIT_INTERNAL_FAILURE'; echo 'Error: it broke'; {}echo; }} >&2",
        filler(700_000, "x"),
        filler(1_500_000, "y")
    );
    let o = outcome("infra-middle", &script);
    assert_eq!(o.status, RunStatus::Error, "{o:?}");
    assert!(
        o.error.as_deref().unwrap_or_default().contains("it broke"),
        "{o:?}"
    );
}

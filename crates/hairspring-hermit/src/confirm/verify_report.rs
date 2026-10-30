//! What the strict verify's detail says when Hermit did not deliver a comparison: a timeout, or a failure of Hermit
//! itself. The result file's shapes are Hermit's: a `no_result` verdict carries `no_result_reason`, an
//! `infrastructure_error` verdict carries `infrastructure_error` (and a null `no_result_reason`), each an object tagged
//! by `kind`.
use serde_json::{Map, Value};

/// The most characters of Hermit's own words the detail carries, so a long line cannot bury the rest.
const MAX_QUOTED_CHARS: usize = 300;
/// The verdicts Hermit gives a verify that did not end in a match or a divergence.
const NO_COMPARISON_VERDICTS: [&str; 2] = ["no_result", "infrastructure_error"];

fn clip(text: &str) -> String {
    text.chars().take(MAX_QUOTED_CHARS).collect()
}

fn verdict_of(result: &Value) -> &str {
    result
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("no verdict")
}

/// True when Hermit's result file says the verify did not end in a match or a divergence.
pub(super) fn is_no_comparison(result: &Value) -> bool {
    NO_COMPARISON_VERDICTS.contains(&verdict_of(result))
}

/// A value as short text: strings as written, an object tagged by `kind` as `kind(field=value, ..)` with its fields in
/// Hermit's (alphabetical) order and its null fields left out, anything else as JSON.
fn describe(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(fields) => describe_object(fields),
        other => other.to_string(),
    }
}

fn describe_object(fields: &Map<String, Value>) -> String {
    let kind = fields.get("kind").and_then(Value::as_str);
    let rest: Vec<String> = fields
        .iter()
        .filter(|(name, value)| name.as_str() != "kind" && !value.is_null())
        .map(|(name, value)| format!("{name}={}", describe(value)))
        .collect();
    match (kind, rest.is_empty()) {
        (Some(kind), true) => kind.to_string(),
        (Some(kind), false) => format!("{kind}({})", rest.join(", ")),
        (None, _) => rest.join(", "),
    }
}

/// The field `name` when it is present and not null: Hermit writes the unused one of the two reason fields as null.
fn present<'a>(result: &'a Value, name: &str) -> Option<&'a Value> {
    result.get(name).filter(|v| !v.is_null())
}

fn run_number(error: &Value) -> String {
    let run = error.get("run").and_then(Value::as_str).unwrap_or("a run");
    match run.strip_prefix("run") {
        Some(n) if !n.is_empty() => format!("run {n}"),
        _ => run.to_string(),
    }
}

/// An `infrastructure_error` object in words. A changed host file is found after both runs were compared, so it is not
/// a verify that could not run.
fn infrastructure_text(error: &Value) -> String {
    match error.get("kind").and_then(Value::as_str) {
        Some("host_input_changed") => format!(
            "the two runs differed, but a host file the guest opened changed during {}: {}, so the difference is not counted against the guest",
            run_number(error),
            error.get("path").and_then(Value::as_str).unwrap_or("an unknown path"),
        ),
        _ => format!(
            "Hermit could not run the verify: {}",
            clip(&describe(error))
        ),
    }
}

/// The detail for a result file whose verdict reached no comparison (see [`is_no_comparison`]). Without a reason the
/// verdict stands in.
pub(super) fn no_comparison_text(result: &Value) -> String {
    let reason = match verdict_of(result) {
        "infrastructure_error" => present(result, "infrastructure_error")
            .map(infrastructure_text)
            .or_else(|| present(result, "no_result_reason").map(|r| clip(&describe(r)))),
        _ => present(result, "no_result_reason").map(|r| clip(&describe(r))),
    };
    match reason {
        Some(text) if text.starts_with("Hermit could not") || text.starts_with("the two runs") => {
            text
        }
        Some(text) => format!("Hermit could not run the verify: {text}"),
        None => format!(
            "Hermit could not run the verify: {} (no reason given)",
            verdict_of(result)
        ),
    }
}

/// `text`, followed by Hermit's own `Error:` line when it printed one that `text` does not already say.
pub(super) fn with_error_line(text: &str, error_line: Option<&str>) -> String {
    match error_line {
        Some(line) if !text.contains(line) => format!("{text}; Hermit said: {}", clip(line)),
        _ => text.to_string(),
    }
}

/// The detail for a verify the engine's own reading says Hermit failed (an infrastructure verdict on the run).
pub(super) fn hermit_failure(reason: &str, error_line: Option<&str>) -> String {
    with_error_line(
        &format!("Hermit could not run the verify: {reason}"),
        error_line,
    )
}

/// The detail for a verify stopped at its time limit. `limit` is what the run was given, which can be less than
/// twice a trial's time when the wall budget was nearly used up; a part of a second counts as a whole one.
pub(super) fn timed_out(limit: std::time::Duration) -> String {
    let secs = limit.as_secs() + u64::from(limit.subsec_nanos() > 0);
    format!("the verify run timed out after {secs} s, so it says nothing")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    /// A real Hermit result file (a rejected first run), kept whole.
    const REAL_REJECTED: &str = include_str!("../../tests/fixtures/verify/first_run_rejected.json");

    #[test]
    fn a_real_rejected_first_run_reads_with_its_non_null_fields_in_hermits_order() {
        let v: Value = serde_json::from_str(REAL_REJECTED).unwrap();
        assert!(is_no_comparison(&v));
        assert_eq!(
            no_comparison_text(&v),
            "Hermit could not run the verify: first_run_rejected(exit_code=0, stderr_bytes=3003, stdout_bytes=128)"
        );
    }

    #[test]
    fn run_failed_names_the_run_the_failure_and_the_detail() {
        let v = json!({"verdict": "no_result", "no_result_reason": {"kind": "run_failed", "run": "run2", "failure": "deadlock", "detail": "all threads blocked"}, "infrastructure_error": null});
        assert_eq!(
            no_comparison_text(&v),
            "Hermit could not run the verify: run_failed(detail=all threads blocked, failure=deadlock, run=run2)"
        );
    }

    #[test]
    fn container_failed_shows_its_nested_disposition() {
        let v = json!({"verdict": "no_result", "no_result_reason": {"kind": "container_failed", "run": "run1", "disposition": {"kind": "exited", "code": 125}}});
        assert_eq!(
            no_comparison_text(&v),
            "Hermit could not run the verify: container_failed(disposition=exited(code=125), run=run1)"
        );
        let signalled = json!({"verdict": "no_result", "no_result_reason": {"kind": "container_failed", "run": "run1", "disposition": {"kind": "signaled", "signal": 9, "core_dumped": false}}});
        assert!(no_comparison_text(&signalled)
            .contains("disposition=signaled(core_dumped=false, signal=9)"));
    }

    #[test]
    fn a_skid_overshoot_is_read_from_infrastructure_error_and_a_null_reason_is_missing() {
        let v = json!({"verdict": "infrastructure_error", "no_result_reason": null, "infrastructure_error": {"kind": "skid_overshoot", "count": 3}});
        assert_eq!(
            no_comparison_text(&v),
            "Hermit could not run the verify: skid_overshoot(count=3)"
        );
    }

    #[test]
    fn a_changed_host_file_is_not_called_a_verify_that_could_not_run() {
        let v = json!({"verdict": "infrastructure_error", "no_result_reason": null, "infrastructure_error": {"kind": "host_input_changed", "run": "run1", "path": "/etc/hosts", "before": {}, "after": {}}});
        let text = no_comparison_text(&v);
        assert!(
            text.starts_with("the two runs differed, but a host file"),
            "{text}"
        );
        assert!(text.contains("during run 1: /etc/hosts"), "{text}");
        assert!(!text.contains("could not run"), "{text}");
    }

    #[test]
    fn a_missing_or_null_reason_falls_back_to_the_verdict() {
        for v in [
            json!({"verdict": "no_result"}),
            json!({"verdict": "no_result", "no_result_reason": null}),
            json!({"verdict": "infrastructure_error", "infrastructure_error": null, "no_result_reason": null}),
        ] {
            let text = no_comparison_text(&v);
            assert!(text.ends_with("(no reason given)"), "{text}");
            assert!(!text.contains("null"), "{text}");
        }
    }

    #[test]
    fn a_plain_string_reason_and_a_kind_without_fields_read_plainly() {
        let plain = json!({"verdict": "no_result", "no_result_reason": "odd"});
        assert!(no_comparison_text(&plain).ends_with(": odd"));
        let bare = json!({"verdict": "no_result", "no_result_reason": {"kind": "not_run"}});
        assert!(no_comparison_text(&bare).ends_with(": not_run"));
    }

    #[test]
    fn only_the_no_comparison_verdicts_are_recognised() {
        assert!(is_no_comparison(&json!({"verdict": "no_result"})));
        assert!(is_no_comparison(
            &json!({"verdict": "infrastructure_error"})
        ));
        assert!(!is_no_comparison(&json!({"verdict": "matched"})));
        assert!(!is_no_comparison(&json!({})));
    }

    #[test]
    fn hermits_own_words_are_cut_to_a_bound_and_never_repeated() {
        let long = "x".repeat(MAX_QUOTED_CHARS * 2);
        let text = hermit_failure("why", Some(&long));
        assert!(text.chars().count() < MAX_QUOTED_CHARS + 80);
        assert!(hermit_failure("why", None).ends_with("why"));
        let again = hermit_failure("hermit failed: Error: X", Some("Error: X"));
        assert_eq!(again.matches("Error: X").count(), 1, "{again}");
    }

    #[test]
    fn the_timeout_is_the_limit_rounded_up_to_a_whole_second() {
        assert!(timed_out(Duration::from_secs(2)).contains("after 2 s"));
        assert!(timed_out(Duration::from_millis(1500)).contains("after 2 s"));
        assert!(timed_out(Duration::from_millis(200)).contains("after 1 s"));
    }
}

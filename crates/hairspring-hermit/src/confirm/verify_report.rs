//! What the strict verify's detail says when Hermit did not deliver a verdict: a timeout, or a failure of Hermit itself.
use serde_json::Value;

/// The most characters of Hermit's own words the detail carries, so a long line cannot bury the rest.
const MAX_QUOTED_CHARS: usize = 300;
/// The verdicts Hermit gives a verify that did not compare two runs.
const NO_COMPARISON_VERDICTS: [&str; 2] = ["no_result", "infrastructure_error"];

fn clip(text: &str) -> String {
    text.chars().take(MAX_QUOTED_CHARS).collect()
}

/// True when Hermit's result file says the verify never compared two runs.
pub(super) fn is_no_comparison(result: &Value) -> bool {
    result
        .get("verdict")
        .and_then(Value::as_str)
        .is_some_and(|v| NO_COMPARISON_VERDICTS.contains(&v))
}

fn field_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

/// Hermit's `no_result_reason`, an object tagged by `kind`, as `kind(field=value, ..)`. A plain string is kept as
/// written; a missing reason falls back to the verdict.
pub(super) fn reason_text(result: &Value) -> String {
    let verdict = result
        .get("verdict")
        .and_then(Value::as_str)
        .unwrap_or("no verdict");
    let Some(reason) = result.get("no_result_reason") else {
        return verdict.to_string();
    };
    let Some(fields) = reason.as_object() else {
        return clip(&field_text(reason));
    };
    let kind = fields
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or(verdict);
    let rest: Vec<String> = fields
        .iter()
        .filter(|(name, _)| name.as_str() != "kind")
        .map(|(name, value)| format!("{name}={}", field_text(value)))
        .collect();
    clip(&if rest.is_empty() {
        kind.to_string()
    } else {
        format!("{kind}({})", rest.join(", "))
    })
}

/// `text`, followed by Hermit's own `Error:` line when it printed one.
pub(super) fn with_error_line(text: &str, error_line: Option<&str>) -> String {
    match error_line {
        Some(line) => format!("{text}; Hermit said: {}", clip(line)),
        None => text.to_string(),
    }
}

/// The detail for a verify Hermit could not run: why, and Hermit's own `Error:` line when it printed one.
pub(super) fn hermit_failure(reason: &str, error_line: Option<&str>) -> String {
    with_error_line(
        &format!("Hermit could not run the verify: {reason}"),
        error_line,
    )
}

/// The detail for a verify stopped at its time limit.
pub(super) fn timed_out(limit_secs: u64) -> String {
    format!("the verify run timed out after {limit_secs} s, so it says nothing")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tagged_reason_reads_as_kind_and_fields() {
        let v = json!({"verdict": "no_result", "no_result_reason": {"kind": "first_run_rejected", "exit_code": 0}});
        assert_eq!(reason_text(&v), "first_run_rejected(exit_code=0)");
    }

    #[test]
    fn a_reason_without_fields_is_its_kind() {
        let v = json!({"verdict": "no_result", "no_result_reason": {"kind": "gave_up"}});
        assert_eq!(reason_text(&v), "gave_up");
    }

    #[test]
    fn a_plain_string_reason_is_kept_and_a_missing_one_falls_back_to_the_verdict() {
        let plain = json!({"no_result_reason": "odd"});
        assert_eq!(reason_text(&plain), "odd");
        let none = json!({"verdict": "infrastructure_error"});
        assert_eq!(reason_text(&none), "infrastructure_error");
        assert_eq!(reason_text(&json!({})), "no verdict");
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
    fn hermits_own_words_are_cut_to_a_bound() {
        let long = "x".repeat(MAX_QUOTED_CHARS * 2);
        let text = hermit_failure("why", Some(&long));
        assert!(text.chars().count() < MAX_QUOTED_CHARS + 80);
        assert!(hermit_failure("why", None).ends_with("why"));
    }
}

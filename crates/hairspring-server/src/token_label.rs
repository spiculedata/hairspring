//! Operator-set token labels: a short, readable name for a token, written only in the token configuration.
//!
//! A label is not a secret and not proof of identity (two people sharing a token share its label). The rules are
//! strict so that a label is safe to write into a log line and cannot be mistaken for another label, a token, or a
//! caller key.

pub const MAX_LABEL_CHARS: usize = 64;

fn is_label_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-')
}

/// Checks the shape of one label. The error never repeats the label.
pub fn check_shape(label: &str) -> Result<(), String> {
    let count = label.chars().count();
    if count == 0 || count > MAX_LABEL_CHARS {
        return Err(format!("a label must be 1 to {MAX_LABEL_CHARS} characters"));
    }
    if !label.chars().all(is_label_char) {
        return Err(
            "a label may use only ASCII letters, digits, space, `.`, `_` and `-`".to_string(),
        );
    }
    if label.starts_with(' ') || label.ends_with(' ') || label.contains("  ") {
        return Err("a label may not start or end with a space or hold two spaces in a row".into());
    }
    Ok(())
}

/// Whether `label` holds any of `tokens`, ignoring case. A label that did would publish a secret in every log line.
pub fn holds_a_token<'a>(label: &str, tokens: impl IntoIterator<Item = &'a str>) -> bool {
    let label = label.to_ascii_lowercase();
    tokens
        .into_iter()
        .any(|t| label.contains(&t.to_ascii_lowercase()))
}

/// The form used to compare labels for uniqueness.
pub fn comparison_form(label: &str) -> String {
    label.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_labels_are_accepted() {
        for ok in ["ci", "alice", "worker-eu.1", "build_bot 2", "A"] {
            assert!(check_shape(ok).is_ok(), "{ok}");
        }
        assert!(check_shape(&"a".repeat(MAX_LABEL_CHARS)).is_ok());
    }

    #[test]
    fn empty_overlong_and_oddly_spaced_labels_are_refused() {
        for bad in [
            String::new(),
            "a".repeat(MAX_LABEL_CHARS + 1),
            " lead".into(),
            "trail ".into(),
            "two  spaces".into(),
        ] {
            assert!(check_shape(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn control_bidi_separator_non_ascii_and_delimiter_characters_are_refused() {
        for bad in [
            "bell\u{7}",
            "nul\0",
            "new\nline",
            "tab\there",
            "esc\u{1b}[31m",
            "rlo\u{202e}gnp",
            "ls\u{2028}x",
            "ps\u{2029}x",
            "zwsp\u{200b}x",
            "nbsp\u{a0}x",
            "caf\u{e9}",
            "\u{430}lice",
            "a:b",
            "a,b",
            "a=b",
            "a\"b",
            "a%b",
        ] {
            assert!(check_shape(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_label_holding_a_token_is_found_in_any_case() {
        let tokens = ["secret-token-0123456789"];
        assert!(holds_a_token("secret-token-0123456789", tokens));
        assert!(holds_a_token("x SECRET-TOKEN-0123456789 y", tokens));
        assert!(!holds_a_token("ci", tokens));
    }
}

//! The organisation name a server may show in the web UI's top bar. It is plain text from the operator's configuration:
//! no markup, no logo, nothing to upload. It is checked once, when the server starts, so every reader can print it as is.
use hairspring_core::environment::is_unsafe_display_char;

/// Longest name, in characters.
pub const MAX_ORGANISATION_CHARS: usize = 80;

/// The name to show, from the text configured. Surrounding white space is dropped and an empty name means none is set.
/// A name with a control character or a character that reorders or hides text (a line break, a bidirectional override,
/// a zero-width character), or one longer than [`MAX_ORGANISATION_CHARS`], is an error that does not repeat the value.
pub fn parse_organisation(raw: &str) -> Result<Option<String>, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > MAX_ORGANISATION_CHARS {
        return Err(format!(
            "the organisation name is longer than {MAX_ORGANISATION_CHARS} characters"
        ));
    }
    if name.chars().any(is_unsafe_display_char) {
        return Err(
            "the organisation name has a control or invisible formatting character".to_owned(),
        );
    }
    Ok(Some(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_name_is_kept_without_its_surrounding_space() {
        assert_eq!(
            parse_organisation("  Acme Ltd \t").unwrap().as_deref(),
            Some("Acme Ltd")
        );
        assert_eq!(
            parse_organisation("Société Générale – 研究所")
                .unwrap()
                .as_deref(),
            Some("Société Générale – 研究所")
        );
    }

    #[test]
    fn an_empty_or_blank_name_means_none_is_set() {
        for blank in ["", "   ", "\n\t "] {
            assert_eq!(parse_organisation(blank), Ok(None), "{blank:?}");
        }
    }

    #[test]
    fn the_length_limit_counts_characters_not_bytes() {
        let at_limit = "é".repeat(MAX_ORGANISATION_CHARS);
        assert_eq!(parse_organisation(&at_limit), Ok(Some(at_limit.clone())));
        let over = format!("{at_limit}é");
        let err = parse_organisation(&over).unwrap_err();
        assert!(err.contains("80"), "{err}");
        assert!(!err.contains('é'), "the value is not repeated: {err}");
    }

    #[test]
    fn control_and_invisible_formatting_characters_are_refused() {
        for bad in [
            "a\nb",
            "a\tb",
            "a\u{0}b",
            "a\u{1b}[31mb",
            "a\u{202E}b",
            "a\u{200B}b",
            "a\u{2066}b",
            "a\u{FEFF}b",
        ] {
            let err = parse_organisation(bad).unwrap_err();
            assert!(err.contains("control or invisible"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn markup_is_ordinary_text_here_and_left_to_the_page_to_escape() {
        assert_eq!(
            parse_organisation("<b>Acme</b> & Sons").unwrap().as_deref(),
            Some("<b>Acme</b> & Sons")
        );
    }
}

//! A small reader for the parts of apt's configuration syntax that decide whether unattended upgrades run.
//!
//! Understood: `//` and `/* */` comments, `#` comment lines, quoted values, several statements on a line, nested
//! scopes (`APT::Periodic { Unattended-Upgrade "1"; };`), case-insensitive names. Not understood: `#include` and
//! `#clear` directives, and the `-o` / `APT_CONFIG` overrides of a particular apt invocation.
use super::autoupdate::AutoUpdates;

const UNATTENDED: &str = "apt::periodic::unattended-upgrade";
const ENABLE: &str = "apt::periodic::enable";

/// The state the configuration texts describe, given in the order apt reads them (the last assignment wins). `None`
/// when none of them sets `APT::Periodic::Unattended-Upgrade` or `APT::Periodic::Enable`.
pub fn evaluate(texts: &[String]) -> Option<AutoUpdates> {
    let assignments: Vec<(String, String)> = texts.iter().flat_map(|t| assignments(t)).collect();
    let last = |key: &str| {
        assignments
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let (enable, unattended) = (last(ENABLE), last(UNATTENDED));
    if enable.is_none() && unattended.is_none() {
        return None;
    }
    if enable.map(interval_days) == Some(Some(0)) {
        return Some(AutoUpdates::Disabled);
    }
    Some(match unattended.map(interval_days) {
        None => AutoUpdates::Disabled,
        Some(Some(0)) => AutoUpdates::Disabled,
        Some(Some(_)) => AutoUpdates::Enabled,
        Some(None) => AutoUpdates::Unknown,
    })
}

/// A number of days or an interval such as `1`, `7`, `0d`, `2w`; zero means off. `None` for anything else (such as
/// `always`), which this reader does not interpret.
fn interval_days(value: &str) -> Option<u64> {
    let value = value.trim();
    let digits_end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let (digits, suffix) = value.split_at(digits_end);
    if !matches!(suffix, "" | "s" | "m" | "h" | "d" | "w") {
        return None;
    }
    digits.parse().ok()
}

/// Every `name "value"` assignment in the text, in order, with its scopes joined by `::` and lower-cased.
fn assignments(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut scopes: Vec<String> = Vec::new();
    let mut tag: Option<String> = None;
    for token in tokens(&strip_comments(text)) {
        match token {
            Token::Word(word) => tag = Some(word),
            Token::Quoted(value) => {
                if let Some(name) = tag.take() {
                    let key = scopes
                        .iter()
                        .chain(std::iter::once(&name))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("::");
                    found.push((key.to_ascii_lowercase(), value));
                }
            }
            Token::Open => scopes.push(tag.take().unwrap_or_default()),
            Token::Close => {
                scopes.pop();
            }
            Token::End => tag = None,
        }
    }
    found.into_iter().map(drop_empty_scopes).collect()
}

fn drop_empty_scopes((key, value): (String, String)) -> (String, String) {
    let clean: Vec<&str> = key.split("::").filter(|part| !part.is_empty()).collect();
    (clean.join("::"), value)
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
    Open,
    Close,
    End,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let value: String = chars.by_ref().take_while(|c| *c != '"').collect();
                out.push(Token::Quoted(value));
            }
            '{' => out.push(Token::Open),
            '}' => out.push(Token::Close),
            ';' => out.push(Token::End),
            c if c.is_whitespace() => {}
            first => {
                let mut word = String::from(first);
                while let Some(next) =
                    chars.next_if(|n| !n.is_whitespace() && !"\"{};".contains(*n))
                {
                    word.push(next);
                }
                out.push(Token::Word(word));
            }
        }
    }
    out
}

/// The text without `//` comments, `/* */` comments and `#` comment lines. Quoted text is left alone, so a `//` in
/// a URL survives. An unterminated block comment runs to the end.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let (mut in_quote, mut at_line_start) = (false, true);
    while let Some(c) = chars.next() {
        match (c, in_quote) {
            ('"', _) => {
                in_quote = !in_quote;
                out.push(c);
            }
            ('/', false) if chars.peek() == Some(&'/') => skip_line(&mut chars),
            ('/', false) if chars.peek() == Some(&'*') => skip_block(&mut chars),
            ('#', false) if at_line_start => skip_line(&mut chars),
            _ => out.push(c),
        }
        at_line_start = c == '\n' || (at_line_start && c.is_whitespace());
        if c == '\n' {
            in_quote = false;
        }
    }
    out
}

fn skip_line(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while chars.next_if(|c| *c != '\n').is_some() {}
}

fn skip_block(chars: &mut std::iter::Peekable<std::str::Chars>) {
    chars.next();
    let mut previous = ' ';
    for c in chars.by_ref() {
        if previous == '*' && c == '/' {
            return;
        }
        previous = c;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(texts: &[&str]) -> Option<AutoUpdates> {
        evaluate(&texts.iter().map(|t| t.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_nonzero_setting_is_enabled_and_zero_is_disabled() {
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"1\";"]),
            Some(AutoUpdates::Enabled)
        );
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"7\";"]),
            Some(AutoUpdates::Enabled)
        );
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"0\";"]),
            Some(AutoUpdates::Disabled)
        );
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"0d\";"]),
            Some(AutoUpdates::Disabled)
        );
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"2w\";"]),
            Some(AutoUpdates::Enabled)
        );
    }

    #[test]
    fn nothing_about_the_setting_is_none() {
        assert_eq!(eval(&[]), None);
        assert_eq!(eval(&["Acquire::Retries \"3\";"]), None);
    }

    #[test]
    fn the_last_assignment_wins_across_texts() {
        let later_off = [
            "APT::Periodic::Unattended-Upgrade \"1\";",
            "APT::Periodic::Unattended-Upgrade \"0\";",
        ];
        assert_eq!(eval(&later_off), Some(AutoUpdates::Disabled));
        let later_on = [
            "APT::Periodic::Unattended-Upgrade \"0\";",
            "APT::Periodic::Unattended-Upgrade \"1\";",
        ];
        assert_eq!(eval(&later_on), Some(AutoUpdates::Enabled));
    }

    #[test]
    fn an_uninterpretable_last_assignment_is_unknown_not_the_earlier_value() {
        let later_odd = [
            "APT::Periodic::Unattended-Upgrade \"1\";",
            "APT::Periodic::Unattended-Upgrade \"always\";",
        ];
        assert_eq!(eval(&later_odd), Some(AutoUpdates::Unknown));
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"1x\";"]),
            Some(AutoUpdates::Unknown)
        );
        assert_eq!(
            eval(&["APT::Periodic::Unattended-Upgrade \"\";"]),
            Some(AutoUpdates::Unknown)
        );
    }

    #[test]
    fn enable_zero_turns_everything_off() {
        let off = ["APT::Periodic::Unattended-Upgrade \"1\";\nAPT::Periodic::Enable \"0\";"];
        assert_eq!(eval(&off), Some(AutoUpdates::Disabled));
        let on_again = [
            "APT::Periodic::Enable \"0\";",
            "APT::Periodic::Enable \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";",
        ];
        assert_eq!(eval(&on_again), Some(AutoUpdates::Enabled));
    }

    #[test]
    fn names_match_regardless_of_case() {
        assert_eq!(
            eval(&["apt::periodic::unattended-upgrade \"1\";"]),
            Some(AutoUpdates::Enabled)
        );
        assert_eq!(
            eval(&["Apt::PERIODIC::Unattended-upgrade \"1\";"]),
            Some(AutoUpdates::Enabled)
        );
    }

    #[test]
    fn comments_of_every_kind_are_ignored() {
        let text = "// APT::Periodic::Unattended-Upgrade \"1\";\n# APT::Periodic::Unattended-Upgrade \"1\";\n/* APT::Periodic::Unattended-Upgrade \"1\";\n APT::Periodic::Unattended-Upgrade \"1\"; */\n";
        assert_eq!(eval(&[text]), None);
        let trailing = "APT::Periodic::Unattended-Upgrade \"1\"; // on\n";
        assert_eq!(eval(&[trailing]), Some(AutoUpdates::Enabled));
        let block_then_real = "/* off */ APT::Periodic::Unattended-Upgrade \"1\";";
        assert_eq!(eval(&[block_then_real]), Some(AutoUpdates::Enabled));
    }

    #[test]
    fn a_double_slash_inside_quotes_is_not_a_comment() {
        let text =
            "Acquire::http::Proxy \"http://proxy:3142\"; APT::Periodic::Unattended-Upgrade \"1\";";
        assert_eq!(eval(&[text]), Some(AutoUpdates::Enabled));
    }

    #[test]
    fn scoped_blocks_and_several_statements_on_a_line_are_read() {
        let scoped = "APT::Periodic { Unattended-Upgrade \"1\"; };";
        assert_eq!(eval(&[scoped]), Some(AutoUpdates::Enabled));
        let nested = "APT { Periodic { Enable \"1\"; Unattended-Upgrade \"0\"; }; };";
        assert_eq!(eval(&[nested]), Some(AutoUpdates::Disabled));
        let one_line =
            "APT::Periodic::Update-Package-Lists \"1\"; APT::Periodic::Unattended-Upgrade \"1\";";
        assert_eq!(eval(&[one_line]), Some(AutoUpdates::Enabled));
    }

    #[test]
    fn a_scope_does_not_leak_to_the_next_statement() {
        let text = "Other { Unattended-Upgrade \"1\"; };\nUnattended-Upgrade \"1\";";
        assert_eq!(eval(&[text]), None);
    }
}

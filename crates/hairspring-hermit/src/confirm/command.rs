//! The Hermit command a check ran, written as a record of the invocation (shell-quoted so it reads unambiguously).
//!
//! It is not a paste-and-run line: a run clears the guest's environment and sets `HAIRSPRING_SEED`, the backend's
//! `extra_env` and the events variables outside the command line, and runs in a private directory. The target's full
//! argv and `hermit_args` are included, so a credential passed in them is exposed to whoever may read the check.
//!
//! The program is named by its file name only (a stored command is read by other people, and the path to Hermit on one
//! host is not part of what ran). The per-run plumbing Hermit is given (the summary file, the events directory) is left
//! out, and the strict verify's result file is written as `verify.json`.
use crate::exec::{hermit_args, ProcessSpec};
use crate::HermitSettings;

/// Characters that need no quoting in a shell word.
fn is_plain(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '-' | '_' | '.' | '/' | '=' | ':' | ',' | '+' | '@' | '%')
}

fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(is_plain) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// `hermit run ... -- <command>` for one run at `seed`, with `extra` Hermit flags.
pub(super) fn line(
    settings: &HermitSettings,
    p: &ProcessSpec,
    seed: u64,
    extra: &[String],
) -> String {
    let program = settings.bin.file_name().map_or_else(
        || "hermit".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    std::iter::once(program)
        .chain(hermit_args(settings, p, seed, None, None, extra))
        .map(|w| quote(&w))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn plain_words_are_left_alone_and_others_quoted() {
        assert_eq!(quote("--sched-seed=3"), "--sched-seed=3");
        assert_eq!(quote("echo fine"), "'echo fine'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }
}

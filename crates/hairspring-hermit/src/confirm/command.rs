//! The Hermit command a check ran, written as a record of the invocation (shell-quoted so it reads unambiguously).
//!
//! It is not a paste-and-run line: a run clears the guest's environment and sets `HAIRSPRING_SEED`, the backend's
//! `extra_env` and the events variables outside the command line, and runs in a private directory. The target's full
//! argv and `hermit_args` are included, so a credential passed in them is exposed to whoever may read the check.
//!
//! The program is named by its file name only (a stored command is read by other people, and the path to Hermit on one
//! host is not part of what ran). The per-run plumbing Hermit is given (the summary file, the events directory) is left
//! out, and the strict verify's result file is written as `verify.json`.
use crate::exec::{hermit_args_for, ProcessSpec};
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
/// The variables the engine sets for Hermit's own temporary files (`TMPDIR`, `HERMIT_VERIFY_TMPDIR`) are left out, like
/// the rest of the environment: they belong to the engine's clean-up and are not part of what a replay needs.
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
        .chain(hermit_args_for(
            settings,
            p,
            seed,
            None,
            None,
            settings.events_file,
            extra,
        ))
        .map(|w| quote(&w))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_engine::scheduler::SchedulerSpec;
    use std::collections::BTreeMap;

    #[test]
    fn plain_words_are_left_alone_and_others_quoted() {
        assert_eq!(quote("--sched-seed=3"), "--sched-seed=3");
        assert_eq!(quote("echo fine"), "'echo fine'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    fn spec<'a>(
        env: &'a BTreeMap<String, String>,
        command: &'a [String],
        hermit_args: &'a [String],
        scheduler: &'a SchedulerSpec,
    ) -> ProcessSpec<'a> {
        ProcessSpec {
            target: "svc",
            command,
            env,
            cwd: None,
            hermit_args,
            trial_timeout_secs: None,
            scheduler,
            hang_is_violation: false,
            parallelism: 1,
            pass_env: true,
        }
    }

    #[test]
    fn the_stored_command_names_the_passed_variables_and_never_a_value() {
        let env = BTreeMap::from([("API_TOKEN".to_string(), "s3cret-value".to_string())]);
        let command = vec!["./svc".to_string()];
        let hermit_args = vec!["--base-env=minimal".to_string()];
        let scheduler = SchedulerSpec::default();
        let p = spec(&env, &command, &hermit_args, &scheduler);
        let settings = HermitSettings {
            events_file: true,
            ..HermitSettings::default()
        };
        let line = line(&settings, &p, 41, &[]);
        for name in [
            "API_TOKEN",
            "HAIRSPRING_SEED",
            "HAIRSPRING_PROC",
            "HAIRSPRING_EVENTS_FILE",
            "PATH",
        ] {
            assert!(line.contains(&format!("--env={name} ")), "{name}: {line}");
        }
        assert!(!line.contains("s3cret-value"), "{line}");
        assert!(!line.contains("--env=HAIRSPRING_SEED="), "{line}");
        assert!(!line.contains("SEED=41"), "{line}");
    }

    #[test]
    fn with_passing_off_the_stored_command_has_no_env_words() {
        let env = BTreeMap::new();
        let command = vec!["./svc".to_string()];
        let hermit_args = vec!["--base-env=minimal".to_string()];
        let scheduler = SchedulerSpec::default();
        let p = ProcessSpec {
            pass_env: false,
            ..spec(&env, &command, &hermit_args, &scheduler)
        };
        let line = line(&HermitSettings::default(), &p, 1, &[]);
        assert!(!line.contains("--env="), "{line}");
    }
}

//! The warning for a target whose Hermit run config, manifest or image decides the guest's environment.
//!
//! Under `--base-env=minimal` and `empty` Hairspring names its variables with `--env=NAME`. A `--config` or `--from`
//! file replaces an `--env` list given on the command line, and an `--image` brings its own environment, so there the
//! names are not added and the program gets the variables only if the file or image provides them. That is said once
//! per target on standard error, like the other warnings about a target's setup.
use crate::exec::ProcessSpec;
use hairspring_engine::hermit_args::{uses_image, uses_run_config};
use std::collections::BTreeSet;
use std::sync::Mutex;

static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// What to tell the user about this target, if anything. The target's name is checked when the configuration loads;
/// nothing else the user wrote is repeated.
pub(crate) fn message(target: &str, hermit_args: &[String], pass_env: bool) -> Option<String> {
    if !pass_env {
        return None;
    }
    let source = if uses_run_config(hermit_args) {
        "a Hermit `--config` or `--from` file"
    } else if uses_image(hermit_args) {
        "a Hermit `--image`"
    } else {
        return None;
    };
    Some(format!(
        "target {target}: `hermit_args` name {source}, so Hairspring does not add HAIRSPRING_SEED, HAIRSPRING_PROC or the \
         target's `env` to the program's environment; they reach it only if that file or image passes them \
         (`pass_env = false` silences this warning)"
    ))
}

/// Prints [`message`] the first time it is asked about `p`'s target.
pub(crate) fn warn_once(p: &ProcessSpec) {
    let Some(text) = message(p.target, p.hermit_args, p.pass_env) else {
        return;
    };
    let first = WARNED
        .lock()
        .map_or(true, |mut warned| warned.insert(p.target.to_string()));
    if first {
        eprintln!("hairspring: warning: {text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn a_run_config_manifest_or_image_with_passing_on_is_warned_about() {
        for list in [
            &["--config=f.yaml"][..],
            &["--base-env=minimal", "--from", "m.yaml"][..],
        ] {
            let text = message("svc", &args(list), true).unwrap();
            assert!(text.starts_with("target svc: "), "{text}");
            assert!(text.contains("`--config` or `--from`"), "{text}");
            assert!(
                !text.contains("f.yaml") && !text.contains("m.yaml"),
                "{text}"
            );
        }
        let image = message("svc", &args(&["--image=x"]), true).unwrap();
        assert!(image.contains("`--image`"), "{image}");
        assert!(!image.contains("=x"), "{image}");
    }

    #[test]
    fn nothing_is_said_without_those_flags_or_with_passing_off() {
        assert!(message("svc", &args(&["--base-env=minimal"]), true).is_none());
        assert!(message("svc", &args(&[]), true).is_none());
        assert!(message("svc", &args(&["--config=f"]), false).is_none());
        assert!(message("svc", &args(&["--image=x"]), false).is_none());
    }

    #[test]
    fn the_message_is_one_line() {
        let text = message("svc", &args(&["--config=f"]), true).unwrap();
        assert!(!text.contains('\n'), "{text}");
    }
}

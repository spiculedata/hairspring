//! Reading a process target's `hermit_args` for the environment they give the guest.
//!
//! Hermit builds the guest's environment from a base (`--base-env`) and then each `--env`. Under `minimal` and `empty`
//! it starts from almost nothing, so the variables Hairspring sets on Hermit's own process never reach the guest unless
//! each is named with `--env=NAME`. These functions decide, from the arguments alone, whether Hairspring adds those
//! names and which names the target already passes itself. They read arguments only; no value is ever looked at.
use std::collections::BTreeSet;

const BASE_ENV_FLAG: &str = "--base-env";
const BASE_ENV_DEFAULT: &str = "default";
const BASE_ENV_UNKNOWN: &str = "unknown";
const BASE_ENV_MODES: [&str; 3] = ["minimal", "host", "empty"];
/// Base environments that start the guest from (almost) nothing.
const SELF_CONTAINED: [&str; 2] = ["minimal", "empty"];
/// Flags that make Hermit read its environment from somewhere Hairspring cannot see or must not override.
const RUN_CONFIG_FLAGS: [&str; 2] = ["--config", "--from"];
const IMAGE_FLAG: &str = "--image";
/// Variables the engine owns for the guest; a target may not pass them itself.
pub const ENGINE_OWNED_VARS: [&str; 3] = [
    "HAIRSPRING_PROC",
    "HAIRSPRING_PROC_PID",
    "HAIRSPRING_EVENTS_FILE",
];

/// The base-environment mode Hermit was asked for, lower-cased because Hermit reads the value that way. With no flag
/// Hermit's own default decides, which this cannot see; a flag with a value we do not know, or none, is `unknown`.
/// The last `--base-env` wins.
pub fn base_env_mode(args: &[String]) -> String {
    let mut mode = BASE_ENV_DEFAULT.to_string();
    let mut i = 0;
    while i < args.len() {
        let value = if args[i] == BASE_ENV_FLAG {
            i += 1;
            Some(args.get(i).map(String::as_str))
        } else {
            args[i]
                .strip_prefix(BASE_ENV_FLAG)
                .and_then(|rest| rest.strip_prefix('='))
                .map(Some)
        };
        if let Some(value) = value {
            let lowered = value.map(str::to_ascii_lowercase);
            mode = match lowered {
                Some(v) if BASE_ENV_MODES.contains(&v.as_str()) => v,
                _ => BASE_ENV_UNKNOWN.to_string(),
            };
        }
        i += 1;
    }
    mode
}

/// The `NAME[=VALUE]` text of every `--env` the arguments give Hermit, as the pinned Hermit's parser reads them:
/// `--env X`, `--env=X`, `-e X`, `-e=X`, `-eX`, and `-e` at the end of a cluster of short flags, where `-u` (a flag
/// without a value) may come first (`-ueX`, `-uu-e`...). `-e` takes the rest of its cluster as its value.
fn env_specs(args: &[String]) -> Vec<&str> {
    let mut specs = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--env" {
            i += 1;
            specs.extend(args.get(i).map(String::as_str));
        } else if let Some(spec) = arg.strip_prefix("--env=") {
            specs.push(spec);
        } else if let Some(cluster) = arg.strip_prefix('-').filter(|rest| !rest.starts_with('-')) {
            let rest = cluster.trim_start_matches('u');
            if let Some(value) = rest.strip_prefix('e') {
                let value = value.strip_prefix('=').unwrap_or(value);
                if value.is_empty() {
                    i += 1;
                    specs.extend(args.get(i).map(String::as_str));
                } else {
                    specs.push(value);
                }
            }
        }
        i += 1;
    }
    specs
}

fn name_of(spec: &str) -> &str {
    spec.split('=').next().unwrap_or(spec)
}

/// The names of the variables the arguments set for the guest with `--env`, with or without a value.
pub fn names_passed(args: &[String]) -> BTreeSet<String> {
    env_specs(args)
        .into_iter()
        .map(|spec| name_of(spec).to_string())
        .collect()
}

/// The names the arguments pass through from Hermit's own environment: `--env NAME` without a value. A name given a
/// value is not among them, because it says nothing about Hermit's own variable of that name.
pub fn names_passed_through(args: &[String]) -> BTreeSet<String> {
    env_specs(args)
        .into_iter()
        .filter(|spec| !spec.contains('='))
        .map(str::to_string)
        .collect()
}

fn names_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| {
        a == flag
            || a.strip_prefix(flag)
                .is_some_and(|rest| rest.starts_with('='))
    })
}

/// Whether Hermit is given a run config file or a manifest (`--config`, `--from`), which carries its own environment.
pub fn uses_run_config(args: &[String]) -> bool {
    RUN_CONFIG_FLAGS.iter().any(|f| names_flag(args, f))
}

/// Whether Hermit is given a container image (`--image`), which brings its own environment.
pub fn uses_image(args: &[String]) -> bool {
    names_flag(args, IMAGE_FLAG)
}

/// Whether the arguments leave the guest's environment to something Hairspring cannot add to (a config file, a
/// manifest or an image).
pub fn environment_set_elsewhere(args: &[String]) -> bool {
    uses_run_config(args) || uses_image(args)
}

/// Whether Hairspring adds `--env=NAME` entries for the variables it sets: the target has not turned it off, the base
/// environment is `minimal` or `empty`, and nothing else decides the environment.
pub fn pass_through_active(hermit_args: &[String], pass_env: bool) -> bool {
    pass_env
        && !environment_set_elsewhere(hermit_args)
        && SELF_CONTAINED.contains(&base_env_mode(hermit_args).as_str())
}

/// Whether `name` can be the name in `--env=NAME`: printable ASCII without `=`. Hermit also accepts the control
/// characters BEL to US; those are refused here so a name can always be shown and quoted on one line.
pub fn is_passable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| (0x20..=0x7e).contains(&b) && b != b'=')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn names(list: &[&str]) -> Vec<String> {
        names_passed(&args(list)).into_iter().collect()
    }

    #[test]
    fn the_base_environment_is_read_case_insensitively_and_the_last_flag_wins() {
        assert_eq!(base_env_mode(&args(&[])), "default");
        assert_eq!(base_env_mode(&args(&["--base-env=Minimal"])), "minimal");
        assert_eq!(base_env_mode(&args(&["--base-env", "EMPTY"])), "empty");
        assert_eq!(
            base_env_mode(&args(&["--base-env=minimal", "--base-env=host"])),
            "host"
        );
        assert_eq!(base_env_mode(&args(&["--base-env=surprise"])), "unknown");
        assert_eq!(base_env_mode(&args(&["--base-env"])), "unknown");
    }

    #[test]
    fn every_spelling_of_env_names_its_variable() {
        for spelling in [
            &["--env=A"][..],
            &["--env", "A"][..],
            &["-e", "A"][..],
            &["-eA"][..],
            &["-e=A"][..],
            &["--env=A=1"][..],
            &["--env", "A=1"][..],
            &["-e", "A=1"][..],
            &["-eA=1"][..],
            &["-e=A=1"][..],
        ] {
            assert_eq!(names(spelling), ["A"], "{spelling:?}");
        }
    }

    #[test]
    fn short_flag_clusters_are_read_as_the_pinned_parser_reads_them() {
        for spelling in [
            &["-ueA"][..],
            &["-uuea"][..],
            &["-ue", "A"][..],
            &["-ue=A"][..],
            &["-ueA=1"][..],
            &["-u", "-eA"][..],
        ] {
            let got = names(spelling);
            assert!(got == ["A"] || got == ["a"], "{spelling:?}: {got:?}");
        }
        // `-e` takes the rest of the cluster as its value.
        assert_eq!(names(&["-eu"]), ["u"]);
        // A cluster without `-e`, and `-u` alone, name nothing.
        assert!(names(&["-u", "-uu", "-x"]).is_empty());
    }

    #[test]
    fn engine_owned_names_are_found_in_every_spelling() {
        for spelling in [
            &["-ueHAIRSPRING_PROC=x"][..],
            &["-ueHAIRSPRING_PROC"][..],
            &["-ue", "HAIRSPRING_PROC=x"][..],
            &["-ue=HAIRSPRING_PROC"][..],
        ] {
            assert!(
                names_passed(&args(spelling)).contains("HAIRSPRING_PROC"),
                "{spelling:?}"
            );
        }
    }

    #[test]
    fn only_a_name_without_a_value_passes_hermits_own_variable_through() {
        let through = |list: &[&str]| -> Vec<String> {
            names_passed_through(&args(list)).into_iter().collect()
        };
        assert_eq!(
            through(&["--env=A", "--env=B=1", "-e", "C", "-eD=2"]),
            ["A", "C"]
        );
        assert_eq!(through(&["-ueX"]), ["X"]);
        assert!(through(&["--env=X=1", "-e", "Y=2"]).is_empty());
    }

    #[test]
    fn other_arguments_name_nothing() {
        assert!(names(&["--base-env=minimal", "--epoch=1", "--tmp=/x"]).is_empty());
        assert_eq!(names(&["--env=B", "-e", "A", "--env=B"]), ["A", "B"]);
    }

    #[test]
    fn a_run_config_or_image_is_found_in_either_spelling() {
        for flag in ["--config", "--from"] {
            assert!(uses_run_config(&args(&[flag, "f"])), "{flag}");
            assert!(uses_run_config(&args(&[&format!("{flag}=f")])), "{flag}");
        }
        assert!(uses_image(&args(&["--image", "x"])));
        assert!(uses_image(&args(&["--image=x"])));
        assert!(!uses_image(&args(&["--image-extra"])));
        assert!(!uses_run_config(&args(&["--configure"])));
    }

    #[test]
    fn pass_through_is_active_only_for_a_self_contained_base_without_a_file_or_image() {
        let active = |list: &[&str], pass_env: bool| pass_through_active(&args(list), pass_env);
        assert!(active(&["--base-env=minimal"], true));
        assert!(active(&["--base-env", "empty"], true));
        assert!(active(&["--base-env=MINIMAL"], true));
        assert!(!active(&["--base-env=minimal"], false));
        assert!(!active(&[], true));
        assert!(!active(&["--base-env=host"], true));
        assert!(!active(&["--base-env=surprise"], true));
        assert!(!active(&["--base-env=minimal", "--config=f"], true));
        assert!(!active(&["--base-env=minimal", "--from", "f"], true));
        assert!(!active(&["--base-env=minimal", "--image=x"], true));
    }

    #[test]
    fn hermit_name_grammar() {
        assert!(is_passable_name("GOMAXPROCS"));
        assert!(is_passable_name("a-b.c"));
        assert!(!is_passable_name(""));
        assert!(!is_passable_name("A=B"));
        assert!(!is_passable_name("café"));
        assert!(!is_passable_name("A\u{1}B"));
        assert!(!is_passable_name("A\u{7f}"));
        for control in ["A\nB", "A\rB", "A\u{7}B", "A\u{1b}B", "A\tB", "\n"] {
            assert!(!is_passable_name(control), "{control:?}");
        }
        assert!(is_passable_name("A B"));
    }
}

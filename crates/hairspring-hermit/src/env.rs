//! The environment a guest program sees. Hermit passes the host environment through, and the host
//! filesystem and environment are *inputs* to a run, so we start from nothing and pin everything.
use crate::events::{EVENTS_FILE_ENV, GUEST_EVENTS_FILE};
use hairspring_engine::hermit_args::{names_passed, pass_through_active};
use std::collections::BTreeMap;

/// The variable an SDK reads for its process tag, and the fixed tag the backend gives the root process.
pub(crate) const ROOT_PROC_ENV: &str = "HAIRSPRING_PROC";
pub(crate) const ROOT_PROC_TAG: &str = "p1";
/// Set by an SDK for the processes it forks; inheriting it would turn the root into `p1.s<pid>`.
pub(crate) const PROC_PID_ENV: &str = "HAIRSPRING_PROC_PID";
/// The trial's seed, an input like any other; tools such as hairspring-dapr-sim derive their faults from it.
pub(crate) const SEED_ENV: &str = "HAIRSPRING_SEED";

const FIXED_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
/// A directory that does not exist: Python finds no bytecode cache and, with writing disabled, never creates one.
const EMPTY_PYCACHE: &str = "/hairspring-empty-pycache";

fn is_python(command: &[String]) -> bool {
    command
        .first()
        .and_then(|c| c.rsplit('/').next())
        .is_some_and(|name| name.starts_with("python"))
}

/// Minimal pinned environment plus the target's configured variables (which win).
pub fn guest_env(
    command: &[String],
    configured: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        ("PATH".to_string(), FIXED_PATH.to_string()),
        ("LANG".to_string(), "C".to_string()),
        ("LC_ALL".to_string(), "C".to_string()),
        ("TZ".to_string(), "UTC".to_string()),
    ]);
    if is_python(command) {
        // Native Python runs rewrite `.pyc` files, which changes what a sandboxed Python executes
        // (measured: the first run after a native run differed). Freeze that state.
        env.insert("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string());
        env.insert("PYTHONPYCACHEPREFIX".to_string(), EMPTY_PYCACHE.to_string());
        env.insert("PYTHONHASHSEED".to_string(), "0".to_string());
    }
    env.extend(configured.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

/// Every variable the engine itself gives the guest for one trial: the pinned environment and the target's `env`
/// ([`guest_env`]), the seed, the events file when it is on, and the root process tag. The process-id variable is never
/// among them. This one map is both what is set on Hermit's process and where the `--env=NAME` names come from, so the
/// names and the values cannot disagree.
pub fn engine_guest_env(
    command: &[String],
    configured: &BTreeMap<String, String>,
    seed: u64,
    events_on: bool,
) -> BTreeMap<String, String> {
    let mut env = guest_env(command, configured);
    env.insert(SEED_ENV.to_string(), seed.to_string());
    if events_on {
        env.insert(EVENTS_FILE_ENV.to_string(), GUEST_EVENTS_FILE.to_string());
    }
    env.insert(ROOT_PROC_ENV.to_string(), ROOT_PROC_TAG.to_string());
    env.remove(PROC_PID_ENV);
    env
}

/// The `--env=NAME` arguments that make Hermit hand `guest` to the program: one per name, in byte order, none when
/// Hairspring is not passing variables (see [`pass_through_active`]) and none for a name the target already passes with
/// its own `--env`, whose value then stays as the target set it. Names only: the values are read by Hermit from its own
/// environment, so they never appear on a command line.
pub fn pass_through_args(
    hermit_args: &[String],
    pass_env: bool,
    guest: &BTreeMap<String, String>,
) -> Vec<String> {
    if !pass_through_active(hermit_args, pass_env) {
        return Vec::new();
    }
    let own = names_passed(hermit_args);
    guest
        .keys()
        .filter(|name| !own.contains(*name))
        .map(|name| format!("--env={name}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str) -> Vec<String> {
        vec![program.to_string()]
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn names(list: &[&str], program: &str, events: bool, env: &[(&str, &str)]) -> Vec<String> {
        let configured = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let guest = engine_guest_env(&command(program), &configured, 7, events);
        pass_through_args(&args(list), true, &guest)
    }

    const MINIMAL: &[&str] = &["--base-env=minimal"];

    #[test]
    fn the_base_list_is_exact_and_in_byte_order() {
        assert_eq!(
            names(MINIMAL, "./svc", false, &[]),
            [
                "--env=HAIRSPRING_PROC",
                "--env=HAIRSPRING_SEED",
                "--env=LANG",
                "--env=LC_ALL",
                "--env=PATH",
                "--env=TZ",
            ]
        );
    }

    #[test]
    fn a_python_command_adds_its_three_pins() {
        let got = names(MINIMAL, "/usr/bin/python3", false, &[]);
        for pin in [
            "PYTHONDONTWRITEBYTECODE",
            "PYTHONHASHSEED",
            "PYTHONPYCACHEPREFIX",
        ] {
            assert!(got.contains(&format!("--env={pin}")), "{pin}: {got:?}");
        }
        assert_eq!(got.len(), 9);
    }

    #[test]
    fn the_events_file_adds_one_name_only_when_it_is_on() {
        let on = names(MINIMAL, "./svc", true, &[]);
        assert!(on.contains(&"--env=HAIRSPRING_EVENTS_FILE".to_string()));
        assert_eq!(on.len(), 7);
        assert_eq!(names(MINIMAL, "./svc", false, &[]).len(), 6);
    }

    #[test]
    fn every_key_of_the_targets_env_is_added_once() {
        let got = names(
            MINIMAL,
            "./svc",
            false,
            &[("GOMAXPROCS", "2"), ("LANG", "en"), ("TMPDIR", "/x")],
        );
        assert_eq!(got.iter().filter(|n| *n == "--env=LANG").count(), 1);
        assert!(got.contains(&"--env=GOMAXPROCS".to_string()));
        assert!(got.contains(&"--env=TMPDIR".to_string()));
        let mut sorted = got.clone();
        sorted.sort();
        assert_eq!(got, sorted);
    }

    #[test]
    fn never_the_pid_variable_nor_the_engines_temp_variables() {
        let configured = BTreeMap::from([("HAIRSPRING_PROC_PID".to_string(), "9".to_string())]);
        let guest = engine_guest_env(&command("./svc"), &configured, 1, true);
        let got = pass_through_args(&args(MINIMAL), true, &guest);
        for banned in ["HAIRSPRING_PROC_PID", "HERMIT_VERIFY_TMPDIR", "TMPDIR"] {
            assert!(
                !got.iter().any(|a| a == &format!("--env={banned}")),
                "{banned}"
            );
        }
    }

    #[test]
    fn a_name_the_target_passes_itself_is_skipped_in_every_spelling() {
        for own in [
            &["--env=LANG=en"][..],
            &["--env", "LANG"][..],
            &["-e", "LANG=en"][..],
            &["-eLANG"][..],
            &["-e=LANG"][..],
        ] {
            let mut list = vec!["--base-env=minimal"];
            list.extend(own);
            let got = names(&list, "./svc", false, &[]);
            assert!(!got.contains(&"--env=LANG".to_string()), "{own:?}: {got:?}");
            assert_eq!(got.len(), 5, "{own:?}");
        }
    }

    #[test]
    fn nothing_is_added_when_passing_is_off_or_not_applicable() {
        let guest = engine_guest_env(&command("./svc"), &BTreeMap::new(), 1, true);
        let none = |list: &[&str], pass_env: bool| {
            pass_through_args(&args(list), pass_env, &guest).is_empty()
        };
        assert!(none(MINIMAL, false));
        assert!(none(&[], true));
        assert!(none(&["--base-env=host"], true));
        assert!(none(&["--base-env=surprise"], true));
        assert!(none(&["--base-env=minimal", "--config=f"], true));
        assert!(none(&["--base-env=minimal", "--from", "f"], true));
        assert!(none(&["--base-env=minimal", "--image=x"], true));
        assert!(!none(&["--base-env=Empty"], true));
    }

    #[test]
    fn the_seed_is_in_the_map_but_the_target_cannot_change_it() {
        let configured = BTreeMap::from([("HAIRSPRING_SEED".to_string(), "99".to_string())]);
        let guest = engine_guest_env(&command("./svc"), &configured, 7, false);
        assert_eq!(guest["HAIRSPRING_SEED"], "7");
        assert_eq!(guest["HAIRSPRING_PROC"], "p1");
    }
}

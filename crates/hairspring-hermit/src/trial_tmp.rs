//! Where Hermit puts its own temporary files for one trial.
//!
//! Hermit keeps a private copy of the host `/etc/group` in a file made in the temporary directory (`$TMPDIR`, else
//! `/tmp`), and removes it when it exits. A `SIGKILL` at the trial timeout skips that, so the file stays. Giving the
//! Hermit process a `TMPDIR` inside the trial's engine-owned directory puts the file where the engine removes it.
//!
//! The catch is that `TMPDIR` must not reach the guest. Hermit's default base environment (`host`) passes its own
//! environment through to the guest, so a `TMPDIR` set for Hermit would be the guest's: a host path that differs from
//! trial to trial. The guest starts without a `TMPDIR` today, and giving it one would change its environment, so the
//! variable is set only when the base environment is one Hermit builds without looking at its own.
use crate::host_identity::base_env_mode;
use crate::summary_dir::ScratchDir;
use std::collections::BTreeMap;
use std::path::PathBuf;

const TMPDIR: &str = "TMPDIR";
/// Base environments Hermit builds from nothing but its own flags (`minimal`, `empty`).
const SELF_CONTAINED_BASE_ENVS: [&str; 2] = ["minimal", "empty"];

/// Whether an argument passes `TMPDIR` from Hermit's environment to the guest: `--env TMPDIR`, `-e TMPDIR`, and the
/// joined forms.
fn passes_tmpdir_through(args: &[String]) -> bool {
    args.iter().enumerate().any(|(i, arg)| {
        let joined = ["--env=", "-e=", "-e"]
            .iter()
            .find_map(|prefix| arg.strip_prefix(prefix));
        let separate = matches!(arg.as_str(), "--env" | "-e")
            .then(|| args.get(i + 1).map(String::as_str))
            .flatten();
        joined == Some(TMPDIR) || separate == Some(TMPDIR)
    })
}

/// Hermit can read a run config file that sets the base environment and the variables passed on; none of that can be
/// read here.
fn reads_a_config_file(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--config" || a.starts_with("--config="))
}

/// The `TMPDIR` to start Hermit with for this trial, or `None` when setting one could reach the guest (or the target
/// has already chosen its own).
pub(crate) fn hermit_tmpdir(
    hermit_args: &[String],
    guest_env: &BTreeMap<String, String>,
    extra_env: &[(String, String)],
    scratch: Option<&ScratchDir>,
) -> Option<PathBuf> {
    let scratch = scratch?;
    let self_contained = SELF_CONTAINED_BASE_ENVS.contains(&base_env_mode(hermit_args).as_str());
    let chosen = guest_env.contains_key(TMPDIR) || extra_env.iter().any(|(name, _)| name == TMPDIR);
    (self_contained
        && !chosen
        && !passes_tmpdir_through(hermit_args)
        && !reads_a_config_file(hermit_args))
    .then(|| scratch.hermit_tmp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary_dir::PrivateDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn scratch() -> (PrivateDir, ScratchDir) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("hairspring-tt-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let private = PrivateDir::create(&base).unwrap();
        let dir = private.scratch_for().unwrap();
        (private, dir)
    }

    fn chosen(list: &[&str]) -> bool {
        let (_keep, dir) = scratch();
        hermit_tmpdir(&args(list), &BTreeMap::new(), &[], Some(&dir)).is_some()
    }

    #[test]
    fn only_a_base_environment_hermit_builds_itself_gets_a_tmpdir() {
        assert!(chosen(&["--base-env=minimal"]));
        assert!(chosen(&["--base-env", "empty"]));
        assert!(!chosen(&[]), "the default passes the environment through");
        assert!(!chosen(&["--base-env=host"]));
        assert!(!chosen(&["--base-env=surprise"]));
    }

    #[test]
    fn a_tmpdir_that_hermit_would_pass_to_the_guest_is_never_set() {
        for pass in [
            &["--env=TMPDIR"][..],
            &["--env", "TMPDIR"][..],
            &["-e", "TMPDIR"][..],
            &["-eTMPDIR"][..],
        ] {
            let mut list = vec!["--base-env=minimal"];
            list.extend(pass);
            assert!(!chosen(&list), "{pass:?}");
        }
        assert!(chosen(&["--base-env=minimal", "--env=TMPDIR_NOT=1"]));
        assert!(chosen(&["--base-env=minimal", "--env=A=TMPDIR"]));
    }

    #[test]
    fn a_run_config_file_makes_it_unknowable() {
        assert!(!chosen(&["--base-env=minimal", "--config=run.yaml"]));
        assert!(!chosen(&["--base-env=minimal", "--config", "run.yaml"]));
    }

    #[test]
    fn a_tmpdir_the_target_configured_is_kept() {
        let (_keep, dir) = scratch();
        let a = args(&["--base-env=minimal"]);
        let env = BTreeMap::from([("TMPDIR".to_string(), "/x".to_string())]);
        assert!(hermit_tmpdir(&a, &env, &[], Some(&dir)).is_none());
        let extra = [("TMPDIR".to_string(), "/x".to_string())];
        assert!(hermit_tmpdir(&a, &BTreeMap::new(), &extra, Some(&dir)).is_none());
    }

    #[test]
    fn without_a_scratch_directory_there_is_none() {
        let a = args(&["--base-env=minimal"]);
        assert!(hermit_tmpdir(&a, &BTreeMap::new(), &[], None).is_none());
    }

    #[test]
    fn the_directory_chosen_is_the_hermit_one_not_the_guests() {
        let (_keep, dir) = scratch();
        let a = args(&["--base-env=minimal"]);
        let chosen = hermit_tmpdir(&a, &BTreeMap::new(), &[], Some(&dir)).unwrap();
        assert_eq!(chosen, dir.hermit_tmp());
        assert_ne!(chosen, dir.guest_tmp());
    }
}

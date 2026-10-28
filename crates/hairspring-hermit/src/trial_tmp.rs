//! Where Hermit puts its own temporary files for one trial.
//!
//! Hermit makes temporary entries in the host temporary directory (`$TMPDIR`, else `/tmp`) and removes them when it
//! exits: its private guest `/tmp` (when it is not given `--tmp`), a private copy of the host `/etc/group`, an empty
//! directory over the nscd socket directory, and, for a strict verify, the two run logs. A `SIGKILL` at the trial
//! timeout skips that, so they stay. Pointing Hermit's temporary directory into the trial's engine-owned directory
//! puts them where the engine removes them. No `--tmp` is passed for this: Hermit keeps making the guest's `/tmp`
//! itself, so the guest sees what it always saw.
//!
//! Two variables do this, and they differ in what reaches the guest:
//!
//! - `TMPDIR` is read from Hermit's environment for everything but the verify logs. Hermit's default base environment
//!   (`host`) hands its environment to the guest, so a `TMPDIR` there would be the guest's: a host path that changes
//!   from trial to trial. It is set only for `--base-env=minimal` and `empty`, which build the guest's environment
//!   without looking at Hermit's own.
//! - `HERMIT_VERIFY_TMPDIR` is Hermit's own setting for the verify logs, which Hermit removes from the guest's
//!   environment under `host` and which `minimal` and `empty` never pass on. It is set in every mode.
//!
//! So under the default base environment the private guest `/tmp` and the `/etc/group` copy can still be left behind
//! by a timeout. Either variable is left alone when the target sets it, passes it through to the guest, or names a
//! Hermit run config file (whose contents cannot be read here).
use crate::host_identity::base_env_mode;
use crate::summary_dir::ScratchDir;
use std::collections::BTreeMap;
use std::path::PathBuf;

const TMPDIR: &str = "TMPDIR";
const VERIFY_TMPDIR: &str = "HERMIT_VERIFY_TMPDIR";
/// Base environments Hermit builds from nothing but its own flags.
const SELF_CONTAINED_BASE_ENVS: [&str; 2] = ["minimal", "empty"];
/// Base environments whose guest environment is known: those above, and the one that passes Hermit's through.
const KNOWN_BASE_ENVS: [&str; 4] = ["minimal", "empty", "host", "default"];

/// Whether an argument passes variable `name` from Hermit's environment to the guest: `--env NAME`, `-e NAME`, and the
/// joined forms.
fn passes_through(args: &[String], name: &str) -> bool {
    args.iter().enumerate().any(|(i, arg)| {
        let joined = ["--env=", "-e=", "-e"]
            .iter()
            .find_map(|prefix| arg.strip_prefix(prefix));
        let separate = matches!(arg.as_str(), "--env" | "-e")
            .then(|| args.get(i + 1).map(String::as_str))
            .flatten();
        joined == Some(name) || separate == Some(name)
    })
}

/// Hermit can read a run config file that sets the base environment and the variables passed on; none of that can be
/// read here.
fn reads_a_config_file(args: &[String]) -> bool {
    args.iter()
        .any(|a| a == "--config" || a.starts_with("--config="))
}

/// The variables to start Hermit with so that its temporary files go into `dir`.
pub(crate) fn hermit_temp_env(
    hermit_args: &[String],
    guest_env: &BTreeMap<String, String>,
    extra_env: &[(String, String)],
    dir: &ScratchDir,
) -> Vec<(&'static str, PathBuf)> {
    if reads_a_config_file(hermit_args) {
        return Vec::new();
    }
    let mode = base_env_mode(hermit_args);
    let free = |name: &str| {
        !guest_env.contains_key(name)
            && !extra_env.iter().any(|(n, _)| n == name)
            && !passes_through(hermit_args, name)
    };
    let mut vars = Vec::new();
    if SELF_CONTAINED_BASE_ENVS.contains(&mode.as_str()) && free(TMPDIR) {
        vars.push((TMPDIR, dir.path().to_path_buf()));
    }
    if KNOWN_BASE_ENVS.contains(&mode.as_str()) && free(VERIFY_TMPDIR) {
        vars.push((VERIFY_TMPDIR, dir.path().to_path_buf()));
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summary_dir::PrivateDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    fn names(list: &[&str]) -> Vec<&'static str> {
        names_with(list, &BTreeMap::new(), &[])
    }

    fn names_with(
        list: &[&str],
        env: &BTreeMap<String, String>,
        extra: &[(String, String)],
    ) -> Vec<&'static str> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("hairspring-tt-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let private = PrivateDir::create(&base).unwrap();
        let dir = private.scratch_for().unwrap();
        let vars = hermit_temp_env(&args(list), env, extra, &dir);
        assert!(vars.iter().all(|(_, path)| path == dir.path()));
        vars.into_iter().map(|(name, _)| name).collect()
    }

    const BOTH: [&str; 2] = [TMPDIR, VERIFY_TMPDIR];

    #[test]
    fn a_self_contained_base_environment_gets_both_variables() {
        assert_eq!(names(&["--base-env=minimal"]), BOTH);
        assert_eq!(names(&["--base-env", "empty"]), BOTH);
    }

    #[test]
    fn the_host_base_environment_gets_only_the_one_hermit_strips() {
        assert_eq!(names(&[]), [VERIFY_TMPDIR]);
        assert_eq!(names(&["--base-env=host"]), [VERIFY_TMPDIR]);
        assert_eq!(names(&["--base-env", "host"]), [VERIFY_TMPDIR]);
    }

    #[test]
    fn an_unknown_base_environment_gets_nothing() {
        assert!(names(&["--base-env=surprise"]).is_empty());
    }

    #[test]
    fn a_variable_hermit_would_pass_to_the_guest_is_never_set() {
        for pass in [
            &["--env=TMPDIR"][..],
            &["--env", "TMPDIR"][..],
            &["-e", "TMPDIR"][..],
            &["-eTMPDIR"][..],
        ] {
            let mut list = vec!["--base-env=minimal"];
            list.extend(pass);
            assert_eq!(names(&list), [VERIFY_TMPDIR], "{pass:?}");
        }
        assert_eq!(
            names(&["--base-env=minimal", "--env=HERMIT_VERIFY_TMPDIR"]),
            [TMPDIR]
        );
        assert_eq!(names(&["--base-env=minimal", "--env=TMPDIR_NOT=1"]), BOTH);
        assert_eq!(names(&["--base-env=minimal", "--env=A=TMPDIR"]), BOTH);
    }

    #[test]
    fn a_run_config_file_makes_it_unknowable() {
        assert!(names(&["--base-env=minimal", "--config=run.yaml"]).is_empty());
        assert!(names(&["--base-env=minimal", "--config", "run.yaml"]).is_empty());
    }

    #[test]
    fn a_variable_the_target_configured_is_kept() {
        let env = BTreeMap::from([("TMPDIR".to_string(), "/x".to_string())]);
        assert_eq!(
            names_with(&["--base-env=minimal"], &env, &[]),
            [VERIFY_TMPDIR]
        );
        let extra = [("HERMIT_VERIFY_TMPDIR".to_string(), "/x".to_string())];
        assert_eq!(
            names_with(&["--base-env=minimal"], &BTreeMap::new(), &extra),
            [TMPDIR]
        );
    }
}

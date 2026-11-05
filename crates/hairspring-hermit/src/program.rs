//! Whether a process target's program exists. A program that is not there exits 127 on every seed, which would look
//! like a reproduced failure; both checks here make it a configuration error instead.
//!
//! Before any trial runs, the program is looked up on the host as Hermit starts it by default: in the target's `cwd`
//! with the guest's environment, so a name with a `/` is relative to `cwd` and a bare name is searched on the guest's
//! `PATH`. Some `hermit_args` change what the guest sees (its mounts, its root, its environment, its working
//! directory); with one of those the host lookup proves nothing, so a miss is only a warning and Hermit decides. The
//! check runs wherever the backend runs, so a remote worker checks its own machine.
//!
//! After a trial, Hermit's own refusal to start the program (exit 127 with its "does not exist" or "Could not resolve
//! program" message) is recognised, so the case the host lookup skipped is still a configuration error. A program
//! that exits 127 itself, without that message, is judged like any other exit status.
use crate::env::guest_env;
use crate::exec::{ProcessSpec, Trial};
use hairspring_engine::EngineError;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Hermit flags that change what the guest's filesystem, environment or working directory is, so that a lookup on the
/// host does not show what the guest will find: `--mount`, `--bind` and `--tmp` add or replace guest paths, `--image`
/// gives the guest another root, `--env`/`-e` and `--base-env` change its `PATH`, `--workdir` its working directory,
/// and `--config`/`--from` can carry any of these. Taken from `hermit run`'s options at the pinned commit.
const GUEST_VIEW_FLAGS: [&str; 10] = [
    "--mount",
    "--bind",
    "--tmp",
    "--image",
    "--env",
    "--base-env",
    "--workdir",
    "--config",
    "--from",
    "-e",
];

/// The exit status Hermit gives a program it cannot start, as GNU `env` does.
const HERMIT_PROGRAM_NOT_FOUND_EXIT: i32 = 127;
/// How Hermit says it could not start the program: a path that is not there, and a bare name not on the guest `PATH`.
const HERMIT_NOT_FOUND_MESSAGES: [(&str, &str); 2] = [
    ("Program ", " does not exist or is not accessible"),
    ("Could not resolve program ", " in the guest PATH"),
];

/// Targets already warned about in this process, so a run that looks the program up several times warns once.
static WARNED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Whether `hermit_args` contain a flag that changes what the guest sees (see [`GUEST_VIEW_FLAGS`]).
fn changes_guest_view(hermit_args: &[String]) -> bool {
    hermit_args.iter().any(|arg| {
        let name = arg.split('=').next().unwrap_or(arg);
        // `-e NAME=V` and `-eNAME=V` are both the short form of `--env`.
        GUEST_VIEW_FLAGS.contains(&name) || (arg.starts_with("-e") && !arg.starts_with("--"))
    })
}

/// The line of Hermit's standard error that says it could not start the program, when the trial ended with Hermit's
/// "program not found" status.
pub(crate) fn hermit_refused_program(t: &Trial) -> Option<String> {
    if t.exit_code != Some(HERMIT_PROGRAM_NOT_FOUND_EXIT) {
        return None;
    }
    t.stderr
        .lines()
        .find(|line| {
            HERMIT_NOT_FOUND_MESSAGES.iter().any(|(start, rest)| {
                line.find(start)
                    .is_some_and(|at| line[at + start.len()..].contains(rest))
            })
        })
        .map(|line| line.trim().to_string())
}

/// The configuration error for a trial whose program Hermit could not start.
pub(crate) fn refused_program_message(p: &ProcessSpec, hermit_line: &str) -> String {
    let program = p.command.first().map_or("", String::as_str);
    format!(
        "target {}: program {program} not found or not executable (Hermit: {hermit_line})",
        p.target
    )
}

/// The program `command[0]` names, resolved as Hermit will resolve it, or `None` when no executable file is there.
pub(crate) fn resolve(program: &str, cwd: Option<&Path>, path_var: &str) -> Option<PathBuf> {
    let base = |p: &Path| cwd.map_or_else(|| p.to_path_buf(), |dir| dir.join(p));
    if program.contains('/') {
        return Some(base(Path::new(program))).filter(|p| is_executable_file(p));
    }
    path_var
        .split(':')
        // POSIX: an empty PATH entry means the current directory.
        .map(|dir| if dir.is_empty() { "." } else { dir })
        .map(|dir| base(Path::new(dir)).join(program))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// What the host lookup found for a target's program.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HostLookup {
    /// Found, or nothing to look up (an empty command is the config validation's to refuse).
    Found,
    /// Not found, and the default guest view applies: a configuration error.
    Missing(String),
    /// Not found on the host, but `hermit_args` change the guest's view, so Hermit decides.
    Unprovable(String),
}

pub(crate) fn lookup(p: &ProcessSpec) -> HostLookup {
    let Some(program) = p.command.first() else {
        return HostLookup::Found;
    };
    let env = guest_env(p.command, p.env);
    let path_var = env.get("PATH").map_or("", String::as_str);
    let cwd = p.cwd.map(Path::new);
    if resolve(program, cwd, path_var).is_some() {
        return HostLookup::Found;
    }
    let target = p.target;
    if changes_guest_view(p.hermit_args) {
        return HostLookup::Unprovable(format!(
            "target {target}: could not find {program} on the host; Hermit's arguments change what the guest sees, \
             so continuing"
        ));
    }
    let looked = match (program.contains('/'), p.cwd) {
        (false, _) => format!(" (looked on the guest PATH {path_var})"),
        (true, Some(dir)) if !program.starts_with('/') => format!(" (relative to cwd {dir})"),
        (true, _) => String::new(),
    };
    HostLookup::Missing(format!(
        "target {target}: program {program} not found or not executable{looked}"
    ))
}

/// `Ok` when the target's program can be started, or when only Hermit can tell (warned about once per target on
/// standard error); otherwise the configuration error that names it.
pub(crate) fn check(p: &ProcessSpec) -> Result<(), EngineError> {
    match lookup(p) {
        HostLookup::Found => Ok(()),
        HostLookup::Missing(message) => Err(EngineError::Backend(message)),
        HostLookup::Unprovable(message) => {
            let first = WARNED
                .lock()
                .map_or(true, |mut warned| warned.insert(p.target.to_string()));
            if first {
                eprintln!("hairspring: warning: {message}");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hairspring-program-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn an_absolute_program_must_be_an_executable_file() {
        let dir = scratch("abs");
        let exe = file(&dir, "svc", 0o755);
        let plain = file(&dir, "data", 0o644);
        let path = exe.to_str().unwrap();
        assert_eq!(resolve(path, None, ""), Some(exe.clone()));
        // cwd does not change an absolute path.
        assert_eq!(resolve(path, Some(Path::new("/elsewhere")), ""), Some(exe));
        assert_eq!(resolve(plain.to_str().unwrap(), None, ""), None);
        assert_eq!(
            resolve(dir.join("absent").to_str().unwrap(), None, ""),
            None
        );
        assert_eq!(
            resolve(dir.to_str().unwrap(), None, ""),
            None,
            "a directory"
        );
    }

    #[test]
    fn a_relative_program_is_found_from_the_targets_cwd() {
        let dir = scratch("rel");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let exe = file(&dir.join("bin"), "svc", 0o755);
        assert_eq!(
            resolve("./bin/svc", Some(&dir), ""),
            Some(dir.join("./bin/svc"))
        );
        assert!(exe.exists());
        assert_eq!(resolve("./bin/other", Some(&dir), ""), None);
    }

    #[test]
    fn a_bare_name_is_looked_up_on_the_guest_path_only() {
        let a = scratch("path-a");
        let b = scratch("path-b");
        let exe = file(&b, "svc", 0o755);
        file(&a, "svc", 0o644);
        let path_var = format!("{}:{}", a.display(), b.display());
        assert_eq!(resolve("svc", None, &path_var), Some(exe));
        assert_eq!(resolve("svc", None, &a.display().to_string()), None);
        assert_eq!(resolve("svc", None, ""), None);
    }

    #[test]
    fn relative_and_empty_path_entries_are_relative_to_the_cwd() {
        let dir = scratch("path-rel");
        std::fs::create_dir_all(dir.join("tools")).unwrap();
        file(&dir.join("tools"), "svc", 0o755);
        file(&dir, "here", 0o755);
        assert!(resolve("svc", Some(&dir), "tools").is_some());
        assert!(resolve("here", Some(&dir), "/nonexistent-dir:").is_some());
    }

    fn spec<'a>(
        command: &'a [String],
        env: &'a std::collections::BTreeMap<String, String>,
        cwd: Option<&'a str>,
        scheduler: &'a hairspring_engine::scheduler::SchedulerSpec,
    ) -> ProcessSpec<'a> {
        ProcessSpec {
            target: "svc",
            command,
            env,
            cwd,
            hermit_args: &[],
            trial_timeout_secs: None,
            scheduler,
            hang_is_violation: false,
            parallelism: 1,
        }
    }

    #[test]
    fn a_missing_program_is_a_configuration_error_naming_the_target_and_the_program() {
        let env = std::collections::BTreeMap::new();
        let scheduler = Default::default();
        let command = vec!["/definitely/not/a/program".to_string()];
        let err = check(&spec(&command, &env, None, &scheduler))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(
                "target svc: program /definitely/not/a/program not found or not executable"
            ),
            "{err}"
        );
        let bare = vec!["definitely-not-a-program-hs".to_string()];
        let err = check(&spec(&bare, &env, None, &scheduler))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("guest PATH /usr/local/bin:/usr/bin:/bin"),
            "{err}"
        );
    }

    #[test]
    fn a_program_on_the_fixed_guest_path_or_a_configured_path_passes() {
        let scheduler = Default::default();
        let env = std::collections::BTreeMap::new();
        let sh = vec!["sh".to_string(), "-c".to_string(), "true".to_string()];
        assert!(check(&spec(&sh, &env, None, &scheduler)).is_ok());
        let dir = scratch("configured-path");
        file(&dir, "svc-only-here", 0o755);
        let configured =
            std::collections::BTreeMap::from([("PATH".to_string(), dir.display().to_string())]);
        let own = vec!["svc-only-here".to_string()];
        assert!(check(&spec(&own, &configured, None, &scheduler)).is_ok());
        assert!(check(&spec(&own, &env, None, &scheduler)).is_err());
    }

    #[test]
    fn an_empty_command_is_left_to_config_validation() {
        let scheduler = Default::default();
        let env = std::collections::BTreeMap::new();
        assert!(check(&spec(&[], &env, None, &scheduler)).is_ok());
    }

    fn with_args<'a>(p: ProcessSpec<'a>, hermit_args: &'a [String]) -> ProcessSpec<'a> {
        ProcessSpec { hermit_args, ..p }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn hermit_args_that_change_the_guest_view_turn_a_miss_into_a_warning() {
        let scheduler = Default::default();
        let env = std::collections::BTreeMap::new();
        // The cases a review checked against the pinned Hermit: a PATH given to Hermit, Hermit's minimal base
        // environment (whose PATH has /usr/sbin and /sbin), and a bind mount that puts the program in place.
        let cases: [(&[&str], &str); 5] = [
            (&["--env=PATH=/opt/tools"], "mytool-hs-absent"),
            (&["--base-env=minimal"], "chown-hs-absent"),
            (
                &["--mount=type=bind,source=/srv/app,target=/opt/app"],
                "/opt/app/svc-hs-absent",
            ),
            (&["--bind", "/srv/app:/tmp/app"], "/tmp/app/svc-hs-absent"),
            (&["-ePATH=/opt/tools"], "mytool-hs-absent"),
        ];
        for (args, program) in cases {
            let args = words(args);
            let command = vec![program.to_string()];
            let p = with_args(spec(&command, &env, None, &scheduler), &args);
            match lookup(&p) {
                HostLookup::Unprovable(message) => {
                    assert!(
                        message.contains("Hermit's arguments change what the guest sees"),
                        "{message}"
                    );
                }
                other => panic!("{args:?}: {other:?}"),
            }
            assert!(check(&p).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn hermit_args_that_leave_the_guest_view_alone_keep_the_error() {
        let scheduler = Default::default();
        let env = std::collections::BTreeMap::new();
        let args = words(&["--verbose", "--summary", "--preemption-timeout=1000"]);
        let command = vec!["/definitely/not/a/program".to_string()];
        let p = with_args(spec(&command, &env, None, &scheduler), &args);
        assert!(matches!(lookup(&p), HostLookup::Missing(_)));
        assert!(check(&p).is_err());
    }

    fn trial(exit: i32, stderr: &str) -> Trial {
        Trial {
            exit_code: Some(exit),
            stderr: stderr.to_string(),
            ..Trial::default()
        }
    }

    #[test]
    fn hermits_own_refusal_to_start_the_program_is_recognised() {
        let missing = trial(
            127,
            "Error: Program /opt/app/svc does not exist or is not accessible. Check the path and any --mount or --bind target.\n",
        );
        let line = hermit_refused_program(&missing).expect("recognised");
        assert!(line.contains("/opt/app/svc does not exist"), "{line}");
        let unresolved = trial(
            127,
            "Error: Could not resolve program \"mytool\" in the guest PATH. Check PATH or use an absolute executable path.\n",
        );
        assert!(hermit_refused_program(&unresolved).is_some());
        let scheduler = Default::default();
        let env = std::collections::BTreeMap::new();
        let command = vec!["/opt/app/svc".to_string()];
        let message = refused_program_message(&spec(&command, &env, None, &scheduler), &line);
        assert!(
            message.starts_with("target svc: program /opt/app/svc not found or not executable"),
            "{message}"
        );
    }

    #[test]
    fn a_program_that_exits_127_itself_is_not_taken_for_a_missing_one() {
        assert_eq!(
            hermit_refused_program(&trial(127, "sh: frobnicate: command not found\n")),
            None
        );
        assert_eq!(hermit_refused_program(&trial(127, "")), None);
        // Hermit's words with another status are the program's own output.
        let words = "Error: Program /x does not exist or is not accessible.\n";
        assert_eq!(hermit_refused_program(&trial(1, words)), None);
    }
}

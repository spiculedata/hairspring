//! Whether a process target's program exists before any trial runs. Hermit starts in the target's `cwd` with the
//! guest's environment, so the program is looked up the same way: a name with a `/` relative to `cwd`, a bare name on
//! the guest's `PATH`. A program that is not there would otherwise exit 127 on every seed and look like a reproduced
//! failure; this makes it a configuration error. The check runs wherever the backend runs (a remote worker checks its
//! own machine).
use crate::env::guest_env;
use crate::exec::ProcessSpec;
use hairspring_engine::EngineError;
use std::path::{Path, PathBuf};

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

/// `Ok` when the target's program can be started; otherwise the configuration error that names it.
pub(crate) fn check(target: &str, p: &ProcessSpec) -> Result<(), EngineError> {
    let Some(program) = p.command.first() else {
        return Ok(());
    };
    let env = guest_env(p.command, p.env);
    let path_var = env.get("PATH").map_or("", String::as_str);
    let cwd = p.cwd.map(Path::new);
    if resolve(program, cwd, path_var).is_some() {
        return Ok(());
    }
    let looked = match (program.contains('/'), p.cwd) {
        (false, _) => format!(" (looked on the guest PATH {path_var})"),
        (true, Some(dir)) if !program.starts_with('/') => format!(" (relative to cwd {dir})"),
        (true, _) => String::new(),
    };
    Err(EngineError::Backend(format!(
        "target {target}: program {program} not found or not executable{looked}"
    )))
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
        let err = check("svc", &spec(&command, &env, None, &scheduler))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(
                "target svc: program /definitely/not/a/program not found or not executable"
            ),
            "{err}"
        );
        let bare = vec!["definitely-not-a-program-hs".to_string()];
        let err = check("svc", &spec(&bare, &env, None, &scheduler))
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
        assert!(check("svc", &spec(&sh, &env, None, &scheduler)).is_ok());
        let dir = scratch("configured-path");
        file(&dir, "svc-only-here", 0o755);
        let configured =
            std::collections::BTreeMap::from([("PATH".to_string(), dir.display().to_string())]);
        let own = vec!["svc-only-here".to_string()];
        assert!(check("svc", &spec(&own, &configured, None, &scheduler)).is_ok());
        assert!(check("svc", &spec(&own, &env, None, &scheduler)).is_err());
    }

    #[test]
    fn an_empty_command_is_left_to_config_validation() {
        let scheduler = Default::default();
        let env = std::collections::BTreeMap::new();
        assert!(check("svc", &spec(&[], &env, None, &scheduler)).is_ok());
    }
}

//! Starting programs. Every program this backend starts goes through here and through the allow-list in `commands`: a
//! command that is not one of the allowed shapes is refused before anything is spawned. The environment of a started
//! program is cleared (the root commands set their own library path through `env`), nothing is read from standard input,
//! and a VM is started in its own process group.
use crate::bounded::{run_bounded, BoundedError};
use crate::commands::{Approved, Commands};
use crate::error::{InfraError, InfraKind};
use crate::host::{HostConfig, Timings};
use crate::logs::parse_free_kib;
use crate::vmname::VmName;
use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// What a short helper command said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub code: Option<i32>,
    pub stdout: String,
}

/// Runs the allowed commands.
#[derive(Debug, Clone)]
pub struct Spawner {
    commands: Commands,
    timings: Timings,
}

fn infra_of(e: BoundedError) -> InfraError {
    match e {
        BoundedError::Refused(r) => InfraError::new(InfraKind::Refused, r.to_string()),
        BoundedError::TimedOut => InfraError::new(
            InfraKind::StartFailed,
            "a helper command did not answer in time",
        ),
        BoundedError::Start(k) | BoundedError::Wait(k) => {
            InfraError::new(InfraKind::StartFailed, k)
        }
    }
}

impl Spawner {
    pub fn new(host: &HostConfig, timings: Timings) -> Spawner {
        Spawner {
            commands: Commands::new(host),
            timings,
        }
    }

    fn approve(&self, argv: &[String], expected: Approved) -> Result<(), InfraError> {
        match self.commands.check(argv) {
            Ok(got) if got == expected => Ok(()),
            Ok(_) => Err(InfraError::new(
                InfraKind::Refused,
                "not the command that was asked for",
            )),
            Err(r) => Err(InfraError::new(InfraKind::Refused, r.to_string())),
        }
    }

    fn run_helper(&self, argv: &[String], expected: Approved) -> Result<Said, InfraError> {
        self.approve(argv, expected)?;
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| InfraError::bare(InfraKind::Refused))?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let answer = run_bounded(program, &args, self.timings.helper_cap).map_err(infra_of)?;
        Ok(Said {
            code: answer.code,
            stdout: answer.stdout,
        })
    }

    /// Destroys the VM named `name`. Its exit status is returned and not judged: destroying a VM that is not there fails,
    /// which is the usual case before a start.
    pub fn destroy(&self, name: &VmName) -> Result<Option<i32>, InfraError> {
        let argv = self.commands.destroy(name);
        Ok(self.run_helper(&argv, Approved::Destroy)?.code)
    }

    /// Checkpoints the running VM `name` into `dir`.
    pub fn checkpoint(&self, dir: &Path, name: &VmName) -> Result<(), InfraError> {
        let argv = self.commands.checkpoint(dir, name);
        let said = self.run_helper(&argv, Approved::Checkpoint)?;
        match said.code {
            Some(0) => Ok(()),
            Some(n) => Err(InfraError::new(
                InfraKind::Checkpoint,
                format!("the checkpoint command exited {n}"),
            )),
            None => Err(InfraError::new(
                InfraKind::Checkpoint,
                "the checkpoint command was ended by a signal",
            )),
        }
    }

    /// The free space under `dir`, in KiB.
    pub fn free_kib(&self, dir: &Path) -> Result<u64, InfraError> {
        let argv = Commands::free_space(dir);
        let said = self.run_helper(&argv, Approved::FreeSpace)?;
        parse_free_kib(&said.stdout)
            .ok_or_else(|| InfraError::new(InfraKind::LowSpace, "the free space could not be read"))
    }

    /// Starts a VM with `args` (the last of which is its name), its standard error written to `run_log`, in its own
    /// process group.
    pub fn start_vm(&self, args: &[String], run_log: &Path) -> Result<Child, InfraError> {
        let argv = self.commands.bhyve(args);
        self.approve(&argv, Approved::Bhyve)?;
        let log = File::create(run_log).map_err(|e| InfraError::io("create the run log", &e))?;
        let (program, rest) = argv
            .split_first()
            .ok_or_else(|| InfraError::bare(InfraKind::Refused))?;
        Command::new(program)
            .args(rest)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .process_group(0)
            .spawn()
            .map_err(|e| InfraError::new(InfraKind::StartFailed, e.kind().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmname::Stamp;

    fn host() -> HostConfig {
        HostConfig {
            sudo: "/nonexistent/sudo".into(),
            env: "/usr/bin/env".into(),
            bhyve: "/nonexistent/bhyve".into(),
            bhyvectl: "/nonexistent/bhyvectl".into(),
            lib_dir: "/nonexistent/lib".into(),
            state_dir: std::env::temp_dir(),
            kernel: "/nonexistent/k".into(),
            initrd: "/nonexistent/i".into(),
        }
    }

    fn spawner() -> Spawner {
        Spawner::new(&host(), Timings::fast())
    }

    #[test]
    fn a_program_that_loads_a_module_is_refused_before_anything_is_spawned() {
        let argv = vec!["/sbin/kldload".to_string(), "vmm".to_string()];
        let err = spawner().run_helper(&argv, Approved::Destroy).unwrap_err();
        assert_eq!(err.kind, InfraKind::Refused);
        assert!(err.detail.contains("kernel modules"), "{err}");
    }

    #[test]
    fn a_command_that_is_not_the_one_asked_for_is_refused() {
        let s = spawner();
        let name = VmName::new(&Stamp::from_counter(1), 0).unwrap();
        let destroy = s.commands.destroy(&name);
        let err = s.run_helper(&destroy, Approved::Checkpoint).unwrap_err();
        assert_eq!(err.kind, InfraKind::Refused);
    }

    #[test]
    fn a_vm_start_with_a_name_outside_the_grammar_is_refused_and_creates_no_log() {
        let log = std::env::temp_dir().join(format!("hs-dhyve-spawn-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        let args = vec!["-c".to_string(), "1".to_string(), "hs_plain".to_string()];
        let err = spawner().start_vm(&args, &log).unwrap_err();
        assert_eq!(err.kind, InfraKind::Refused);
        assert!(!log.exists());
    }

    #[test]
    fn a_program_that_cannot_be_started_is_a_start_failure() {
        let name = VmName::new(&Stamp::from_counter(1), 0).unwrap();
        let err = spawner().destroy(&name).unwrap_err();
        assert_eq!(err.kind, InfraKind::StartFailed);
    }

    #[test]
    fn free_space_is_read_from_the_real_df() {
        assert!(spawner().free_kib(&std::env::temp_dir()).unwrap() > 0);
    }
}

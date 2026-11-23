//! Asking a process, or the process group a VM was started in, to end. The workspace has no `libc` dependency, so the one
//! system call needed is declared here. A VM runs as root under `sudo` and the backend does not, so the system may refuse
//! (`EPERM`): a refusal is an answer, never an error that stops the cleanup.
use std::process::Child;

const SIGTERM: i32 = 15;

extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

/// What a request to end a process came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sent {
    Delivered,
    Refused,
}

/// How the backend ends the process it started. Tests can stand in a version that always refuses, which is what a root
/// process under `sudo` does to an unprivileged caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Terminator {
    #[default]
    System,
    AlwaysRefused,
}

impl Terminator {
    /// SIGTERM to the process group `child` leads (started with its own group).
    pub fn terminate_group(self, child: &Child) -> Sent {
        if self == Terminator::AlwaysRefused {
            return Sent::Refused;
        }
        let Ok(pid) = i32::try_from(child.id()) else {
            return Sent::Refused;
        };
        // SAFETY: `kill` takes two integers and touches no memory; a negative pid names the group.
        if unsafe { kill(-pid, SIGTERM) } == 0 {
            Sent::Delivered
        } else {
            Sent::Refused
        }
    }

    /// SIGKILL to `child` itself.
    pub fn kill_child(self, child: &mut Child) -> Sent {
        if self == Terminator::AlwaysRefused {
            return Sent::Refused;
        }
        match child.kill() {
            Ok(()) => Sent::Delivered,
            Err(_) => Sent::Refused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn sleeper() -> Child {
        Command::new("/bin/sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap()
    }

    fn reaped_within(child: &mut Child, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_group_signal_ends_a_child_that_leads_its_group() {
        let mut child = sleeper();
        assert_eq!(Terminator::System.terminate_group(&child), Sent::Delivered);
        assert!(reaped_within(&mut child, Duration::from_secs(5)));
    }

    #[test]
    fn killing_the_child_itself_ends_it() {
        let mut child = sleeper();
        assert_eq!(Terminator::System.kill_child(&mut child), Sent::Delivered);
        assert!(reaped_within(&mut child, Duration::from_secs(5)));
    }

    #[test]
    fn a_stand_in_that_refuses_leaves_the_child_running() {
        let mut child = sleeper();
        assert_eq!(
            Terminator::AlwaysRefused.terminate_group(&child),
            Sent::Refused
        );
        assert_eq!(
            Terminator::AlwaysRefused.kill_child(&mut child),
            Sent::Refused
        );
        assert!(!reaped_within(&mut child, Duration::from_millis(100)));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_signal_to_a_group_that_is_gone_is_a_refusal_not_a_panic() {
        let mut child = sleeper();
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(Terminator::System.terminate_group(&child), Sent::Refused);
    }
}

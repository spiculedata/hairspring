//! Runs one read-only host command with a deadline, so a command that does not answer is a failed check and never a
//! hang. The environment is cleared and nothing is read from standard input.
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How long a probe command may take.
#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
pub const COMMAND_DEADLINE: Duration = Duration::from_secs(5);
/// How long to wait between looks at a running command.
const POLL: Duration = Duration::from_millis(10);
/// The most output kept; the commands used here print a line.
const MAX_OUTPUT: u64 = 64 * 1024;

/// Why a command gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedError {
    /// It could not be started (missing file, not executable).
    Start(String),
    /// It was still running at the deadline and was killed.
    TimedOut,
    /// It was started but waiting for it failed.
    Wait(String),
    /// It is not one of the commands this crate runs, so it was not started.
    #[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
    Refused(crate::commands::Refusal),
}

impl std::fmt::Display for BoundedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundedError::Start(why) => write!(f, "it could not be started ({why})"),
            BoundedError::TimedOut => write!(f, "it did not answer in time"),
            BoundedError::Wait(why) => write!(f, "waiting for it failed ({why})"),
            BoundedError::Refused(why) => write!(f, "{why}"),
        }
    }
}

/// What a command that finished said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub success: bool,
    /// The exit status; `None` when the command was ended by a signal.
    pub code: Option<i32>,
    pub stdout: String,
}

/// Runs `argv` for at most `deadline` if it is one of the read-only questions the machine check asks.
#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
pub fn run_read_only(argv: &[String], deadline: Duration) -> Result<Answer, BoundedError> {
    crate::commands::check_read_only(argv).map_err(BoundedError::Refused)?;
    let (program, args) = argv
        .split_first()
        .ok_or(BoundedError::Start("empty".into()))?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run_bounded(program, &args, deadline)
}

/// Runs `program` (an absolute path) with `args` for at most `deadline`.
pub fn run_bounded(
    program: &str,
    args: &[&str],
    deadline: Duration,
) -> Result<Answer, BoundedError> {
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BoundedError::Start(e.kind().to_string()))?;
    let reader = child.stdout.take().map(|out| {
        thread::spawn(move || {
            let mut text = String::new();
            // Output that is not text, or that cannot be read, is an empty answer.
            let _ = out.take(MAX_OUTPUT).read_to_string(&mut text);
            text
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BoundedError::TimedOut);
            }
            Ok(None) => thread::sleep(POLL),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BoundedError::Wait(e.kind().to_string()));
            }
        }
    };
    let stdout = reader.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok(Answer {
        success: status.success(),
        code: status.code(),
        stdout,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn a_command_that_answers_returns_its_output_and_status() {
        let a = run_bounded("/bin/sh", &["-c", "echo hello"], SECOND).unwrap();
        assert!(a.success);
        assert_eq!(a.stdout, "hello\n");
    }

    #[test]
    fn a_failing_command_is_an_answer_not_an_error() {
        let a = run_bounded("/bin/sh", &["-c", "exit 3"], SECOND).unwrap();
        assert!(!a.success);
    }

    #[test]
    fn a_command_that_does_not_answer_is_killed_at_the_deadline() {
        let started = Instant::now();
        let r = run_bounded(
            "/bin/sh",
            &["-c", "exec sleep 30"],
            Duration::from_millis(100),
        );
        assert_eq!(r, Err(BoundedError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_missing_program_is_a_start_error() {
        let r = run_bounded("/no/such/program", &[], SECOND);
        assert!(matches!(r, Err(BoundedError::Start(_))), "{r:?}");
    }

    #[test]
    fn the_environment_is_cleared() {
        std::env::set_var("HS_DHYVE_BOUNDED_TEST", "leak");
        let a = run_bounded(
            "/bin/sh",
            &["-c", "echo ${HS_DHYVE_BOUNDED_TEST-unset}"],
            SECOND,
        )
        .unwrap();
        assert_eq!(a.stdout, "unset\n");
    }

    #[test]
    fn output_beyond_the_cap_is_cut_and_the_command_still_finishes() {
        let a = run_bounded("/bin/sh", &["-c", "yes | head -c 300000"], SECOND).unwrap();
        assert!(a.stdout.len() as u64 <= MAX_OUTPUT);
    }

    #[test]
    fn a_read_only_run_refuses_anything_that_is_not_a_machine_check_question() {
        let r = run_read_only(&["/bin/sh".into(), "-c".into(), "echo hi".into()], SECOND);
        assert!(matches!(r, Err(BoundedError::Refused(_))), "{r:?}");
        let r = run_read_only(&["/sbin/kldload".into(), "vmm".into()], SECOND);
        assert_eq!(
            r,
            Err(BoundedError::Refused(
                crate::commands::Refusal::KernelModule
            ))
        );
        let r = run_read_only(&[], SECOND);
        assert!(matches!(r, Err(BoundedError::Refused(_))));
    }

    #[test]
    fn the_exit_status_is_kept() {
        assert_eq!(
            run_bounded("/bin/sh", &["-c", "exit 3"], SECOND)
                .unwrap()
                .code,
            Some(3)
        );
    }

    #[test]
    fn errors_say_what_happened() {
        assert!(BoundedError::TimedOut.to_string().contains("in time"));
        assert!(BoundedError::Start("x".into())
            .to_string()
            .contains("started"));
        assert!(BoundedError::Wait("x".into())
            .to_string()
            .contains("waiting"));
        assert!(BoundedError::Refused(crate::commands::Refusal::NotAllowed)
            .to_string()
            .contains("refused"));
    }
}

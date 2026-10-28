//! The `hairspring` command line. Exposed as a library so that users with their own
//! `Sim` targets can build a binary with their registry:
//!
//! ```no_run
//! fn main() -> std::process::ExitCode {
//!     hairspring_cli::main_with(hairspring_engine::erased::Registry::new() /* .with(...) */)
//! }
//! ```
mod cli;
mod commands;
mod ctx;
mod host;
mod seed_range;
mod timefmt;

use clap::Parser;
use hairspring_engine::erased::Registry;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

/// Exit codes: 0 success, 1 a property was violated, 2 usage or runtime error, 130 interrupted by Ctrl-C.
pub const EXIT_OK: i32 = 0;
pub const EXIT_VIOLATION: i32 = 1;
pub const EXIT_ERROR: i32 = 2;
/// A replay did not reproduce the failure, but the host differs from the recording, so it does not show the bug is fixed.
pub const EXIT_INCONCLUSIVE: i32 = 3;
/// `doctor --process-backend`: this machine cannot run the process backend. Not a found bug and not a usage error.
pub const EXIT_UNSUPPORTED_HOST: i32 = 4;
/// Ctrl-C ended `test` or `confirm`: 128 plus the number of SIGINT, as shells report it.
pub const EXIT_INTERRUPTED: i32 = 130;

/// The status a command ends with: 130 whenever Ctrl-C was pressed while it ran, even if the work it was doing had
/// finished by then (the user asked it to stop), and `code` otherwise. What was stored stays as it was.
pub(crate) fn status_after_interrupt(code: i32, interrupted: bool) -> i32 {
    if interrupted {
        EXIT_INTERRUPTED
    } else {
        code
    }
}

pub fn main_with(registry: Registry) -> ExitCode {
    let code = run(
        registry,
        std::env::args_os(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    ExitCode::from(code as u8)
}

pub fn run<I, T>(registry: Registry, args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match cli::Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { EXIT_ERROR } else { EXIT_OK };
            let _ = if e.use_stderr() {
                write!(err, "{e}")
            } else {
                write!(out, "{e}")
            };
            return code;
        }
    };
    match commands::dispatch(cli, registry, out, err) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            EXIT_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_that_was_interrupted_exits_130_whatever_its_own_status() {
        for code in [EXIT_OK, EXIT_VIOLATION, EXIT_ERROR, EXIT_INCONCLUSIVE] {
            assert_eq!(status_after_interrupt(code, true), EXIT_INTERRUPTED);
            assert_eq!(status_after_interrupt(code, false), code);
        }
    }
}

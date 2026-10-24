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

/// Exit codes: 0 success, 1 a property was violated, 2 usage or runtime error.
pub const EXIT_OK: i32 = 0;
pub const EXIT_VIOLATION: i32 = 1;
pub const EXIT_ERROR: i32 = 2;
/// A replay did not reproduce the failure, but the host differs from the recording, so it does not show the bug is fixed.
pub const EXIT_INCONCLUSIVE: i32 = 3;
/// `doctor --process-backend`: this machine cannot run the process backend. Not a found bug and not a usage error.
pub const EXIT_UNSUPPORTED_HOST: i32 = 4;

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

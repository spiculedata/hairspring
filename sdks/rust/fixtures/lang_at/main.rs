//! The Rust language fixture: one binary, three modes chosen by `argv[1]` (`buggy`, `fixed`, `wide`).
//!
//! Four threads each add one to a shared counter a thousand times. A step loads the counter, spends a fixed number of
//! busy iterations (no system call in the window) and stores the value loaded plus one. `buggy` does that with no lock,
//! so two threads can load the same value and one update is lost; `fixed` holds a mutex around each step; `wide` is
//! `buggy` with a window ten times longer. The mode is the argument, never a build flag, so one artifact runs in every
//! arm.
//!
//! After the workers are joined, the main thread evaluates the two properties once each (`rust.counter_exact`, an
//! `always` with the witness `[final, 4000, 4, 1000]`, and `rust.all_started`, a `sometimes`), writes one `LANGAT v1`
//! line on stdout and exits 0 whatever the property says: the verdict comes from the property report. Exit 3 is an
//! internal invariant failure, exit 64 a usage error.

mod line;
mod mode;
mod window;

use std::io::Write;
use std::process::ExitCode;

use hairspring_sdk::{always, sometimes};

use mode::{Mode, ITERATIONS, THREADS};
use window::{Invariant, Outcome};

const EXIT_INVARIANT: u8 = 3;
const EXIT_USAGE: u8 = 64;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match mode::parse(&args) {
        Ok(mode) => mode,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match window::run_all(mode).and_then(|outcome| report(mode, &outcome)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Invariant(why)) => {
            eprintln!("at-rust: internal invariant failed: {why}");
            ExitCode::from(EXIT_INVARIANT)
        }
    }
}

/// Evaluate the properties once, then write the one stdout line with a single write.
fn report(mode: Mode, outcome: &Outcome) -> Result<(), Invariant> {
    let expected = THREADS * ITERATIONS;
    always!(
        outcome.final_value == expected,
        "rust.counter_exact",
        "counter equals T*K",
        outcome.final_value,
        expected,
        THREADS,
        ITERATIONS
    );
    sometimes!(
        outcome.started == THREADS,
        "rust.all_started",
        "every worker started",
        outcome.started
    );
    let random = std::collections::hash_map::RandomState::new();
    let stdout_line = line::format_line(
        mode,
        outcome.final_value,
        expected,
        line::order_hash(&outcome.arrival),
        std::hash::BuildHasher::hash_one(&random, 0u64),
    );
    std::io::stdout()
        .lock()
        .write_all(stdout_line.as_bytes())
        .map_err(|_| Invariant("the stdout line could not be written"))
}

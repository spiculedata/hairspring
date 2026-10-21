//! `probe-rust`: a std-only program that checks how Rust `std::thread` programs are scheduled. It has no SDK and no
//! property; it takes no arguments.
//!
//! Four threads each take a `Mutex<Vec>` once and push their number (the lock order), run a relaxed `AtomicU64`
//! read-spin-write with no lock, and report on an `mpsc` channel when they finish (the arrival order). A `HashMap` built
//! with the standard library's random state gives a fourth structure (its iteration order). The program prints exactly
//! one line on stdout, with one write, and exits 0:
//!
//! `PROBE v1 lang=rust lock_order=<16 hex> arrival=<16 hex> rand=<16 hex> vtime_ns=<20 digits> extra=<16 hex>`
//!
//! `lock_order`, `arrival` and `extra` are FNV-1a 64 hashes of the thread numbers (or map keys) in that order, `rand` is
//! one value from the standard library's random source, and `vtime_ns` is the monotonic clock difference between the
//! start and the end of the run. Exit 3 is an internal invariant failure (a thread did not start or panicked, the line
//! could not be written); exit 64 is a usage error (any argument).

mod line;
mod workload;

use std::io::Write;
use std::process::ExitCode;

use line::{format_line, hash_sequence, Fields};
use workload::{Invariant, Outcome};

const EXIT_INVARIANT: u8 = 3;
const EXIT_USAGE: u8 = 64;
const USAGE: &str = "usage: probe-rust";

fn main() -> ExitCode {
    if std::env::args().len() > 1 {
        eprintln!("{USAGE}");
        return ExitCode::from(EXIT_USAGE);
    }
    match workload::run().and_then(|outcome| write_line(&outcome)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Invariant(why)) => {
            eprintln!("probe-rust: internal invariant failed: {why}");
            ExitCode::from(EXIT_INVARIANT)
        }
    }
}

fn fields_of(outcome: &Outcome) -> Fields {
    Fields {
        lock_order: hash_sequence(&outcome.lock_order),
        arrival: hash_sequence(&outcome.arrival),
        rand: outcome.rand,
        vtime_ns: outcome.elapsed_ns,
        extra: hash_sequence(&outcome.map_order),
    }
}

/// The one stdout line, with a single write; the flush is part of the same call because the line ends in a newline.
fn write_line(outcome: &Outcome) -> Result<(), Invariant> {
    let text = format_line(&fields_of(outcome));
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|_| Invariant("the stdout line could not be written"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(lock_order: &[u64], arrival: &[u64], map_order: &[u64]) -> Outcome {
        Outcome {
            lock_order: lock_order.to_vec(),
            arrival: arrival.to_vec(),
            racy_final: 800,
            elapsed_ns: 42,
            map_order: map_order.to_vec(),
            rand: 7,
        }
    }

    #[test]
    fn the_fields_hash_each_order_and_pass_the_rest_through() {
        let fields = fields_of(&outcome(&[0, 1, 2, 3], &[3, 2, 1, 0], &[5, 4]));
        assert_eq!(fields.lock_order, hash_sequence(&[0, 1, 2, 3]));
        assert_eq!(fields.arrival, hash_sequence(&[3, 2, 1, 0]));
        assert_eq!(fields.extra, hash_sequence(&[5, 4]));
        assert_eq!((fields.rand, fields.vtime_ns), (7, 42));
    }

    #[test]
    fn the_same_outcome_gives_the_same_line() {
        let a = outcome(&[2, 0, 1, 3], &[1, 0, 2, 3], &[9, 8, 7]);
        assert_eq!(format_line(&fields_of(&a)), format_line(&fields_of(&a)));
    }

    #[test]
    fn a_different_arrival_order_changes_only_the_arrival_field() {
        let a = fields_of(&outcome(&[0, 1, 2, 3], &[0, 1, 2, 3], &[1]));
        let b = fields_of(&outcome(&[0, 1, 2, 3], &[1, 0, 2, 3], &[1]));
        assert_eq!(
            (a.lock_order, a.rand, a.extra),
            (b.lock_order, b.rand, b.extra)
        );
        assert_ne!(a.arrival, b.arrival);
    }
}

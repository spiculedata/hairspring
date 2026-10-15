//! Hermit backend: explores a real program (`kind = "process"`) by running it under
//! [Hermit](https://github.com/rrnewton/hermit) with different scheduler seeds.
//!
//! Hermit makes a multithreaded Linux program a deterministic function of its inputs, so a failing
//! seed reproduces the failure exactly. The result is a seed to replay rather than a state tree:
//! there are no snapshots, so "time travel" means re-running the same seed.
//!
//! Verdicts: the guest violates a property if it exits non-zero or prints a line starting with
//! `HAIRSPRING_VIOLATION:`. A trial that hits its time limit is inconclusive (counted separately, never a pass) unless the
//! target sets `hang_is_violation`; a run where every trial timed out is an error. Linux x86-64 only; experimental.
mod backend;
mod env;
mod exec;
mod trace;

pub use backend::HermitBackend;
pub use env::guest_env;

use std::path::PathBuf;
use std::time::Duration;

/// Environment variable naming the hermit executable.
pub const HERMIT_ENV: &str = "HAIRSPRING_HERMIT";

#[derive(Debug, Clone)]
pub struct HermitSettings {
    /// The hermit executable.
    pub bin: PathBuf,
    /// Virtual clock epoch passed to every run; fixed so time is not an input.
    pub epoch: String,
    /// Longest a single trial may run before it is killed and counted as timed out.
    pub trial_timeout: Duration,
    /// Extra environment for the hermit process itself (not the guest's configured env).
    pub extra_env: Vec<(String, String)>,
    /// How many times a violating seed is re-run to confirm it reproduces.
    pub repro_runs: u32,
    /// Output kept per stream; the rest is drained and discarded.
    pub max_capture_bytes: usize,
}

impl Default for HermitSettings {
    fn default() -> Self {
        HermitSettings {
            bin: std::env::var_os(HERMIT_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("hermit")),
            epoch: "2026-01-01T00:00:00Z".to_string(),
            trial_timeout: Duration::from_secs(60),
            extra_env: Vec::new(),
            repro_runs: 3,
            max_capture_bytes: 1024 * 1024,
        }
    }
}

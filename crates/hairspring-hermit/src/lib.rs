//! Hermit backend: explores a real program (`kind = "process"`) by running it under
//! [Hermit](https://github.com/rrnewton/hermit) with different scheduler seeds.
//!
//! Hermit makes a multithreaded Linux program a deterministic function of its inputs, so a failing
//! seed reproduces the failure exactly. The result is a seed to replay rather than a state tree:
//! there are no snapshots, so "time travel" means re-running the same seed.
//!
//! Verdicts: the guest violates a property if it exits non-zero or prints a line starting with
//! `HAIRSPRING_VIOLATION:`. A trial that hits its time limit is inconclusive (counted separately, never a pass) unless the
//! target sets `hang_is_violation`; a run where every trial timed out is an error. Trials run in batches of `parallelism`
//! seeds, so a cancel takes effect at the next batch boundary. Lines starting with `HAIRSPRING_EVENT ` (the guest event
//! protocol) are captured separately from the capped output and folded into the trial's `events`. Linux x86-64 only; experimental.
mod backend;
mod capture;
mod env;
mod events;
mod exec;
mod head_tail;
mod property_verdict;
mod safe_open;
mod schedule_set;
mod summary;
mod summary_dir;
mod tally;
#[cfg(test)]
mod tally_log_tests;
mod trace;
mod trial_log;
#[cfg(test)]
mod verdict_tests;

pub use backend::HermitBackend;
pub use env::guest_env;
pub use events::{EventsFault, GUEST_EVENTS_FILE};
pub use exec::Trial;
pub use head_tail::Omitted;
pub use tally::TrialObserver;

use hairspring_props::limits::DEFAULT_MAX_EVENT_BYTES_PER_TRIAL;
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
    /// Ordinary output kept per stream: the first half and the last half of this many bytes. The middle is drained,
    /// counted and hashed but not stored, and a marker with its size stands in for it. Event lines are separate.
    pub max_capture_bytes: usize,
    /// Where Hermit writes each trial's `--summary-json`. It must be outside `/tmp` (Hermit gives the guest its own).
    /// `None` turns schedule fingerprints off; defaults to `/dev/shm` when that exists.
    pub summary_dir: Option<PathBuf>,
    /// Offer the guest an events file (`HAIRSPRING_EVENTS_FILE`) in addition to stdout and stderr. Needs `summary_dir`,
    /// and uses Hermit's `--tmp` for the guest's `/tmp`. Off by default: it changes what the guest's `/tmp` is.
    pub events_file: bool,
    /// Where the events file's per-trial directories go. `None` uses `summary_dir`; set this to a disk-backed
    /// directory when guests may write more than `/dev/shm` can hold.
    pub events_dir: Option<PathBuf>,
    /// Most event-line bytes accepted from one trial, shared by stdout, stderr and the events file. Defaults to
    /// 4 MiB (about 4,900 ids at roughly 850 bytes each). The buffer grows as events arrive, so only trials that emit
    /// events spend memory.
    pub max_event_bytes: usize,
}

/// Memory-backed and not `/tmp`, so Hermit accepts it and the files cost nothing.
const DEFAULT_SUMMARY_DIR: &str = "/dev/shm";

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
            summary_dir: std::path::Path::new(DEFAULT_SUMMARY_DIR)
                .is_dir()
                .then(|| PathBuf::from(DEFAULT_SUMMARY_DIR)),
            events_file: false,
            events_dir: None,
            max_event_bytes: DEFAULT_MAX_EVENT_BYTES_PER_TRIAL,
        }
    }
}

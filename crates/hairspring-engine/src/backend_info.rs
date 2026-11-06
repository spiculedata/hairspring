//! What a backend says about itself: who it is, what it can do and what it limits.
//!
//! These are facts the backend states, not checks: an API caller reads them to decide what to ask for. A flag is
//! `false` unless the backend really has the feature, and a limit is `None` unless the backend states one.
use serde::{Deserialize, Serialize};

/// What a backend can do. Every flag defaults to `false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// A recorded failure can be replayed.
    pub replay: bool,
    /// The confirmation checks (reruns, control, strict verify) can run on a failure.
    pub confirm: bool,
    /// A failing run is shrunk to a smaller one.
    pub shrink: bool,
    /// The backend has a strict determinism verify mode that confirmation can use.
    pub strict_verify: bool,
    /// Execution can be snapshotted and branched from a saved state.
    pub snapshot_branch: bool,
    /// The backend injects faults itself (network, disk, clock, crash).
    pub fault_injection: bool,
    /// The target may be a tree of processes rather than one.
    pub multi_process: bool,
}

/// Bounds the backend states for what a caller may ask for. `None` means the backend states no bound.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendLimits {
    /// Most trials the backend runs at the same time.
    pub max_parallelism: Option<u32>,
    /// Longest one trial may run, in seconds.
    pub max_trial_seconds: Option<u64>,
}

/// Which build of the backend this is. The binary fields are filled only when they are known without running
/// anything: asking for them must never start a process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendVersion {
    /// The version of the crate that implements the backend.
    pub crate_version: String,
    /// The version text of an external program the backend drives, when known.
    pub binary_version: Option<String>,
    /// SHA-256 of that program, when known.
    pub binary_sha256: Option<String>,
}

impl BackendVersion {
    pub fn new(crate_version: &str) -> BackendVersion {
        BackendVersion {
            crate_version: crate_version.to_string(),
            binary_version: None,
            binary_sha256: None,
        }
    }
}

/// A backend's identity, as listed for callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    pub id: String,
    pub display_name: String,
    pub version: BackendVersion,
    pub capabilities: Capabilities,
    pub limits: BackendLimits,
}

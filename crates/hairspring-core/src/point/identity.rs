//! Run identity: every input that can change execution, as canonical text.
//!
//! An [`IdentityDraft`] holds what was recorded. [`IdentityDraft::finish`] returns an
//! [`Identity`] only when every input is present and valid; otherwise it returns
//! [`IdentityError::Incomplete`] and no hash exists for the run.
use super::address::BackendKind;
use super::encoding::{sha256_hex, Recorded};
use super::identity_text;

/// Longest identity text, in bytes (the store column limit).
pub const MAX_IDENTITY_TEXT_BYTES: usize = 16 * 1024;
/// Environment variable that carries the seed; it is the seed, not an identity input.
pub const SEED_ENV_NAME: &str = "HAIRSPRING_SEED";

/// Inputs every backend records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommonInputs {
    /// SHA-256 of the running executable's bytes.
    pub hairspring_sha: Recorded<String>,
    pub target: Recorded<String>,
}

/// Sim targets are compiled into the executable, so the common inputs cover their code.
pub type SimInputs = CommonInputs;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WasmInputs {
    /// SHA-256 of the module bytes.
    pub module_sha: Recorded<String>,
    pub fuel: Recorded<u64>,
    pub max_memory_bytes: Recorded<u64>,
    /// The VM semantics version (choice salt, clock rule); bumped on any VM change.
    pub vm_semantics: Recorded<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TmpPolicy {
    /// The guest's `/tmp` is not replaced.
    None,
    /// The flag form of the `--tmp` setting.
    Path(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HermitInputs {
    pub hermit_sha: Recorded<String>,
    /// The target's command line, each element.
    pub target_argv: Recorded<Vec<String>>,
    pub target_exe_sha: Recorded<String>,
    pub epoch: Recorded<String>,
    /// The scheduling heuristic flag value; `None` when the flag is not passed.
    pub sched_heuristic: Recorded<Option<String>>,
    pub sched_sticky_permille: Recorded<Option<u64>>,
    pub sched_chaos: Recorded<bool>,
    pub sched_max_timeslice_ns: Recorded<Option<u64>>,
    pub sched_skid_margin: Recorded<Option<u64>>,
    pub sched_rbc_overshoot: Recorded<bool>,
    pub hermit_args: Recorded<Vec<String>>,
    /// The guest environment in the order given, after the environment is cleared.
    pub guest_env: Recorded<Vec<(String, String)>>,
    /// The resolved working directory.
    pub cwd: Recorded<String>,
    pub tmp_policy: Recorded<TmpPolicy>,
    pub events_transport: Recorded<String>,
    pub loader_ld_so_cache_sha: Recorded<String>,
    pub loader_libc_sha: Recorded<String>,
    pub loader_ld_linux_sha: Recorded<String>,
    pub hang_is_violation: Recorded<bool>,
}

/// What was recorded for a run, before the completeness check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityDraft {
    Sim(SimInputs),
    Wasm(CommonInputs, WasmInputs),
    Hermit(CommonInputs, HermitInputs),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    /// Inputs that were not recorded (or are not valid content hashes). No hash exists.
    Incomplete {
        missing: Vec<&'static str>,
    },
    /// A recorded input that must not appear in the identity.
    Invalid {
        field: &'static str,
        reason: &'static str,
    },
    TooLarge {
        bytes: usize,
    },
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityError::Incomplete { missing } => {
                write!(
                    f,
                    "identity incomplete: not recorded: {}",
                    missing.join(", ")
                )
            }
            IdentityError::Invalid { field, reason } => {
                write!(f, "identity input `{field}` is invalid: {reason}")
            }
            IdentityError::TooLarge { bytes } => write!(
                f,
                "identity text is {bytes} bytes, over the {MAX_IDENTITY_TEXT_BYTES} byte limit"
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

/// A complete run identity: canonical text and its hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    backend: BackendKind,
    text: String,
}

impl IdentityDraft {
    pub fn backend(&self) -> BackendKind {
        match self {
            IdentityDraft::Sim(_) => BackendKind::Sim,
            IdentityDraft::Wasm(..) => BackendKind::Wasm,
            IdentityDraft::Hermit(..) => BackendKind::Hermit,
        }
    }

    /// Checks that every input is recorded and valid, and renders the canonical text.
    pub fn finish(&self) -> Result<Identity, IdentityError> {
        let text = identity_text::render(self)?;
        if text.len() > MAX_IDENTITY_TEXT_BYTES {
            return Err(IdentityError::TooLarge { bytes: text.len() });
        }
        Ok(Identity {
            backend: self.backend(),
            text,
        })
    }
}

impl Identity {
    pub fn backend(&self) -> BackendKind {
        self.backend
    }

    /// The canonical text: one `key=value` line per input, each ended by LF.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// SHA-256 hex of [`Self::text`].
    pub fn hash(&self) -> String {
        sha256_hex(self.text.as_bytes())
    }
}

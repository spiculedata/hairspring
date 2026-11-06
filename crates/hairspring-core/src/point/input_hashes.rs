//! Per-field hashes of a run's inputs.
//!
//! The canonical identity text holds values, among them the working directory and the guest environment. Keeping that
//! for every run would put environment values in a database. What a comparison of two runs needs is only whether an
//! input is the same or different, so each input is reduced to the SHA-256 of its rendered value (the text that follows
//! `key=` in the identity text). The values themselves never leave the draft.
//!
//! A digest of a short value can be guessed by trying candidates, so a store that keeps these must not serve them.
use super::address::BackendKind;
use super::encoding::sha256_hex;
use super::identity::IdentityDraft;
use super::identity_text::render_fields;
use std::collections::BTreeMap;

/// The most fields one record may hold. The largest scheme has 24; the cap leaves room for a later scheme.
pub const MAX_INPUT_FIELDS: usize = 64;
/// The most bytes a field name may have.
pub const MAX_FIELD_NAME_BYTES: usize = 40;

const SIM_FIELDS: [&str; 3] = ["hairspring_sha", "target", "backend"];
const WASM_FIELDS: [&str; 7] = [
    "hairspring_sha",
    "target",
    "backend",
    "module_sha",
    "fuel",
    "max_memory_bytes",
    "vm_semantics",
];
const HERMIT_FIELDS: [&str; 24] = [
    "hairspring_sha",
    "target",
    "backend",
    "hermit_sha",
    "target_argv",
    "target_exe_sha",
    "epoch",
    "sched_heuristic",
    "sched_sticky_permille",
    "sched_chaos",
    "sched_max_timeslice_ns",
    "sched_skid_margin",
    "sched_rbc_overshoot",
    "hermit_args",
    "guest_env",
    "cwd",
    "tmp_policy",
    "events_transport",
    "loader_ld_so_cache_sha",
    "loader_libc_sha",
    "loader_ld_linux_sha",
    "hang_is_violation",
    "trial_timeout_secs",
    "parallelism",
];

/// The field names a scheme records (not `scheme` itself), or `None` for a scheme this build does not know.
pub fn known_fields(scheme: &str) -> Option<&'static [&'static str]> {
    match BackendKind::from_scheme(scheme)? {
        BackendKind::Sim => Some(&SIM_FIELDS),
        BackendKind::Wasm => Some(&WASM_FIELDS),
        BackendKind::Hermit => Some(&HERMIT_FIELDS),
    }
}

/// A run's inputs as digests: the scheme, the digest of each recorded input by name, and the names of the inputs that
/// could not be recorded (missing, or not valid values).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputHashes {
    pub scheme: String,
    pub fields: BTreeMap<String, String>,
    pub missing: Vec<String>,
}

impl IdentityDraft {
    /// Reduces the draft to digests. It never fails: an input that is missing or invalid is listed in
    /// [`InputHashes::missing`] and has no digest, so a partly recorded run still says what it knows.
    pub fn input_hashes(&self) -> InputHashes {
        let (pairs, absent) = render_fields(self);
        let mut missing: Vec<String> = absent.into_iter().map(str::to_string).collect();
        missing.sort();
        missing.dedup();
        InputHashes {
            scheme: self.backend().scheme().to_string(),
            fields: pairs
                .into_iter()
                .map(|(key, value)| (key, sha256_hex(value.as_bytes())))
                .collect(),
            missing,
        }
    }
}

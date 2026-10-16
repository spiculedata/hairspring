//! The point hash: SHA-256 over the run identity, the seed, the cut and the cut's prefix.
//!
//! The cut is hashed as its own `cut=` line, so `end` never hashes like `s<len>` or `c<len>`
//! over the same trace (for WASM they are different VM states).
use super::address::{BackendKind, Cut};
use super::encoding::sha256_hex;
use super::identity::Identity;
use crate::ActionId;

/// First line of every point hash input; bumped if the layout below ever changes.
pub const POINT_HASH_HEADER: &str = "hairspring-point v1\n";

/// What the cut keeps of the trial: the part of the execution that the point fixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix<'a> {
    /// A sim point: the action ids from the root up to the cut.
    Sim(&'a [ActionId]),
    /// A WASM point: the choice values made up to the cut.
    Wasm(&'a [u64]),
    /// A Hermit point: the whole trial (`end`) only.
    HermitEnd,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HashError {
    /// The cut kind is not valid for the backend.
    CutNotValidForBackend,
    /// The cut is further along than the recorded trace.
    CutBeyondTrace { cut: u64, available: usize },
}

impl std::fmt::Display for HashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashError::CutNotValidForBackend => {
                f.write_str("this cut kind is not valid for the run's backend")
            }
            HashError::CutBeyondTrace { cut, available } => {
                write!(f, "cut {cut} is beyond the {available} recorded entries")
            }
        }
    }
}

impl std::error::Error for HashError {}

impl<'a> Prefix<'a> {
    /// Applies a cut to the full trace of a trial (action ids for sim, choice values for WASM;
    /// ignored for Hermit).
    pub fn for_cut(
        backend: BackendKind,
        cut: Cut,
        full_trace: &'a [u64],
    ) -> Result<Prefix<'a>, HashError> {
        match (backend, cut) {
            (BackendKind::Sim, Cut::End) => Ok(Prefix::Sim(full_trace)),
            (BackendKind::Sim, Cut::Step(k)) => take(full_trace, k).map(Prefix::Sim),
            (BackendKind::Wasm, Cut::End) => Ok(Prefix::Wasm(full_trace)),
            (BackendKind::Wasm, Cut::Choice(k)) => take(full_trace, k).map(Prefix::Wasm),
            (BackendKind::Hermit, Cut::End) => Ok(Prefix::HermitEnd),
            _ => Err(HashError::CutNotValidForBackend),
        }
    }

    /// The last line of the hash input, after the `cut=` line. It has no line ending, and is
    /// empty for Hermit, whose point is the whole trial.
    fn line(&self) -> String {
        match self {
            Prefix::Sim(path) => {
                let ids: Vec<String> = path.iter().map(|id| format!("{id:016x}")).collect();
                format!("path={}", ids.join(","))
            }
            Prefix::Wasm(choices) => {
                let values: Vec<String> = choices.iter().map(u64::to_string).collect();
                format!("choices={}", values.join(","))
            }
            Prefix::HermitEnd => String::new(),
        }
    }
}

fn take(trace: &[u64], k: u64) -> Result<&[u64], HashError> {
    usize::try_from(k)
        .ok()
        .and_then(|n| trace.get(..n))
        .ok_or(HashError::CutBeyondTrace {
            cut: k,
            available: trace.len(),
        })
}

/// Converts a seed as stored (an `i64` column holding the `u64` bit pattern) to the `u64` that is
/// hashed. This is the only place the conversion is defined.
pub fn seed_from_stored(stored: i64) -> u64 {
    stored as u64
}

/// The point hash, as lowercase hex.
///
/// The run id is deliberately not an input: the same identity, seed and prefix hash the same in
/// any run. Only a complete [`Identity`] can be passed, so a run with a missing input never gets
/// a hash.
pub fn point_hash(
    identity: &Identity,
    seed: u64,
    cut: Cut,
    trace: &[u64],
) -> Result<String, HashError> {
    let prefix = Prefix::for_cut(identity.backend(), cut, trace)?;
    let input = format!(
        "{POINT_HASH_HEADER}{}seed={seed}\ncut={cut}\n{}",
        identity.text(),
        prefix.line()
    );
    Ok(sha256_hex(input.as_bytes()))
}

use crate::rng::Rng;
use crate::{stable_id, Sim};

#[derive(Debug, PartialEq, Eq)]
pub struct Divergence {
    /// First step at which the two runs disagreed.
    pub step: usize,
    pub first: u64,
    pub second: u64,
}

/// Per-step rolling hashes of a seeded walk. Each hash covers the ordered enabled
/// set, the choice made, and the resulting signature, so order leaks are visible.
fn trace_hashes<S: Sim>(root: &S, seed: u64, steps: usize) -> Vec<u64> {
    let mut rng = Rng::new(seed);
    let mut state = root.clone();
    let mut prev = 0u64;
    let mut hashes = Vec::with_capacity(steps);
    for _ in 0..steps {
        let enabled = state.enabled();
        if enabled.is_empty() {
            break;
        }
        let choice = rng.below(enabled.len());
        let mut parts: Vec<u64> = vec![prev, choice as u64];
        parts.extend(enabled.iter().map(S::action_id));
        state = state.apply(enabled[choice]);
        parts.push(state.signature());
        prev = stable_id(&parts);
        hashes.push(prev);
    }
    hashes
}

/// Run the same seeded walk twice and compare. Ok carries the final trace hash.
/// Note: in-process only; leaks that clone a shared hasher need a cross-process run.
pub fn doctor<S: Sim>(root: &S, seed: u64, steps: usize) -> Result<u64, Divergence> {
    let (a, b) = (
        trace_hashes(root, seed, steps),
        trace_hashes(root, seed, steps),
    );
    let common = a.len().min(b.len());
    if let Some(step) = (0..common).find(|&i| a[i] != b[i]) {
        return Err(Divergence {
            step,
            first: a[step],
            second: b[step],
        });
    }
    if a.len() != b.len() {
        let step = common;
        return Err(Divergence {
            step,
            first: a.get(step).copied().unwrap_or(0),
            second: b.get(step).copied().unwrap_or(0),
        });
    }
    Ok(a.last().copied().unwrap_or(0))
}

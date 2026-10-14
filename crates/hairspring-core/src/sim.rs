//! The contract a system under test implements to be explored.
use std::fmt::Debug;

/// Stable identifier for an action. MUST NOT depend on positions in `enabled()`,
/// otherwise deleting one step during shrinking would re-label every later step.
pub type ActionId = u64;

pub trait Sim: Clone {
    type Action: Copy + Debug;

    /// Actions possible now, in a deterministic order.
    fn enabled(&self) -> Vec<Self::Action>;
    fn action_id(action: &Self::Action) -> ActionId;
    /// Pure transition: returns the next state, never mutates `self`.
    fn apply(&self, action: Self::Action) -> Self;
    /// Abstract state used as exploration "coverage".
    fn signature(&self) -> u64;
    fn violated(&self) -> bool;
}

/// FNV-1a over u64 parts: a stable, dependency-free way to build `ActionId`s.
pub fn stable_id(parts: &[u64]) -> ActionId {
    parts
        .iter()
        .flat_map(|p| p.to_le_bytes())
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        })
}

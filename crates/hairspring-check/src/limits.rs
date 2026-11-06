//! Explicit work limits. Every search is bounded; running out gives `Unknown`, never `Ok`.

/// Default cap on search steps per key (candidate operations tried).
pub const DEFAULT_MAX_STEPS: u64 = 2_000_000;
/// Default cap on remembered failed search states per key.
pub const DEFAULT_MAX_MEMO_ENTRIES: usize = 200_000;
/// Default cap on operations per key.
pub const DEFAULT_MAX_OPS_PER_KEY: usize = 5_000;
/// Default cap on steps spent shrinking a violation's witness (per violation).
pub const DEFAULT_MAX_WITNESS_STEPS: u64 = 500_000;

/// Work limits for the checkers. The linear-time checkers use only `max_ops_per_key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_steps: u64,
    pub max_memo_entries: usize,
    pub max_ops_per_key: usize,
    pub max_witness_steps: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_steps: DEFAULT_MAX_STEPS,
            max_memo_entries: DEFAULT_MAX_MEMO_ENTRIES,
            max_ops_per_key: DEFAULT_MAX_OPS_PER_KEY,
            max_witness_steps: DEFAULT_MAX_WITNESS_STEPS,
        }
    }
}

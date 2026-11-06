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
///
/// What the limits do and do not bound:
/// - `max_steps` counts candidate operations tried and already-placed operations skipped while looking for the next
///   candidate. It is not a time limit: a step also clones the placed-set (`operations / 8` bytes) to probe the memo,
///   so a step costs more on a key with many operations.
/// - `max_memo_entries` counts remembered failed states, not bytes. Each holds a copy of the placed-set, so memory is
///   about `max_memo_entries * (operations_on_the_key / 8 + 40)` bytes: roughly 650 bytes per entry at 5,000
///   operations, so the default 200,000 entries can reach about 130 MB on a key that large. Lower it, or
///   `max_ops_per_key`, where memory matters.
/// - `max_ops_per_key` is checked before any search, per key.
/// - `max_witness_steps` is a separate budget for shrinking a violation; the shrink also honours `max_memo_entries`.
///   When either runs out, the witness is reported as not minimal.
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

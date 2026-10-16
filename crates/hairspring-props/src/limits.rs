//! Protocol v1 constants. These are mirrored in `sdks/conformance/schema/v1.json`; a test keeps the two in step.

/// Every event line starts with this prefix (one trailing space).
pub const EVENT_PREFIX: &str = "HAIRSPRING_EVENT ";
/// The prefix without its space, used to detect a missing separator.
pub const EVENT_PREFIX_BARE: &str = "HAIRSPRING_EVENT";
/// The protocol major version this crate understands.
pub const PROTOCOL_VERSION: u64 = 1;

/// Longest accepted event line in bytes, newline excluded.
pub const MAX_LINE_BYTES: usize = 4096;
/// Longest assertion id in bytes.
pub const MAX_ID_BYTES: usize = 128;
/// Longest thread key in bytes.
pub const MAX_THREAD_KEY_BYTES: usize = 64;
/// Longest process tag in bytes (same charset as thread keys).
pub const MAX_PROC_BYTES: usize = 64;
/// Longest assertion message in bytes.
pub const MAX_MSG_BYTES: usize = 512;
/// Longest SDK name or version string in bytes.
pub const MAX_SDK_STR_BYTES: usize = 64;
/// Longest catalog hash string in bytes.
pub const MAX_HASH_BYTES: usize = 128;
/// Longest source file path in `loc` in bytes.
pub const MAX_LOC_FILE_BYTES: usize = 256;
/// Longest function name in `loc` in bytes.
pub const MAX_LOC_FUNC_BYTES: usize = 128;
/// Exactly this many integer values accompany a first-true or first-false witness.
pub const VALS_LEN: usize = 4;
/// Most catalogued ids one process may declare.
pub const MAX_IDS_PER_PROC: usize = 4096;
/// Most distinct ids the host tracks across a whole run. This is a host-side limit, not part of the guest protocol.
/// One trial can catalog about 4,900 ids within its byte budget, so a program with a stable catalog needs a few
/// thousand at most; 16,384 leaves about three times that for programs that vary their catalog by seed. Each tracked
/// id costs at most about 1.5 KiB (id, message and location), so the map is bounded near 25 MiB however many trials
/// run and however many per-seed-unique ids a guest invents.
pub const MAX_IDS_PER_RUN: usize = 16 * 1024;
/// Most `thread` lines (and so the largest `end.threads` count) one process may send.
pub const MAX_THREADS_PER_PROC: usize = 256;
/// Most processes one trial may report.
pub const MAX_PROCS_PER_TRIAL: usize = 64;
/// The event-line byte budget before it was raised to 4 MiB. Kept so a smaller budget can still be asked for.
pub const LEGACY_MAX_EVENT_BYTES_PER_TRIAL: usize = 1024 * 1024;
/// Default most event-line bytes (newlines included) accepted from one trial. Each transport (stdout, stderr, the
/// events file) is captured against this budget on its own, shared by the processes writing to it, and the fold
/// applies it again to the merged stream; so the events kept never exceed it, but memory per trial can reach about
/// three times it. A catalog line plus an assert line cost about 850 bytes per id,
/// so 4 MiB holds about 4,900 ids, or about 1,200 ids per MiB summed over processes. Backends take the budget as a
/// parameter (`TrialEvents::fold_with_budget`); this is the value the schema documents and the default they use.
pub const DEFAULT_MAX_EVENT_BYTES_PER_TRIAL: usize = 4 * 1024 * 1024;
/// The documented per-trial budget (the default).
pub const MAX_EVENT_BYTES_PER_TRIAL: usize = DEFAULT_MAX_EVENT_BYTES_PER_TRIAL;
/// Deepest JSON nesting accepted inside an event object (root object = depth 1).
pub const MAX_JSON_DEPTH: usize = 4;

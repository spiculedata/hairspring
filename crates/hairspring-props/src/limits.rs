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
/// Most thread entries one `end` line may carry.
pub const MAX_THREADS_PER_PROC: usize = 256;
/// Most processes one trial may report.
pub const MAX_PROCS_PER_TRIAL: usize = 64;
/// Most event-line bytes (newlines included) accepted from one trial.
pub const MAX_EVENT_BYTES_PER_TRIAL: usize = 1024 * 1024;
/// Deepest JSON nesting accepted inside an event object (root object = depth 1).
pub const MAX_JSON_DEPTH: usize = 4;

//! WebAssembly backend: deterministic, portable (macOS and Linux) execution of guests that
//! consult the host for every source of nondeterminism.
//!
//! Guests import from module `hairspring`:
//! - `choose(n: i32) -> i32`: a decision in `0..n` (scheduling, message loss, ...), recorded for replay and shrinking
//! - `rand_u64() -> i64`, `now_ms() -> i64`, `sleep_ms(ms: i64)`: seeded randomness and a virtual clock
//! - `log(ptr, len)` and `fail(ptr, len)`: a log line, and a violated property (stops the run)
//!
//! and export `run() -> i32` (0 = success) plus a `memory`. Everything else in WebAssembly is
//! deterministic once NaN payloads are canonicalised (we do) and threads are absent (they are).
mod backend;
mod sweep;
mod vm;

pub use backend::{WasmBackend, WasmSettings};
pub use vm::{CompiledModule, RunResult, Snapshot, VmConfig, WasmError, WasmVm};

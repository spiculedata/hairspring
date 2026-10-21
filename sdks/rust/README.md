# hairspring-sdk (Rust)

In-source property assertions for Rust programs under test. A call site updates counters in memory; the program reports
once, at exit, in [guest protocol v1](../../docs/guest-protocol.md). No dependencies; Linux and macOS.

```rust
use hairspring_sdk::{always, reachable, sometimes, thread, work};

always!(nbackfill <= mxframe, "wal.ckpt.nbackfill_le_mxframe", "nBackfill <= live mxFrame", nbackfill, mxframe);
sometimes!(waited, "wal.writer.waited", "a writer waited for the lock");
reachable!("wal.reset.reached", "the WAL was reset");
work(1);
let _ = thread("writer", 0);
```

* Documentation: [docs/rust-sdk.md](../../docs/rust-sdk.md) (API, environment, the linker-collected catalog, evidence, limits).
* Example: `cargo run -p hairspring-sdk --example quickstart`.
* Tests: `cargo test -p hairspring-sdk`; conformance against the reference parser: `cargo test -p hairspring-props --test rust_sdk`.
* `samples/` are the programs the tests run; `fixtures/` holds the planted race (`counter_racy.rs`) and its locked twin
  (`counter_locked.rs`).

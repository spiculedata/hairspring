//! Two call sites compiled as plain functions, so their machine code can be read:
//!
//! ```text
//! cargo rustc -p hairspring-sdk --release --example callsite -- --emit asm
//! objdump -d --no-show-raw-insn target/release/examples/callsite | less   # then look at site_always, site_reach
//! ```
//!
//! The functions are `#[no_mangle]` and `#[inline(never)]` only so that they can be found; a real call site is
//! inlined into its caller.

use hairspring_sdk::{always, reachable};

#[no_mangle]
#[inline(never)]
pub extern "C" fn site_always(ok: bool, a: i64, b: i64) {
    always!(ok, "disasm.always", "cond holds", a, b, 3, 4);
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn site_reach() {
    reachable!("disasm.reach", "reached");
}

fn main() {
    site_always(std::env::args().count() > 5, 1, 2);
    site_reach();
}

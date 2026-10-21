//! The buggy build: an unlocked read-modify-write with a busy window and no system call inside it.

#[path = "counter_common.rs"]
mod common;

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

fn main() {
    // Separate load and store of an atomic: well-defined, and still racy (a lost update, not a data race).
    let counter = AtomicU64::new(0);
    common::run(
        &|| {
            let seen = counter.load(Relaxed);
            common::busy(common::WINDOW);
            counter.store(seen + 1, Relaxed);
        },
        &|| counter.load(Relaxed),
    );
}

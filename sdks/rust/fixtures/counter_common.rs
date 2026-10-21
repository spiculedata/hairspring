//! The planted race, shared by its two builds (`counter_racy.rs`, `counter_locked.rs`).
//!
//! Four threads each add one to a shared counter a thousand times. A step reads the counter, spends a fixed number of
//! busy iterations (no system call in the window), and writes back the value read plus one. The racy build does
//! that with no lock, so two threads can read the same value and one update is lost; the locked twin holds a mutex
//! around the whole step, so none can be.
//!
//! The property is one `always`: the final count equals threads times iterations. A `sometimes` marks that every
//! worker started. The program prints one fixed-width line (the final value, a hash of the order the workers finished
//! in, one runtime random value so that entropy leaks show) and exits 0 whatever the property says: the verdict comes
//! from the property report, not from the exit code.

use std::hash::BuildHasher;
use std::sync::Mutex;

use hairspring_sdk::{always, sometimes};

pub const THREADS: u64 = 4;
pub const ITERATIONS: u64 = 1_000;
/// Busy iterations between the read and the write. Wide enough that a preemption or a second core lands inside it.
pub const WINDOW: u64 = 2_000;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A busy loop with an opaque accumulator, so the optimiser can neither remove it nor unroll it away.
#[inline(never)]
pub fn busy(iterations: u64) -> u64 {
    let mut acc = FNV_OFFSET;
    for i in 0..iterations {
        acc = std::hint::black_box(acc.rotate_left(5) ^ i).wrapping_mul(FNV_PRIME);
    }
    acc
}

fn fold_order(order: &[u64]) -> u64 {
    order
        .iter()
        .fold(FNV_OFFSET, |h, &t| (h ^ t).wrapping_mul(FNV_PRIME))
}

/// Run the workers. `step` is one read-modify-write of the build's counter; `total` reads it at the end.
pub fn run(step: &(dyn Fn() + Sync), total: &dyn Fn() -> u64) {
    let finished = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for worker in 0..THREADS {
            let finished = &finished;
            scope.spawn(move || {
                // A refused name only costs the stable key; the worker carries on under its claim-order key.
                let _ = hairspring_sdk::thread("worker", worker as u32);
                sometimes!(
                    true,
                    "rust.all_started",
                    "every worker started",
                    worker as i64
                );
                for _ in 0..ITERATIONS {
                    step();
                    hairspring_sdk::work(1);
                }
                finished
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(worker);
            });
        }
    });
    let expected = THREADS * ITERATIONS;
    let final_value = total();
    always!(
        final_value == expected,
        "rust.counter_exact",
        "no update was lost",
        final_value as i64,
        expected as i64
    );
    let order = fold_order(&finished.lock().unwrap_or_else(|e| e.into_inner()));
    let random = std::collections::hash_map::RandomState::new().hash_one(0u64);
    println!("final={final_value:020} order={order:016x} rnd={random:016x}");
}

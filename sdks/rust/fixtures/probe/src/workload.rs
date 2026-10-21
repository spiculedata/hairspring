//! The workload: four `std::thread` workers, a `Mutex<Vec>` that records who took the lock first, a relaxed `AtomicU64`
//! read-modify-write with no lock, an `mpsc` channel that records who finished first, and a `HashMap` whose iteration
//! order depends on the standard library's random state. Only `std` is used.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::atomic::{
    AtomicBool, AtomicU64,
    Ordering::{Relaxed, SeqCst},
};
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::time::Instant;

pub const THREADS: u64 = 4;
pub const ROUNDS: u64 = 200;
pub const SPIN_ROUNDS: u64 = 1_000;
pub const MAP_ENTRIES: u64 = 32;

#[derive(Debug, PartialEq, Eq)]
pub struct Invariant(pub &'static str);

#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub lock_order: Vec<u64>,
    pub arrival: Vec<u64>,
    /// The racy counter at the end. Not printed (the line has six fields); the tests read it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub racy_final: u64,
    pub elapsed_ns: u64,
    pub map_order: Vec<u64>,
    pub rand: u64,
}

const SPIN_MULTIPLIER: u64 = 0x5851_f42d_4c95_7f2d;
/// A fixed non-zero start, so the accumulator never sticks at zero.
const SPIN_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// A busy loop with an opaque accumulator, so the optimiser can neither remove it nor merge the racy load and store.
#[inline(never)]
pub fn spin(rounds: u64) -> u64 {
    let mut acc = SPIN_SEED;
    for i in 0..rounds {
        acc = std::hint::black_box(acc.rotate_left(7) ^ i).wrapping_mul(SPIN_MULTIPLIER);
    }
    acc
}

/// The keys of a map of `MAP_ENTRIES` entries in iteration order: a function of the hasher's keys alone.
pub fn map_order<S: BuildHasher>(hasher: S) -> Vec<u64> {
    let mut map = HashMap::with_hasher(hasher);
    for key in 0..MAP_ENTRIES {
        map.insert(key, key);
    }
    map.keys().copied().collect()
}

/// State shared by the workers.
struct Shared {
    lock_order: Mutex<Vec<u64>>,
    racy: AtomicU64,
    started: AtomicU64,
    /// Set when a worker could not be started or died, so the others stop waiting.
    abort: AtomicBool,
}

impl Shared {
    fn wait_for_all(&self) {
        while self.started.load(SeqCst) < THREADS && !self.abort.load(SeqCst) {
            std::thread::yield_now();
        }
    }

    fn worker(&self, id: u64, finished: Sender<u64>) {
        let _abort_if_panicking = AbortOnPanic(&self.abort);
        self.started.fetch_add(1, SeqCst);
        self.wait_for_all();
        if self.abort.load(SeqCst) {
            return;
        }
        self.lock_order
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(id);
        for _ in 0..ROUNDS {
            // The racy read-modify-write: two separate operations with a busy loop between them.
            let seen = self.racy.load(Relaxed);
            std::hint::black_box(spin(SPIN_ROUNDS));
            self.racy.store(seen + 1, Relaxed);
        }
        // The order of arrival at the channel is the order the workers finished in.
        let _ = finished.send(id);
    }
}

/// Sets the abort flag when dropped during a panic.
struct AbortOnPanic<'a>(&'a AtomicBool);

impl Drop for AbortOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, SeqCst);
        }
    }
}

/// Run the four workers to completion and collect what the line reports.
pub fn run() -> Result<Outcome, Invariant> {
    let shared = Shared {
        lock_order: Mutex::new(Vec::new()),
        racy: AtomicU64::new(0),
        started: AtomicU64::new(0),
        abort: AtomicBool::new(false),
    };
    let (sender, receiver) = mpsc::channel();
    let started_at = Instant::now();
    let joined = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for id in 0..THREADS {
            let shared = &shared;
            let finished = sender.clone();
            let spawned = std::thread::Builder::new()
                .spawn_scoped(scope, move || shared.worker(id, finished));
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(_) => {
                    shared.abort.store(true, SeqCst);
                    break;
                }
            }
        }
        let spawned = handles.len() as u64;
        // Join every handle before looking at the results: an unjoined panicked thread would make the scope panic.
        let results: Vec<bool> = handles.into_iter().map(|h| h.join().is_ok()).collect();
        spawned == THREADS && results.iter().all(|&ok| ok)
    });
    let elapsed = started_at.elapsed();
    drop(sender);
    if !joined {
        return Err(Invariant("a worker thread failed to start or panicked"));
    }
    let arrival: Vec<u64> = receiver.try_iter().collect();
    let lock_order = shared
        .lock_order
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if arrival.len() as u64 != THREADS || lock_order.len() as u64 != THREADS {
        return Err(Invariant("a worker did not take the lock and finish"));
    }
    Ok(Outcome {
        lock_order,
        arrival,
        racy_final: shared.racy.load(SeqCst),
        elapsed_ns: u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
        map_order: map_order(RandomState::new()),
        rand: RandomState::new().hash_one(0u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::BuildHasherDefault;

    type FixedState = BuildHasherDefault<std::collections::hash_map::DefaultHasher>;

    fn sorted(items: &[u64]) -> Vec<u64> {
        let mut copy = items.to_vec();
        copy.sort_unstable();
        copy
    }

    #[test]
    fn spin_is_deterministic_and_depends_on_its_count() {
        assert_eq!(spin(100), spin(100));
        assert_ne!(spin(100), spin(101));
        assert_ne!(spin(0), spin(1));
    }

    #[test]
    fn the_map_order_with_a_fixed_hasher_is_the_same_every_time() {
        assert_eq!(
            map_order(FixedState::default()),
            map_order(FixedState::default())
        );
    }

    #[test]
    fn the_map_order_holds_every_key_once_whatever_the_hasher() {
        let order = map_order(std::collections::hash_map::RandomState::new());
        assert_eq!(sorted(&order), (0..MAP_ENTRIES).collect::<Vec<_>>());
    }

    #[test]
    fn a_run_takes_the_lock_and_finishes_once_per_thread() {
        let outcome = run().expect("the workers run");
        let all: Vec<u64> = (0..THREADS).collect();
        assert_eq!(sorted(&outcome.lock_order), all);
        assert_eq!(sorted(&outcome.arrival), all);
    }

    #[test]
    fn a_run_counts_at_least_one_and_at_most_every_round() {
        let outcome = run().expect("the workers run");
        assert!(
            (1..=THREADS * ROUNDS).contains(&outcome.racy_final),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_run_reports_a_nonzero_elapsed_time_and_the_whole_map() {
        let outcome = run().expect("the workers run");
        assert!(outcome.elapsed_ns > 0);
        assert_eq!(
            sorted(&outcome.map_order),
            (0..MAP_ENTRIES).collect::<Vec<_>>()
        );
    }
}

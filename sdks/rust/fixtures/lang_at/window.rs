//! The workers: a start barrier, then `ITERATIONS` rounds of load, `spin`, store on one shared counter.

use std::sync::atomic::{
    AtomicBool, AtomicU64,
    Ordering::{Relaxed, SeqCst},
};
use std::sync::Mutex;

use super::line::{FNV_OFFSET, FNV_PRIME};
use super::mode::{Mode, ITERATIONS, THREADS};

/// An internal invariant failed (a thread did not start or finish, the arrival list is incomplete). Exit status 3.
#[derive(Debug, PartialEq, Eq)]
pub struct Invariant(pub &'static str);

/// What the main thread reads after the workers are joined.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub final_value: u64,
    pub started: u64,
    /// Worker numbers in the order they finished.
    pub arrival: Vec<u64>,
}

/// State shared by the workers. Its fields are atomics or a lock, so sharing needs no mutation through `&mut`.
pub struct Shared {
    counter: AtomicU64,
    started: AtomicU64,
    arrived: AtomicU64,
    slots: [AtomicU64; THREADS as usize],
    sink: AtomicU64,
    lock: Mutex<()>,
    /// Set when a worker could not be started, so the others stop waiting at the barrier.
    abort: AtomicBool,
    /// Workers told to panic (test hook; not part of the shipped binary).
    #[cfg(test)]
    panicking: Vec<u64>,
}

impl Shared {
    pub fn new() -> Self {
        Shared {
            counter: AtomicU64::new(0),
            started: AtomicU64::new(0),
            arrived: AtomicU64::new(0),
            slots: [(); THREADS as usize].map(|()| AtomicU64::new(0)),
            sink: AtomicU64::new(0),
            lock: Mutex::new(()),
            abort: AtomicBool::new(false),
            #[cfg(test)]
            panicking: Vec::new(),
        }
    }

    /// Record that `worker` finished; each call takes the next slot (stored as `worker + 1`, so 0 means empty).
    pub fn record_arrival(&self, worker: u64) {
        let slot = self.arrived.fetch_add(1, SeqCst) as usize;
        if let Some(cell) = self.slots.get(slot) {
            cell.store(worker + 1, SeqCst);
        }
    }

    /// The arrival list, or the invariant that failed.
    pub fn arrival(&self) -> Result<Vec<u64>, Invariant> {
        let listed: Vec<u64> = self.slots.iter().map(|s| s.load(SeqCst)).collect();
        if self.arrived.load(SeqCst) != THREADS || listed.contains(&0) {
            return Err(Invariant("the arrival list does not hold four entries"));
        }
        Ok(listed.into_iter().map(|w| w - 1).collect())
    }

    /// One step of the window: load, spin, store. Two separate operations, never a read-modify-write, and nothing
    /// else between them (no system call, allocation, lock, clock read or SDK call).
    fn step(&self, window: u64) -> u64 {
        let seen = self.counter.load(Relaxed);
        let acc = spin(window);
        self.counter.store(seen + 1, Relaxed);
        acc
    }

    /// Wait until all workers have started, or the run is being abandoned.
    fn wait_for_all(&self) {
        while self.started.load(SeqCst) < THREADS && !self.abort.load(SeqCst) {
            std::thread::yield_now();
        }
    }

    fn worker(&self, mode: Mode, id: u64) {
        // If this worker dies for any reason, the others must not wait for it at the barrier.
        let _abort_if_panicking = AbortOnPanic(&self.abort);
        // Test hook: panic at once, before registering as started.
        #[cfg(test)]
        if self.panicking.contains(&id) {
            panic!("worker {id} panics on request");
        }
        // A refused name only costs the stable key; the worker carries on under its claim-order key.
        let _ = hairspring_sdk::thread("worker", id as u32);
        self.started.fetch_add(1, SeqCst);
        self.wait_for_all();
        if self.abort.load(SeqCst) {
            return;
        }
        for _ in 0..ITERATIONS {
            let acc = if mode.is_locked() {
                let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
                self.step(mode.window())
            } else {
                self.step(mode.window())
            };
            // Outside the window: the result goes to a sink so the loop cannot be removed.
            self.sink.store(acc, Relaxed);
            hairspring_sdk::work(1);
        }
        self.record_arrival(id);
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

/// A busy loop with an opaque accumulator in the fixture's own source, so the optimiser can neither remove it nor
/// move the counter's load and store across it.
#[inline(never)]
pub fn spin(iterations: u64) -> u64 {
    let mut acc = FNV_OFFSET;
    for i in 0..iterations {
        acc = std::hint::black_box(acc.rotate_left(5) ^ i).wrapping_mul(FNV_PRIME);
    }
    acc
}

/// Run the four workers of `mode` to completion.
pub fn run_all(mode: Mode) -> Result<Outcome, Invariant> {
    run_shared(Shared::new(), mode)
}

/// `run_all`, with the workers numbered in `panicking` told to panic (compiled in tests only).
#[cfg(test)]
fn run_with(mode: Mode, panicking: &[u64]) -> Result<Outcome, Invariant> {
    let shared = Shared {
        panicking: panicking.to_vec(),
        ..Shared::new()
    };
    run_shared(shared, mode)
}

fn run_shared(shared: Shared, mode: Mode) -> Result<Outcome, Invariant> {
    let joined = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for id in 0..THREADS {
            let shared = &shared;
            let spawned =
                std::thread::Builder::new().spawn_scoped(scope, move || shared.worker(mode, id));
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
        let all_joined = results.iter().all(|&ok| ok);
        spawned == THREADS && all_joined
    });
    if !joined {
        return Err(Invariant("a worker thread failed to start or panicked"));
    }
    Ok(Outcome {
        final_value: shared.counter.load(SeqCst),
        started: shared.started.load(SeqCst),
        arrival: shared.arrival()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spin_is_deterministic_and_depends_on_its_count() {
        assert_eq!(spin(100), spin(100));
        assert_ne!(spin(100), spin(101));
        assert_ne!(spin(0), spin(1));
    }

    #[test]
    fn arrivals_fill_slots_in_order() {
        let shared = Shared::new();
        for w in [2, 0, 3, 1] {
            shared.record_arrival(w);
        }
        assert_eq!(shared.arrival(), Ok(vec![2, 0, 3, 1]));
    }

    #[test]
    fn an_incomplete_arrival_list_is_an_invariant_failure() {
        let shared = Shared::new();
        shared.record_arrival(0);
        shared.record_arrival(1);
        assert!(shared.arrival().is_err());
    }

    #[test]
    fn the_fixed_mode_never_loses_an_update() {
        for _ in 0..10 {
            let out = run_all(Mode::Fixed).expect("workers run");
            assert_eq!(out.final_value, 4_000);
            assert_eq!(out.started, 4);
            let mut sorted = out.arrival.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, [0, 1, 2, 3]);
        }
    }

    #[test]
    fn two_workers_that_panic_before_the_barrier_are_an_invariant_failure_not_a_hang_or_a_panic() {
        assert!(run_with(Mode::Fixed, &[1, 2]).is_err());
    }

    #[test]
    fn one_worker_that_panics_is_an_invariant_failure() {
        assert!(run_with(Mode::Buggy, &[3]).is_err());
    }

    #[test]
    fn the_buggy_mode_never_counts_more_than_the_iterations() {
        let out = run_all(Mode::Buggy).expect("workers run");
        assert!((1..=4_000).contains(&out.final_value), "{out:?}");
        assert_eq!(out.started, 4);
    }
}

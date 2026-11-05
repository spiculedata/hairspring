//! A bounded, in-memory cache of results worked out from a run's stored rows.
//!
//! Each entry is kept with a validity token: the thing the caller read from the store that proves the rows behind the
//! result have not changed (for a finished run, its whole run row). An entry is served only to a caller holding an
//! equal token; any other token replaces it. Entries are dropped least recently used first, so the cache never holds
//! more than its entry and byte bounds.
//!
//! The first caller for a key works the result out (a flight) without holding any lock; later callers for the same key
//! and token wait for that flight and are handed its outcome, so concurrent first reads do the work, and hold its
//! memory, once. A flight that fails hands its error to every caller waiting for it. A result that may not be kept is
//! handed to them when it is right for the token they hold ([`Keep::Share`]); otherwise ([`Keep::No`]) each works its
//! own out, side by side, never one after another.
use crate::{Result, ServiceError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// How much a cache may hold.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    pub entries: usize,
    /// Estimated bytes of the values held, from [`Weigh`].
    pub bytes: usize,
}

/// An estimate of the heap and inline bytes a cached value holds.
pub(crate) trait Weigh {
    fn weight(&self) -> usize;
}

/// Whether a result may be kept, and who else may be given it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keep {
    /// Kept, and served to every later caller with an equal token.
    Yes,
    /// Not kept, because the caller cannot prove the rows held still while it read them, but right for every caller
    /// that held the same token while it was being worked out: those callers are given it.
    Share,
    /// Not kept and given to no one else: callers that were waiting for it work their own out.
    No,
}

/// A result, and whether it may be kept.
pub(crate) struct Computed<V> {
    pub value: V,
    pub keep: Keep,
}

impl<V> Computed<V> {
    pub(crate) fn kept(value: V) -> Computed<V> {
        Computed {
            value,
            keep: Keep::Yes,
        }
    }
}

/// What a flight hands the callers that waited for it.
enum Outcome<V> {
    Value(Arc<V>),
    Failed(ServiceError),
    /// Work it out yourself: the flight's result was right for no one else, or the flight never finished.
    OwnWay,
}

impl<V> Clone for Outcome<V> {
    fn clone(&self) -> Self {
        match self {
            Outcome::Value(v) => Outcome::Value(v.clone()),
            Outcome::Failed(e) => Outcome::Failed(e.clone()),
            Outcome::OwnWay => Outcome::OwnWay,
        }
    }
}

/// One working out of a value, and the callers waiting for it.
struct Flight<V> {
    outcome: Mutex<Option<Outcome<V>>>,
    landed: Condvar,
    waiting: AtomicUsize,
}

impl<V> Flight<V> {
    fn new() -> Flight<V> {
        Flight {
            outcome: Mutex::new(None),
            landed: Condvar::new(),
            waiting: AtomicUsize::new(0),
        }
    }

    fn wait(&self) -> Outcome<V> {
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let mut outcome = lock(&self.outcome);
        loop {
            if let Some(o) = outcome.as_ref() {
                return o.clone();
            }
            outcome = self.landed.wait(outcome).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn land(&self, outcome: Outcome<V>) {
        *lock(&self.outcome) = Some(outcome);
        self.landed.notify_all();
    }
}

enum State<V> {
    Idle,
    Running(Arc<Flight<V>>),
    Ready(Arc<V>),
}

struct Slot<V> {
    state: Mutex<State<V>>,
}

struct Entry<T, V> {
    token: T,
    slot: Arc<Slot<V>>,
    used: u64,
    bytes: usize,
}

struct Inner<T, V> {
    entries: HashMap<String, Entry<T, V>>,
    clock: u64,
    bytes: usize,
}

pub(crate) struct ResultCache<T, V> {
    inner: Mutex<Inner<T, V>>,
    bounds: Bounds,
    computed: AtomicU64,
}

fn lock<X>(m: &Mutex<X>) -> MutexGuard<'_, X> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The caller working out a flight's value. Dropped without [`Lead::finish`] (the computation panicked), it tells the
/// waiting callers to work their own out and leaves nothing behind.
struct Lead<'a, T: PartialEq + Clone, V: Weigh> {
    cache: &'a ResultCache<T, V>,
    key: &'a str,
    slot: &'a Arc<Slot<V>>,
    finished: bool,
}

impl<T: PartialEq + Clone, V: Weigh> Lead<'_, T, V> {
    /// Ends the flight: keeps `keep` in the slot (or drops the slot), then hands `outcome` to the waiting callers.
    fn finish(&mut self, outcome: Outcome<V>, keep: Option<Arc<V>>) {
        self.finished = true;
        let weight = keep.as_ref().map(|v| v.weight());
        if keep.is_none() {
            self.cache.remove_slot(self.key, self.slot);
        }
        let next = match keep {
            Some(v) => State::Ready(v),
            None => State::Idle,
        };
        let flight = match std::mem::replace(&mut *lock(&self.slot.state), next) {
            State::Running(f) => Some(f),
            _ => None,
        };
        if let Some(bytes) = weight {
            self.cache.settle(self.key, self.slot, bytes);
        }
        if let Some(f) = flight {
            f.land(outcome);
        }
    }
}

impl<T: PartialEq + Clone, V: Weigh> Drop for Lead<'_, T, V> {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(Outcome::OwnWay, None);
        }
    }
}

impl<T: PartialEq + Clone, V: Weigh> ResultCache<T, V> {
    pub(crate) fn new(bounds: Bounds) -> ResultCache<T, V> {
        ResultCache {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                clock: 0,
                bytes: 0,
            }),
            bounds,
            computed: AtomicU64::new(0),
        }
    }

    /// The value for `key` worked out under `token`, from memory when an entry with an equal token is held, else from
    /// the flight already working it out, else from `compute`. A failed or unkept computation leaves nothing behind.
    pub(crate) fn get_or_compute(
        &self,
        key: &str,
        token: &T,
        compute: impl FnOnce() -> Result<Computed<V>>,
    ) -> Result<Arc<V>> {
        let slot = self.slot(key, token);
        let joined = {
            let mut state = lock(&slot.state);
            match &*state {
                State::Ready(value) => return Ok(value.clone()),
                State::Running(flight) => Some(flight.clone()),
                State::Idle => {
                    *state = State::Running(Arc::new(Flight::new()));
                    None
                }
            }
        };
        let Some(flight) = joined else {
            return self.lead(key, &slot, compute);
        };
        match flight.wait() {
            Outcome::Value(value) => Ok(value),
            Outcome::Failed(e) => Err(e),
            Outcome::OwnWay => {
                self.computed.fetch_add(1, Ordering::Relaxed);
                compute().map(|c| Arc::new(c.value))
            }
        }
    }

    /// Works out the value of the flight just started in `slot`, and hands the outcome to the callers waiting for it.
    fn lead(
        &self,
        key: &str,
        slot: &Arc<Slot<V>>,
        compute: impl FnOnce() -> Result<Computed<V>>,
    ) -> Result<Arc<V>> {
        let mut lead = Lead {
            cache: self,
            key,
            slot,
            finished: false,
        };
        self.computed.fetch_add(1, Ordering::Relaxed);
        let Computed { value, keep } = match compute() {
            Ok(computed) => computed,
            Err(e) => {
                lead.finish(Outcome::Failed(e.clone()), None);
                return Err(e);
            }
        };
        let value = Arc::new(value);
        match keep {
            Keep::Yes => lead.finish(Outcome::Value(value.clone()), Some(value.clone())),
            Keep::Share => lead.finish(Outcome::Value(value.clone()), None),
            Keep::No => lead.finish(Outcome::OwnWay, None),
        }
        Ok(value)
    }

    /// Drops whatever is held for `key`.
    pub(crate) fn forget(&self, key: &str) {
        let mut inner = lock(&self.inner);
        if let Some(old) = inner.entries.remove(key) {
            inner.bytes -= old.bytes;
        }
    }

    /// How many times a value was worked out rather than served from memory or handed over by a flight.
    pub(crate) fn computed(&self) -> u64 {
        self.computed.load(Ordering::Relaxed)
    }

    /// The slot for `key` under `token`: the held one when its token is equal, otherwise a new, empty one that takes
    /// the place of any entry made under another token.
    fn slot(&self, key: &str, token: &T) -> Arc<Slot<V>> {
        let mut inner = lock(&self.inner);
        inner.clock += 1;
        let now = inner.clock;
        if let Some(entry) = inner.entries.get_mut(key) {
            if entry.token == *token {
                entry.used = now;
                return entry.slot.clone();
            }
        }
        let slot = Arc::new(Slot {
            state: Mutex::new(State::Idle),
        });
        let entry = Entry {
            token: token.clone(),
            slot: slot.clone(),
            used: now,
            bytes: 0,
        };
        if let Some(old) = inner.entries.insert(key.to_owned(), entry) {
            inner.bytes -= old.bytes;
        }
        self.evict(&mut inner);
        slot
    }

    /// Records the size of a value just stored in `slot`, if the slot is still the entry for `key`, and drops the
    /// least recently used entries until the cache is within its bounds. A value larger than the whole byte bound is
    /// not kept, and costs no other entry its place.
    fn settle(&self, key: &str, slot: &Arc<Slot<V>>, bytes: usize) {
        let mut inner = lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(key) else {
            return;
        };
        if !Arc::ptr_eq(&entry.slot, slot) {
            return;
        }
        if bytes > self.bounds.bytes {
            inner.entries.remove(key);
            return;
        }
        entry.bytes = bytes;
        inner.bytes += bytes;
        self.evict(&mut inner);
    }

    fn remove_slot(&self, key: &str, slot: &Arc<Slot<V>>) {
        let mut inner = lock(&self.inner);
        if inner
            .entries
            .get(key)
            .is_some_and(|e| Arc::ptr_eq(&e.slot, slot))
        {
            if let Some(old) = inner.entries.remove(key) {
                inner.bytes -= old.bytes;
            }
        }
    }

    fn evict(&self, inner: &mut Inner<T, V>) {
        while inner.entries.len() > self.bounds.entries || inner.bytes > self.bounds.bytes {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                return;
            };
            if let Some(old) = inner.entries.remove(&oldest) {
                inner.bytes -= old.bytes;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn held(&self) -> (usize, usize) {
        let inner = lock(&self.inner);
        (inner.entries.len(), inner.bytes)
    }

    /// How many callers are waiting for the flight working out `key`.
    #[cfg(test)]
    pub(crate) fn waiting(&self, key: &str) -> usize {
        let Some(slot) = lock(&self.inner).entries.get(key).map(|e| e.slot.clone()) else {
            return 0;
        };
        let state = lock(&slot.state);
        match &*state {
            State::Running(f) => f.waiting.load(Ordering::SeqCst),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::time::{Duration, Instant};

    struct Blob(usize);

    impl Weigh for Blob {
        fn weight(&self) -> usize {
            self.0
        }
    }

    fn cache(entries: usize, bytes: usize) -> ResultCache<u64, Blob> {
        ResultCache::new(Bounds { entries, bytes })
    }

    fn put(c: &ResultCache<u64, Blob>, key: &str, token: u64, size: usize) -> usize {
        c.get_or_compute(key, &token, || Ok(Computed::kept(Blob(size))))
            .unwrap()
            .0
    }

    /// Waits, without sleeping, until `cond` holds; false if it has not held within a generous bound, so a test whose
    /// callers are wrongly made to wait for each other fails instead of hanging.
    fn until(cond: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cond() {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::yield_now();
        }
        true
    }

    const READERS: usize = 8;

    /// `READERS` callers ask for key "r" under one token at once; `compute` is told whether it is the first call. The
    /// first call returns only once every other caller is waiting for its flight.
    fn concurrent(
        c: &ResultCache<u64, Blob>,
        compute: impl Fn(bool) -> Result<Computed<Blob>> + Sync,
    ) -> Vec<Result<usize>> {
        let calls = AtomicUsize::new(0);
        let gate = Barrier::new(READERS);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..READERS)
                .map(|_| {
                    s.spawn(|| {
                        gate.wait();
                        c.get_or_compute("r", &1, || {
                            let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
                            if first {
                                assert!(
                                    until(|| c.waiting("r") == READERS - 1),
                                    "the others never joined the flight"
                                );
                            }
                            compute(first)
                        })
                        .map(|v| v.0)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        })
    }

    #[test]
    fn a_value_is_served_from_memory_while_its_token_is_unchanged() {
        let c = cache(4, 1000);
        assert_eq!(put(&c, "r", 1, 10), 10);
        assert_eq!(put(&c, "r", 1, 99), 10, "held, not worked out again");
        assert_eq!(c.computed(), 1);
    }

    #[test]
    fn another_token_replaces_the_entry() {
        let c = cache(4, 1000);
        put(&c, "r", 1, 10);
        assert_eq!(put(&c, "r", 2, 20), 20);
        assert_eq!(c.held(), (1, 20));
        assert_eq!(put(&c, "r", 1, 30), 30, "the old token's value is gone");
        assert_eq!(c.computed(), 3);
    }

    #[test]
    fn a_value_that_may_not_be_kept_or_could_not_be_worked_out_leaves_nothing() {
        let c = cache(4, 1000);
        for keep in [Keep::Share, Keep::No] {
            let v = c
                .get_or_compute("r", &1, || {
                    Ok(Computed {
                        value: Blob(5),
                        keep,
                    })
                })
                .unwrap();
            assert_eq!(v.0, 5);
            assert_eq!(c.held(), (0, 0));
        }
        let err = c.get_or_compute("r", &1, || Err(ServiceError::Internal("x".into())));
        assert!(err.is_err());
        assert_eq!(c.held(), (0, 0));
        assert_eq!(put(&c, "r", 1, 7), 7);
    }

    #[test]
    fn the_cache_never_holds_more_entries_or_bytes_than_its_bounds() {
        let c = cache(8, 100);
        for i in 0..200u64 {
            put(&c, &format!("r{i}"), i, (i as usize % 30) + 1);
            let (entries, bytes) = c.held();
            assert!(entries <= 8, "{entries} entries");
            assert!(bytes <= 100, "{bytes} bytes");
        }
    }

    #[test]
    fn the_least_recently_used_entry_goes_first() {
        let c = cache(2, 1000);
        put(&c, "a", 1, 1);
        put(&c, "b", 1, 1);
        put(&c, "a", 1, 1);
        put(&c, "c", 1, 1);
        assert_eq!(c.computed(), 3);
        put(&c, "a", 1, 1);
        assert_eq!(c.computed(), 3, "a was used last, so b went");
        put(&c, "b", 1, 1);
        assert_eq!(c.computed(), 4);
    }

    #[test]
    fn a_value_larger_than_the_byte_bound_is_not_kept_and_costs_no_other_entry() {
        let c = cache(4, 100);
        put(&c, "small", 1, 40);
        assert_eq!(put(&c, "huge", 1, 101), 101);
        assert_eq!(c.held(), (1, 40));
        put(&c, "small", 1, 40);
        assert_eq!(c.computed(), 2, "the small entry stayed");
    }

    #[test]
    fn forgetting_a_key_drops_its_entry_and_its_bytes() {
        let c = cache(4, 100);
        put(&c, "r", 1, 40);
        c.forget("r");
        assert_eq!(c.held(), (0, 0));
        c.forget("r");
        assert_eq!(put(&c, "r", 1, 41), 41);
    }

    #[test]
    fn readers_waiting_on_one_key_share_the_first_readers_result() {
        let c = cache(4, 1000);
        let got = concurrent(&c, |_| Ok(Computed::kept(Blob(3))));
        assert!(got.iter().all(|v| matches!(v, Ok(3))));
        assert_eq!(c.computed(), 1);
        assert_eq!(c.held(), (1, 3));
    }

    #[test]
    fn a_failure_is_handed_to_every_waiting_reader_without_working_it_out_again() {
        let c = cache(4, 1000);
        let got = concurrent(&c, |_| Err(ServiceError::Internal("store down".into())));
        for v in &got {
            assert!(
                matches!(v, Err(ServiceError::Internal(m)) if m == "store down"),
                "{:?}",
                v.as_ref().map(|_| ())
            );
        }
        assert_eq!(c.computed(), 1, "one computation for all eight readers");
        assert_eq!(c.held(), (0, 0));
    }

    #[test]
    fn an_unkept_result_right_for_the_token_is_handed_to_every_waiting_reader() {
        let c = cache(4, 1000);
        let got = concurrent(&c, |_| {
            Ok(Computed {
                value: Blob(4),
                keep: Keep::Share,
            })
        });
        assert!(got.iter().all(|v| matches!(v, Ok(4))));
        assert_eq!(c.computed(), 1, "one computation for all eight readers");
        assert_eq!(c.held(), (0, 0), "and nothing kept");
    }

    #[test]
    fn readers_given_no_result_work_theirs_out_side_by_side() {
        let c = cache(4, 1000);
        // Each waiting reader's computation returns only once all of them are under way: callers made to work theirs
        // out one after another never all get there.
        let under_way = AtomicUsize::new(0);
        let got = concurrent(&c, |first| {
            if first {
                return Ok(Computed {
                    value: Blob(1),
                    keep: Keep::No,
                });
            }
            under_way.fetch_add(1, Ordering::SeqCst);
            let together = until(|| under_way.load(Ordering::SeqCst) == READERS - 1);
            Ok(Computed {
                value: Blob(if together { 2 } else { 0 }),
                keep: Keep::Yes,
            })
        });
        let mut sizes: Vec<usize> = got.into_iter().map(Result::unwrap).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, [1, 2, 2, 2, 2, 2, 2, 2]);
        assert_eq!(c.computed(), READERS as u64);
        assert_eq!(c.held(), (0, 0), "results worked out alone are not kept");
    }

    #[test]
    fn a_computation_that_panics_leaves_waiting_readers_to_work_theirs_out() {
        let c = cache(4, 1000);
        let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            concurrent(&c, |first| {
                if first {
                    panic!("computation panicked");
                }
                Ok(Computed::kept(Blob(6)))
            })
        }));
        // The panicking reader's thread fails the scope; the others finished with their own result.
        assert!(got.is_err());
        assert_eq!(c.computed(), READERS as u64);
        assert_eq!(c.held(), (0, 0));
        assert_eq!(put(&c, "r", 1, 9), 9);
    }
}

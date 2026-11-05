//! A bounded, in-memory cache of results worked out from a run's stored rows.
//!
//! Each entry is kept with a validity token: the thing the caller read from the store that proves the rows behind the
//! result have not changed (for a finished run, its whole run row). An entry is served only to a caller holding an
//! equal token; any other token replaces it. Entries are dropped least recently used first, so the cache never holds
//! more than its entry and byte bounds. The first caller for a key works the result out while later callers for the
//! same key and token wait for it, so concurrent first reads do the work, and hold its memory, once.
use crate::Result;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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

/// A result, and whether it may be kept: a caller that cannot prove the rows held still while it read them says no.
pub(crate) struct Computed<V> {
    pub value: V,
    pub keep: bool,
}

impl<V> Computed<V> {
    pub(crate) fn kept(value: V) -> Computed<V> {
        Computed { value, keep: true }
    }
}

struct Slot<V> {
    value: Mutex<Option<Arc<V>>>,
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
    /// `compute`. A failed or unkept computation leaves nothing behind.
    pub(crate) fn get_or_compute(
        &self,
        key: &str,
        token: &T,
        compute: impl FnOnce() -> Result<Computed<V>>,
    ) -> Result<Arc<V>> {
        let slot = self.slot(key, token);
        let mut held = lock(&slot.value);
        if let Some(value) = held.as_ref() {
            return Ok(value.clone());
        }
        self.computed.fetch_add(1, Ordering::Relaxed);
        let Computed { value, keep } = match compute() {
            Ok(computed) => computed,
            Err(e) => {
                drop(held);
                self.remove_slot(key, &slot);
                return Err(e);
            }
        };
        let value = Arc::new(value);
        if !keep {
            drop(held);
            self.remove_slot(key, &slot);
            return Ok(value);
        }
        *held = Some(value.clone());
        drop(held);
        self.settle(key, &slot, value.weight());
        Ok(value)
    }

    /// Drops whatever is held for `key`.
    pub(crate) fn forget(&self, key: &str) {
        let mut inner = lock(&self.inner);
        if let Some(old) = inner.entries.remove(key) {
            inner.bytes -= old.bytes;
        }
    }

    /// How many times a value was worked out rather than served from memory.
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
            value: Mutex::new(None),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceError;

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
        let v = c
            .get_or_compute("r", &1, || {
                Ok(Computed {
                    value: Blob(5),
                    keep: false,
                })
            })
            .unwrap();
        assert_eq!(v.0, 5);
        assert_eq!(c.held(), (0, 0));
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
        let c = Arc::new(cache(4, 1000));
        let gate = Arc::new(std::sync::Barrier::new(8));
        let sizes: Vec<usize> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let (c, gate) = (c.clone(), gate.clone());
                    s.spawn(move || {
                        gate.wait();
                        c.get_or_compute("r", &1, || {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            Ok(Computed::kept(Blob(i + 1)))
                        })
                        .unwrap()
                        .0
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(c.computed(), 1);
        assert!(sizes.windows(2).all(|w| w[0] == w[1]));
    }
}

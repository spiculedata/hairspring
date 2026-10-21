//! A small cache of coverage series for finished runs.
//!
//! The series is a scan of every trial of a run. A finished run's trial rows no longer change, so its series is
//! worked out when its job finishes and kept in the store; this cache holds the ones read recently (and the series
//! of runs finished before the store kept them, which are scanned on first read). Runs that are still open are never
//! cached.
use crate::view::Coverage;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Most series held at once. A series is at most a few thousand small points, so this bounds the cache to a
/// few megabytes.
pub(crate) const MAX_CACHED_SERIES: usize = 64;

#[derive(Default)]
pub(crate) struct CoverageCache {
    series: Mutex<HashMap<String, Arc<Coverage>>>,
    scans: AtomicU64,
}

impl CoverageCache {
    /// Records that a series was worked out from the stored trials rather than served from memory.
    pub(crate) fn note_scan(&self) {
        self.scans.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn scans(&self) -> u64 {
        self.scans.load(Ordering::Relaxed)
    }

    pub(crate) fn get(&self, run_id: &str) -> Option<Arc<Coverage>> {
        self.lock().get(run_id).cloned()
    }

    /// Remembers a series. When the cache is full it starts again empty: the series are cheap to rebuild and this
    /// keeps the policy trivial.
    pub(crate) fn insert(&self, run_id: &str, coverage: Arc<Coverage>) {
        let mut series = self.lock();
        if series.len() >= MAX_CACHED_SERIES && !series.contains_key(run_id) {
            series.clear();
        }
        series.insert(run_id.to_owned(), coverage);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Coverage>>> {
        self.series.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(run_id: &str) -> Arc<Coverage> {
        Arc::new(Coverage {
            run_id: run_id.to_owned(),
            trials: 0,
            downsampled: false,
            points: Vec::new(),
        })
    }

    #[test]
    fn a_stored_series_is_returned() {
        let cache = CoverageCache::default();
        cache.insert("r1", series("r1"));
        assert_eq!(cache.get("r1").unwrap().run_id, "r1");
        assert!(cache.get("r2").is_none());
    }

    #[test]
    fn the_cache_never_holds_more_than_its_bound() {
        let cache = CoverageCache::default();
        for i in 0..(MAX_CACHED_SERIES * 3) {
            let id = format!("r{i}");
            cache.insert(&id, series(&id));
            assert!(cache.len() <= MAX_CACHED_SERIES);
        }
        assert!(cache
            .get(&format!("r{}", MAX_CACHED_SERIES * 3 - 1))
            .is_some());
    }
}

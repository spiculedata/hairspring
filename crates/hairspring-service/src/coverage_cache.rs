//! A small cache of coverage series.
//!
//! The series is a scan of every trial of a run. A finished run's trial rows no longer change, so its series is
//! worked out when its job finishes and kept in the store; this cache holds the ones read recently (and the series
//! of runs finished before the store kept them, which are scanned on first read), under the run row they were read
//! with. A run that is still open is cached under its attempt and its count of trial rows: within one attempt rows are
//! only ever added, and a new attempt removes the old rows and raises the attempt, so an equal pair means equal rows.
use crate::result_cache::{Bounds, Keep, ResultCache, Weigh};
use crate::view::{Coverage, CoveragePoint};
use hairspring_store::RunRecord;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A test's hook, run by a coverage read of an open run just after it has counted the run's trial rows.
pub(crate) type CountHook = Arc<dyn Fn() + Send + Sync>;

/// Most series held at once. A series is at most [`crate::query::MAX_COVERAGE_POINTS`] small points (about 64 KB).
pub(crate) const MAX_CACHED_SERIES: usize = 64;
/// Most estimated bytes of series held at once.
pub(crate) const MAX_CACHED_SERIES_BYTES: usize = 16 * 1024 * 1024;

/// What proves a held series is still the run's series.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CoverageToken {
    /// A finished run, by its whole row.
    Finished(Box<RunRecord>),
    /// An open run: its current attempt, and how many trial rows it held.
    Open { attempt: u64, trials: u64 },
}

/// An open run's rows as counted: how many, and the attempt they belong to (`None` once the run has gone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenRows {
    pub trials: u64,
    pub attempt: Option<u64>,
}

/// Whether a series scanned from an open run may be kept under the rows counted `before` the scan: the same attempt and
/// count `after` it, and a scan that saw every counted row. Attempts only go up and rows are only added within one, so
/// the scan then saw exactly the rows counted.
pub(crate) fn held_still(before: OpenRows, after: OpenRows, scanned: u64) -> bool {
    before.attempt.is_some() && before == after && scanned == before.trials
}

/// What may be done with a series scanned from an open run. Kept when the rows [`held_still`]. Otherwise, while the
/// attempt is unchanged, the scan saw every row counted `before` it, plus rows added while it ran, all of which callers
/// holding the same count were waiting through: they are given it, as the scanning caller is. A new attempt (or a run
/// that went) may have changed the rows under the scan, so the waiting callers scan for themselves.
pub(crate) fn open_scan_keep(before: OpenRows, after: OpenRows, scanned: u64) -> Keep {
    if held_still(before, after, scanned) {
        Keep::Yes
    } else if before.attempt.is_some() && before.attempt == after.attempt {
        Keep::Share
    } else {
        Keep::No
    }
}

impl Weigh for Coverage {
    fn weight(&self) -> usize {
        std::mem::size_of::<Coverage>()
            + self.run_id.len()
            + self.points.len() * std::mem::size_of::<CoveragePoint>()
    }
}

pub(crate) struct CoverageCache {
    pub(crate) series: ResultCache<CoverageToken, Coverage>,
    scans: AtomicU64,
    count_hook: Mutex<Option<CountHook>>,
}

impl Default for CoverageCache {
    fn default() -> Self {
        CoverageCache {
            series: ResultCache::new(Bounds {
                entries: MAX_CACHED_SERIES,
                bytes: MAX_CACHED_SERIES_BYTES,
            }),
            scans: AtomicU64::new(0),
            count_hook: Mutex::new(None),
        }
    }
}

impl CoverageCache {
    pub(crate) fn set_count_hook(&self, hook: CountHook) {
        *self.count_hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    /// Runs the test's hook, if one is set; not while holding the lock, so the hook may read coverage itself.
    pub(crate) fn after_count(&self) {
        let hook = self
            .count_hook
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Records that a series was worked out from the stored trials rather than served from memory.
    pub(crate) fn note_scan(&self) {
        self.scans.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn scans(&self) -> u64 {
        self.scans.load(Ordering::Relaxed)
    }

    pub(crate) fn forget(&self, run_id: &str) {
        self.series.forget(run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(trials: u64, attempt: u64) -> OpenRows {
        OpenRows {
            trials,
            attempt: Some(attempt),
        }
    }

    #[test]
    fn a_scan_is_kept_only_when_nothing_came_or_went_while_it_ran() {
        assert!(held_still(rows(5, 1), rows(5, 1), 5));
        assert!(!held_still(rows(5, 1), rows(6, 1), 5), "a row was added");
        assert!(!held_still(rows(5, 1), rows(5, 2), 5), "a new attempt");
        assert!(
            !held_still(rows(5, 1), rows(5, 1), 4),
            "the scan missed a row"
        );
        let gone = OpenRows {
            trials: 5,
            attempt: None,
        };
        assert!(!held_still(rows(5, 1), gone, 5), "the run went");
        assert!(!held_still(gone, gone, 5), "no run to hold");
    }

    #[test]
    fn an_unkept_scan_is_shared_only_while_the_attempt_is_unchanged() {
        assert_eq!(open_scan_keep(rows(5, 1), rows(5, 1), 5), Keep::Yes);
        assert_eq!(open_scan_keep(rows(5, 1), rows(7, 1), 7), Keep::Share);
        assert_eq!(open_scan_keep(rows(5, 1), rows(2, 2), 2), Keep::No);
        let gone = OpenRows {
            trials: 0,
            attempt: None,
        };
        assert_eq!(open_scan_keep(rows(5, 1), gone, 5), Keep::No);
        assert_eq!(open_scan_keep(gone, gone, 0), Keep::No);
    }

    #[test]
    fn a_full_series_weighs_what_its_points_do() {
        let points = crate::query::MAX_COVERAGE_POINTS;
        let series = Coverage {
            run_id: "r1".to_owned(),
            trials: 0,
            downsampled: false,
            points: vec![
                CoveragePoint {
                    seq: 0,
                    ids_hit: 0,
                    ids_true: 0,
                    schedules: 0,
                };
                points
            ],
        };
        let w = series.weight();
        assert!(w >= points * std::mem::size_of::<CoveragePoint>());
        assert!(w * MAX_CACHED_SERIES <= MAX_CACHED_SERIES_BYTES);
    }
}

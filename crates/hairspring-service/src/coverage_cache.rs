//! A small cache of coverage series.
//!
//! The series is a scan of every trial of a run. A finished run's trial rows no longer change, so its series is
//! worked out when its job finishes and kept in the store; this cache holds the ones read recently (and the series
//! of runs finished before the store kept them, which are scanned on first read), under the run row they were read
//! with. A run that is still open is cached under its attempt and its count of trial rows: within one attempt rows are
//! only ever added, and a new attempt removes the old rows and raises the attempt, so an equal pair means equal rows.
use crate::result_cache::{Bounds, ResultCache, Weigh};
use crate::view::{Coverage, CoveragePoint};
use hairspring_store::RunRecord;
use std::sync::atomic::{AtomicU64, Ordering};

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
}

impl Default for CoverageCache {
    fn default() -> Self {
        CoverageCache {
            series: ResultCache::new(Bounds {
                entries: MAX_CACHED_SERIES,
                bytes: MAX_CACHED_SERIES_BYTES,
            }),
            scans: AtomicU64::new(0),
        }
    }
}

impl CoverageCache {
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

//! Reading a run's trials in order, a bounded amount at a time, and asking what a digest says about one property.
use crate::query::SCAN_CAP;
use crate::{Result, Service};
use hairspring_props::trial::failure_basis;
use hairspring_props::TrialDigest;
use hairspring_store::TrialRecord;

/// Trial rows read from the store per call.
const STORE_PAGE: u32 = 500;

/// Whether a scan goes on after the trial it just showed.
pub(crate) enum Flow {
    Continue,
    Stop,
}

/// Where a scan stopped.
pub(crate) struct ScanEnd {
    /// The last trial read; a cursor to continue from.
    pub last_seq: Option<u64>,
    pub scanned: u64,
    /// The scan reached the end of the run.
    pub exhausted: bool,
}

/// What one digest says about one catalog index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Presence {
    /// The trial reported counts for the id.
    pub hit: bool,
    /// The id failed in the trial (a false, a hit of an unreachable, or a conflicting declaration).
    pub failed: bool,
}

/// The digest's account of property `idx`, or `None` when the trial's data cannot be used: it was left out of the
/// statistics, or it timed out and its report decides nothing.
pub(crate) fn presence(digest: &TrialDigest, idx: u32) -> Option<Presence> {
    if digest.flags.excluded || digest.flags.inconclusive {
        return None;
    }
    let entry = digest.entries.iter().find(|e| e.idx == idx);
    Some(match entry {
        None => Presence {
            hit: false,
            failed: false,
        },
        Some(e) => Presence {
            hit: e.catalogued && e.hits > 0,
            failed: e.conflicted
                || (e.catalogued && failure_basis(e.kind, e.hits, e.falses).is_some()),
        },
    })
}

impl Service {
    /// Calls `visit` on each trial of the run after `after`, in order, until it says `Stop`, the run ends, or
    /// [`SCAN_CAP`] trials have been read. Reading stops cleanly at the cap so a filter that matches nothing cannot
    /// hold a connection; the caller continues from `last_seq`.
    pub(crate) fn scan_trials(
        &self,
        run_id: &str,
        after: Option<u64>,
        mut visit: impl FnMut(&TrialRecord) -> Result<Flow>,
    ) -> Result<ScanEnd> {
        let (mut cursor, mut scanned, mut last_seq) = (after, 0usize, None);
        loop {
            let page = self.store.list_trials(run_id, cursor, STORE_PAGE)?;
            let exhausted = page.len() < STORE_PAGE as usize;
            for trial in &page {
                scanned += 1;
                last_seq = Some(trial.seq);
                if matches!(visit(trial)?, Flow::Stop) {
                    return Ok(ScanEnd {
                        last_seq,
                        scanned: scanned as u64,
                        exhausted: false,
                    });
                }
                if scanned >= SCAN_CAP {
                    let at_end = exhausted && trial.seq == page[page.len() - 1].seq;
                    return Ok(ScanEnd {
                        last_seq,
                        scanned: scanned as u64,
                        exhausted: at_end,
                    });
                }
            }
            if exhausted || page.is_empty() {
                return Ok(ScanEnd {
                    last_seq,
                    scanned: scanned as u64,
                    exhausted: true,
                });
            }
            cursor = last_seq;
        }
    }
}

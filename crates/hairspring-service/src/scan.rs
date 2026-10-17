//! Reading a run's trials in order, a bounded amount at a time, and asking what a digest says about one property.
use crate::query::SCAN_CAP;
use crate::{Result, Service};
use hairspring_props::trial::failure_basis;
use hairspring_props::TrialDigest;
use hairspring_store::{TrialRecord, TrialStripRow};

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

/// Whether the trial's data says anything about properties: it was not left out of the statistics and did not time out.
fn usable(digest: &TrialDigest) -> bool {
    !digest.flags.excluded && !digest.flags.inconclusive
}

fn entry_failed(e: &hairspring_props::DigestEntry) -> bool {
    e.conflicted || (e.catalogued && failure_basis(e.kind, e.hits, e.falses).is_some())
}

/// The digest's account of property `idx`, or `None` when the trial's data cannot be used: it was left out of the
/// statistics, or it timed out and its report decides nothing.
pub(crate) fn presence(digest: &TrialDigest, idx: u32) -> Option<Presence> {
    if !usable(digest) {
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
            failed: entry_failed(e),
        },
    })
}

/// The catalog indexes of the properties that failed in this trial, by the same rule as [`presence`]; none when the
/// trial's data cannot be used.
pub(crate) fn failing_indexes(digest: &TrialDigest) -> Vec<u32> {
    if !usable(digest) {
        return Vec::new();
    }
    digest
        .entries
        .iter()
        .filter(|e| entry_failed(e))
        .map(|e| e.idx)
        .collect()
}

impl Service {
    /// Calls `visit` on each trial of the run after `after`, in order, until it says `Stop`, the run ends, or
    /// [`SCAN_CAP`] trials have been read. Reading stops cleanly at the cap so a filter that matches nothing cannot
    /// hold a connection; the caller continues from `last_seq`.
    pub(crate) fn scan_trials(
        &self,
        run_id: &str,
        after: Option<u64>,
        visit: impl FnMut(&TrialRecord) -> Result<Flow>,
    ) -> Result<ScanEnd> {
        scan_pages(
            after,
            |t: &TrialRecord| t.seq,
            |cursor| Ok(self.store.list_trials(run_id, cursor, STORE_PAGE)?),
            visit,
        )
    }

    /// The same walk as [`Service::scan_trials`] over the verdict and flags of each trial only. No digest or text is
    /// read, so what a request costs does not depend on what a worker stored.
    pub(crate) fn scan_strip_rows(
        &self,
        run_id: &str,
        after: Option<u64>,
        visit: impl FnMut(&TrialStripRow) -> Result<Flow>,
    ) -> Result<ScanEnd> {
        scan_pages(
            after,
            |t: &TrialStripRow| t.seq,
            |cursor| {
                Ok(self
                    .store
                    .list_trial_strip_rows(run_id, cursor, STORE_PAGE)?)
            },
            visit,
        )
    }
}

/// Pages through rows in sequence order: `fetch` returns up to [`STORE_PAGE`] rows after a cursor, `seq_of` names a
/// row's sequence number. The walk ends at `Stop`, at the end of the run, or after [`SCAN_CAP`] rows.
fn scan_pages<T>(
    after: Option<u64>,
    seq_of: impl Fn(&T) -> u64,
    fetch: impl Fn(Option<u64>) -> Result<Vec<T>>,
    mut visit: impl FnMut(&T) -> Result<Flow>,
) -> Result<ScanEnd> {
    let (mut cursor, mut scanned, mut last_seq) = (after, 0usize, None);
    loop {
        let page = fetch(cursor)?;
        let exhausted = page.len() < STORE_PAGE as usize;
        for row in &page {
            scanned += 1;
            last_seq = Some(seq_of(row));
            if matches!(visit(row)?, Flow::Stop) {
                return Ok(ScanEnd {
                    last_seq,
                    scanned: scanned as u64,
                    exhausted: false,
                });
            }
            if scanned >= SCAN_CAP {
                let at_end = exhausted && page.last().map(&seq_of) == last_seq;
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

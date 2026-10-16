//! The seed strip: one code per judged trial, in judging order, for drawing a whole run in a few pixels.
//!
//! The strip reads the same rows as the trial list and classifies each with [`TrialClass`], so a trial is a pass on the
//! strip exactly when it is a pass in the list. It needs no digest, only the row.
use crate::query::*;
use crate::scan::Flow;
use crate::trial_class::{TrialClass, ALL_CLASSES};
use crate::view::{LegendEntry, StripPage};
use crate::{Result, Service};

impl Service {
    /// One page of the strip, up to [`MAX_STRIP_PAGE`] consecutive trials. `None` for an unknown run.
    ///
    /// A page ends early at a gap in the stored sequence numbers; the next page then starts after the gap.
    pub fn strip(&self, run_id: &str, q: &StripQuery) -> Result<Option<StripPage>> {
        let after = parse_cursor(q.cursor.as_deref())?;
        let limit = page_size(q.limit, MAX_STRIP_PAGE, MAX_STRIP_PAGE);
        if self.store.get_run(run_id)?.is_none() {
            return Ok(None);
        }
        let mut codes = String::new();
        let (mut first_seq, mut last_seq): (Option<u64>, Option<u64>) = (None, None);
        let mut more = false;
        let end = self.scan_trials(run_id, after, |t| {
            let gap = last_seq.is_some_and(|prev| t.seq != prev + 1);
            if gap || codes.len() == limit {
                more = true;
                return Ok(Flow::Stop);
            }
            first_seq.get_or_insert(t.seq);
            last_seq = Some(t.seq);
            codes.push(TrialClass::of(t).code());
            Ok(Flow::Continue)
        })?;
        let next_cursor = if more {
            last_seq
        } else if end.exhausted {
            None
        } else {
            end.last_seq
        };
        Ok(Some(StripPage {
            run_id: run_id.to_owned(),
            first_seq: first_seq.unwrap_or(0),
            codes,
            legend: ALL_CLASSES
                .iter()
                .map(|c| LegendEntry {
                    code: c.code(),
                    name: c.as_str(),
                })
                .collect(),
            next_cursor: next_cursor.map(|s| s.to_string()),
            scanned_trials: end.scanned,
        }))
    }
}

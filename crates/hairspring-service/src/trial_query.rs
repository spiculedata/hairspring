//! A run's trials: the list with its filters, one trial's detail and output, and the coverage series.
use crate::output_view::{OutputMeta, STREAMS};
use crate::query::*;
use crate::scan::{presence, Flow};
use crate::view::*;
use crate::{Result, Service, ServiceError};
use hairspring_props::record::TrialRecord as KeptRecord;
use hairspring_props::TrialDigest;
use hairspring_store::{KeptState, TrialRecord};
use std::collections::BTreeSet;

fn trial_row(t: &TrialRecord) -> TrialRow {
    TrialRow {
        seq: t.seq,
        seed: t.seed.to_string(),
        verdict: t.verdict.as_str(),
        timed_out: t.timed_out,
        degraded: t.degraded,
        perturbing: t.perturbing,
        report_missing: t.report_missing,
        rated: t.rated,
        claimable: t.claimable,
        exit_code: t.exit_code,
        signal: t.signal,
        behaviour_hash: t.behaviour_hash.clone(),
        schedule_fp: t.schedule_fp.clone(),
        syscalls: t.syscalls,
        work_ops: t.work_ops,
        elapsed_ms: t.elapsed_ms,
        worker_trusted: t.worker_trusted,
        attempt: t.attempt,
        kept: t.kept.as_str(),
        has_detail: t.detail_ref.is_some(),
        has_output: t.output_ref.is_some(),
        guest: GuestReason {
            reason: t.reason.clone(),
        },
    }
}

fn decode(t: &TrialRecord) -> Result<TrialDigest> {
    TrialDigest::decode(&t.digest)
        .map_err(|e| ServiceError::Internal(format!("trial {}: {e}", t.seq)))
}

/// The filters of a trial query, checked.
struct TrialFilter {
    verdict: Option<hairspring_store::TrialVerdict>,
    fails: Option<u32>,
    hits: Option<u32>,
    misses: Option<u32>,
    degraded: Option<bool>,
    perturbing: Option<bool>,
    fp: Option<String>,
    seed: Option<u64>,
}

impl TrialFilter {
    fn parse(q: &TrialQuery) -> Result<TrialFilter> {
        Ok(TrialFilter {
            verdict: parse_verdict(q.verdict.as_deref())?,
            fails: parse_idx(q.fails.as_deref(), "fails")?,
            hits: parse_idx(q.hits.as_deref(), "hits")?,
            misses: parse_idx(q.misses.as_deref(), "misses")?,
            degraded: parse_bool(q.degraded.as_deref(), "degraded")?,
            perturbing: parse_bool(q.perturbing.as_deref(), "perturbing")?,
            fp: parse_fingerprint(q.fp.as_deref())?,
            seed: parse_seed(q.seed.as_deref())?,
        })
    }

    fn needs_digest(&self) -> bool {
        self.fails.is_some() || self.hits.is_some() || self.misses.is_some()
    }

    fn matches_row(&self, t: &TrialRecord) -> bool {
        self.seed.is_none_or(|s| t.seed == s)
            && self.verdict.is_none_or(|v| t.verdict == v)
            && self.degraded.is_none_or(|d| t.degraded == d)
            && self.perturbing.is_none_or(|p| t.perturbing == p)
            && self
                .fp
                .as_ref()
                .is_none_or(|f| t.schedule_fp.as_deref() == Some(f.as_str()))
    }

    fn matches_digest(&self, d: &TrialDigest) -> bool {
        let at = |idx: u32| presence(d, idx);
        self.fails.is_none_or(|i| at(i).is_some_and(|p| p.failed))
            && self.hits.is_none_or(|i| at(i).is_some_and(|p| p.hit))
            && self.misses.is_none_or(|i| at(i).is_some_and(|p| !p.hit))
    }
}

/// A blob read from the reference a trial row holds, never from a hash a caller supplied.
fn blob_text(svc: &Service, hash: Option<&str>) -> Result<Option<Vec<u8>>> {
    match hash {
        Some(h) => Ok(svc.store.get_blob(h)?),
        None => Ok(None),
    }
}

/// Keeps `points` to at most [`MAX_COVERAGE_POINTS`], spread evenly, always including the first and last.
fn downsample(points: Vec<CoveragePoint>) -> (Vec<CoveragePoint>, bool) {
    if points.len() <= MAX_COVERAGE_POINTS {
        return (points, false);
    }
    let last = points.len() - 1;
    let kept = (0..MAX_COVERAGE_POINTS)
        .map(|i| points[i * last / (MAX_COVERAGE_POINTS - 1)])
        .collect();
    (kept, true)
}

impl Service {
    /// One page of a run's trials in judging order. `None` for an unknown run.
    pub fn trial_page(&self, run_id: &str, q: &TrialQuery) -> Result<Option<TrialPage>> {
        let filter = TrialFilter::parse(q)?;
        let after = parse_cursor(q.cursor.as_deref())?;
        let limit = page_size(q.limit, DEFAULT_PAGE, MAX_PAGE);
        if self.store.get_run(run_id)?.is_none() {
            return Ok(None);
        }
        let mut rows: Vec<TrialRow> = Vec::new();
        let mut more = false;
        let end = self.scan_trials(run_id, after, |t| {
            if !filter.matches_row(t)
                || (filter.needs_digest() && !filter.matches_digest(&decode(t)?))
            {
                return Ok(Flow::Continue);
            }
            if rows.len() == limit {
                more = true;
                return Ok(Flow::Stop);
            }
            rows.push(trial_row(t));
            Ok(Flow::Continue)
        })?;
        let next_cursor = if more {
            rows.last().map(|r| r.seq.to_string())
        } else if end.exhausted {
            None
        } else {
            end.last_seq.map(|s| s.to_string())
        };
        Ok(Some(TrialPage {
            run_id: run_id.to_owned(),
            trials: rows,
            next_cursor,
            scanned_trials: end.scanned,
            guest_note: GUEST_NOTE,
        }))
    }

    /// One trial, with its kept detail if it has any. `None` for an unknown run or sequence number.
    pub fn trial_detail(&self, run_id: &str, seq: u64) -> Result<Option<TrialDetail>> {
        let Some(t) = self.store.get_trial(run_id, seq)? else {
            return Ok(None);
        };
        let detail = match blob_text(self, t.detail_ref.as_deref())? {
            // The record was checked when it was stored; it is checked again before it is served.
            Some(bytes) => KeptRecord::decode(&bytes)
                .ok()
                .and_then(|r| serde_json::to_value(r).ok()),
            None => None,
        };
        let meta = blob_text(self, t.output_ref.as_deref())?
            .map(|bytes| OutputMeta::read(&bytes))
            .unwrap_or_default();
        Ok(Some(TrialDetail {
            run_id: run_id.to_owned(),
            trial: trial_row(&t),
            guest: GuestDetail {
                detail,
                schedule: meta.schedule,
            },
            output_streams: meta.streams,
            output_omitted_bytes: meta.omitted,
            guest_note: GUEST_NOTE,
        }))
    }

    /// The kept text of one output stream of a trial (`stdout` or `stderr`). `None` when the trial kept none.
    pub fn trial_output(&self, run_id: &str, seq: u64, stream: &str) -> Result<Option<String>> {
        let Some(stream) = STREAMS.into_iter().find(|s| *s == stream) else {
            return Err(ServiceError::InvalidRequest(
                "`stream` must be stdout or stderr".into(),
            ));
        };
        let Some(t) = self.store.get_trial(run_id, seq)? else {
            return Ok(None);
        };
        if t.kept != KeptState::Kept {
            return Ok(None);
        }
        let Some(bytes) = blob_text(self, t.output_ref.as_deref())? else {
            return Ok(None);
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| ServiceError::Internal(format!("kept output is unreadable: {e}")))?;
        Ok(v.get(stream).and_then(|x| x.as_str()).map(str::to_owned))
    }

    /// Cumulative distinct properties reached (and true) and distinct schedules, by trial. `None` for an unknown run.
    pub fn coverage(&self, run_id: &str) -> Result<Option<Coverage>> {
        if self.store.get_run(run_id)?.is_none() {
            return Ok(None);
        }
        let (mut hit, mut tru, mut fps): (BTreeSet<u32>, BTreeSet<u32>, BTreeSet<String>) =
            Default::default();
        let (mut points, mut trials) = (Vec::new(), 0u64);
        let mut last: Option<CoveragePoint> = None;
        self.scan_all(run_id, |t| {
            trials += 1;
            let digest = decode(t)?;
            let usable = !digest.flags.excluded && !digest.flags.inconclusive;
            let before = (hit.len(), tru.len(), fps.len());
            if usable {
                for e in digest.entries.iter().filter(|e| e.catalogued) {
                    if e.hits > 0 {
                        hit.insert(e.idx);
                    }
                    if e.trues > 0 {
                        tru.insert(e.idx);
                    }
                }
            }
            if let Some(fp) = &t.schedule_fp {
                fps.insert(fp.clone());
            }
            let point = CoveragePoint {
                seq: t.seq,
                ids_hit: hit.len() as u64,
                ids_true: tru.len() as u64,
                schedules: fps.len() as u64,
            };
            if points.is_empty() || before != (hit.len(), tru.len(), fps.len()) {
                points.push(point);
            }
            last = Some(point);
            Ok(())
        })?;
        // The series always ends at the last trial, so a reader sees where it stopped.
        if let Some(p) = last.filter(|p| points.last().is_none_or(|q| q.seq != p.seq)) {
            points.push(p);
        }
        let (points, downsampled) = downsample(points);
        Ok(Some(Coverage {
            run_id: run_id.to_owned(),
            trials,
            downsampled,
            points,
        }))
    }

    /// Every trial of the run, in order, with no cap. Used only where the answer is one small series.
    fn scan_all(
        &self,
        run_id: &str,
        mut visit: impl FnMut(&TrialRecord) -> Result<()>,
    ) -> Result<()> {
        let mut after = None;
        loop {
            let end = self.scan_trials(run_id, after, |t| visit(t).map(|()| Flow::Continue))?;
            if end.exhausted || end.last_seq.is_none() {
                return Ok(());
            }
            after = end.last_seq;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(n: u64) -> Vec<CoveragePoint> {
        (0..n)
            .map(|i| CoveragePoint {
                seq: i,
                ids_hit: i,
                ids_true: 0,
                schedules: 0,
            })
            .collect()
    }

    #[test]
    fn a_short_series_is_returned_whole() {
        let (points, downsampled) = downsample(series(MAX_COVERAGE_POINTS as u64));
        assert_eq!(points.len(), MAX_COVERAGE_POINTS);
        assert!(!downsampled);
    }

    #[test]
    fn a_long_series_is_cut_to_the_cap_keeping_its_first_and_last_points_in_order() {
        let (points, downsampled) = downsample(series(5000));
        assert!(downsampled);
        assert_eq!(points.len(), MAX_COVERAGE_POINTS);
        assert_eq!(points.first().unwrap().seq, 0);
        assert_eq!(points.last().unwrap().seq, 4999);
        assert!(points.windows(2).all(|w| w[0].seq < w[1].seq));
    }
}

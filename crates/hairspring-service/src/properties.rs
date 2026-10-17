//! Property statistics: folded by the server from the stored per-trial digests, written when a job finishes, and
//! read back as the same figures.
use crate::completion_of;
use crate::ingest::stored_row;
use crate::types::{GuestStrings, PropertySummary, RunTrust};
use crate::{Result, Service, ServiceError};
use hairspring_props::limits::MAX_IDS_PER_RUN;
use hairspring_props::stats::{rule_of_three, wilson, Z_95};
use hairspring_props::{
    AssertKind, FirstFailure, PropertyStat, PropertyStatus, Rate, RateBasis, RunCatalog, RunProps,
    RunSummary, TrialDigest, UnratedReason,
};
use hairspring_store::{CatalogRowRecord, RunPropertyRow, RunRecord};
use std::collections::BTreeMap;

/// Digests read from the store at a time.
const PAGE: u32 = 500;
/// Most ids listed by name in a run's property summary.
const MAX_LISTED_IDS: usize = 10;
/// Largest stored run summary, in bytes; the kind-mismatch details are dropped to fit.
const MAX_SUMMARY_BYTES: usize = 64 * 1024;

pub(crate) struct Folded {
    pub stats: Vec<PropertyStat>,
    pub summary: RunSummary,
    pub catalog: RunCatalog,
    /// Trials that reported cleanly: the denominator of every rate.
    pub rated_trials: u64,
    /// Rated trials that were not perturbing: the trials a must-hit claim is made over.
    pub claimable_trials: u64,
}

pub(crate) fn status_name(s: PropertyStatus) -> &'static str {
    match s {
        PropertyStatus::Pass => "pass",
        PropertyStatus::Violation => "violation",
        PropertyStatus::Unmet => "unmet",
        PropertyStatus::Unrated => "unrated",
    }
}

pub(crate) fn parse_status(s: &str) -> Option<PropertyStatus> {
    Some(match s {
        "pass" => PropertyStatus::Pass,
        "violation" => PropertyStatus::Violation,
        "unmet" => PropertyStatus::Unmet,
        "unrated" => PropertyStatus::Unrated,
        _ => return None,
    })
}

fn basis_name(b: RateBasis) -> &'static str {
    match b {
        RateBasis::TrialsFalse => "trials_false",
        RateBasis::TrialsTrue => "trials_true",
        RateBasis::TrialsHit => "trials_hit",
    }
}

fn parse_basis(s: &str) -> Option<RateBasis> {
    Some(match s {
        "trials_false" => RateBasis::TrialsFalse,
        "trials_true" => RateBasis::TrialsTrue,
        "trials_hit" => RateBasis::TrialsHit,
        _ => return None,
    })
}

/// A statistic as a database row. The Wilson bounds and the rule of three are left out: they are computed on read.
fn to_row(s: &PropertyStat, idx: u32) -> RunPropertyRow {
    RunPropertyRow {
        idx,
        status: status_name(s.status).to_owned(),
        trials: s.trials,
        trials_rated: s.trials_rated,
        trials_unreported: s.trials_unreported,
        trials_degraded: s.trials_degraded,
        trials_perturbing: s.trials_perturbing,
        trials_conflicted: s.trials_conflicted,
        trials_inconclusive_with_failures: s.trials_inconclusive_with_failures,
        trials_claimable: s.trials_claimable,
        trials_hit: s.trials_hit,
        trials_true: s.trials_true,
        trials_false: s.trials_false,
        evals: s.evals,
        evals_true: s.evals_true,
        evals_false: s.evals_false,
        rate_basis: s.rate.map(|r| basis_name(r.basis).to_owned()),
        rate_k: s.rate.map(|r| r.k),
        rate_n: s.rate.map(|r| r.n),
        first_failure_seed: s.first_failure.as_ref().map(|f| f.seed),
    }
}

/// The statistic a row and its catalog entry describe, with the rate's interval recomputed from `k` and `n`.
fn from_row(row: &RunPropertyRow, cat: &CatalogRowRecord) -> Option<PropertyStat> {
    let kind = AssertKind::from_wire(&cat.kind)?;
    let rate = match (
        row.rate_basis.as_deref().and_then(parse_basis),
        row.rate_k,
        row.rate_n,
    ) {
        (Some(basis), Some(k), Some(n)) => wilson(k, n, Z_95).map(|(lower, upper)| Rate {
            basis,
            k,
            n,
            point: k as f64 / n as f64,
            lower,
            upper,
            rule_of_three: if k == 0 { rule_of_three(n) } else { None },
        }),
        _ => None,
    };
    Some(PropertyStat {
        id: cat.prop_id.clone(),
        kind,
        must_hit: kind.must_hit(),
        msg: cat.msg.clone(),
        loc: cat.loc_file.as_ref().map(|file| hairspring_props::Loc {
            file: file.clone(),
            line: cat.loc_line.unwrap_or(0),
            func: cat.loc_func.clone(),
        }),
        trials: row.trials,
        trials_rated: row.trials_rated,
        trials_unreported: row.trials_unreported,
        trials_degraded: row.trials_degraded,
        trials_perturbing: row.trials_perturbing,
        trials_conflicted: row.trials_conflicted,
        trials_inconclusive_with_failures: row.trials_inconclusive_with_failures,
        trials_claimable: row.trials_claimable,
        trials_hit: row.trials_hit,
        trials_true: row.trials_true,
        trials_false: row.trials_false,
        evals: row.evals,
        evals_true: row.evals_true,
        evals_false: row.evals_false,
        status: parse_status(&row.status)?,
        unrated_reason: rate.is_none().then_some(UnratedReason::NoRatedTrials),
        rate,
        first_failure: row.first_failure_seed.map(|seed| FirstFailure {
            seed,
            witness: None,
        }),
    })
}

/// A run that did not complete its trials makes no must-hit claim: UNMET becomes unrated.
pub(crate) fn withhold_unmet(stats: Vec<PropertyStat>) -> Vec<PropertyStat> {
    stats.into_iter().map(unmet_to_unrated).collect()
}

fn unmet_to_unrated(s: PropertyStat) -> PropertyStat {
    match s.status {
        PropertyStatus::Unmet => PropertyStat {
            status: PropertyStatus::Unrated,
            ..s
        },
        _ => s,
    }
}

impl Service {
    /// Folds every stored digest of the run, in the order the trials were judged.
    pub(crate) fn fold_run(&self, run_id: &str) -> Result<Folded> {
        let rows = self
            .store
            .get_catalog(run_id)?
            .iter()
            .map(stored_row)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| ServiceError::Internal("a stored catalog row is unreadable".into()))?;
        let catalog =
            RunCatalog::from_rows(rows).map_err(|e| ServiceError::Internal(e.to_string()))?;
        let mut run = RunProps::with_id_cap(MAX_IDS_PER_RUN);
        let (mut rated_trials, mut claimable_trials) = (0u64, 0u64);
        let mut after = None;
        loop {
            let page = self.store.list_trial_digests(run_id, after, PAGE)?;
            let Some(last) = page.last() else { break };
            after = Some(last.seq);
            for row in &page {
                let digest = TrialDigest::decode(&row.digest)
                    .map_err(|e| ServiceError::Internal(format!("trial {}: {e}", row.seq)))?;
                let f = digest.flags;
                let rated = !f.excluded && !f.inconclusive && !f.degraded && !f.report_missing;
                rated_trials += u64::from(rated);
                claimable_trials += u64::from(rated && !f.perturbing);
                run = run.add_digest(row.seed, &digest, &catalog);
            }
        }
        let summary = run.summary();
        Ok(Folded {
            stats: run.finish(),
            summary,
            catalog,
            rated_trials,
            claimable_trials,
        })
    }

    /// The property rows to store with a finished run, and the JSON of its summary.
    pub(crate) fn property_rows(
        &self,
        folded: &Folded,
        withhold: bool,
    ) -> (Vec<RunPropertyRow>, Option<String>) {
        let stats = if withhold {
            withhold_unmet(folded.stats.clone())
        } else {
            folded.stats.clone()
        };
        let rows = stats
            .iter()
            .filter_map(|s| folded.catalog.index_of(&s.id).map(|idx| to_row(s, idx)))
            .collect();
        (rows, summary_json(folded))
    }

    /// The statistics stored for a run, with intervals recomputed from the stored counts.
    pub fn property_stats(&self, run_id: &str) -> Result<Vec<PropertyStat>> {
        Ok(self
            .indexed_stats(run_id)?
            .into_iter()
            .map(|(_, s)| s)
            .collect())
    }

    /// The same statistics paired with their catalog index, sorted by id.
    pub(crate) fn indexed_stats(&self, run_id: &str) -> Result<Vec<(u32, PropertyStat)>> {
        let catalog = self.store.get_catalog(run_id)?;
        let by_idx: BTreeMap<u32, &CatalogRowRecord> = catalog.iter().map(|c| (c.idx, c)).collect();
        let mut stats: Vec<(u32, PropertyStat)> = self
            .store
            .list_run_properties(run_id)?
            .iter()
            .filter_map(|row| {
                by_idx
                    .get(&row.idx)
                    .and_then(|c| from_row(row, c))
                    .map(|s| (row.idx, s))
            })
            .collect();
        stats.sort_by(|a, b| a.1.id.cmp(&b.1.id));
        if self.unmet_is_unverified(run_id)? {
            stats = stats
                .into_iter()
                .map(|(idx, s)| (idx, unmet_to_unrated(s)))
                .collect();
        }
        Ok(stats)
    }

    /// Whether the run's UNMET results rest on worker-reported limits nobody has verified: it ended on its time or
    /// step limit and not every contributing worker is trusted. Decided when read, since trust can change after a
    /// run ends.
    fn unmet_is_unverified(&self, run_id: &str) -> Result<bool> {
        let Some(run) = self.store.get_run(run_id)? else {
            return Ok(false);
        };
        let limited = matches!(
            completion_of(&run),
            crate::RunCompletion::TimeOrStepLimited { .. }
        );
        if !limited {
            return Ok(false);
        }
        Ok(!self.run_trust(run_id)?.is_some_and(|t| t.all_trusted))
    }

    /// Who reported the run's trials, and whether all of them were trusted. `None` for an unknown run.
    pub fn run_trust(&self, run_id: &str) -> Result<Option<RunTrust>> {
        if self.store.get_run(run_id)?.is_none() {
            return Ok(None);
        }
        let workers = self.store.run_contributors(run_id)?;
        let all_trusted = !workers.is_empty() && workers.iter().all(|w| w.trusted);
        Ok(Some(RunTrust {
            workers,
            all_trusted,
        }))
    }

    /// Counts by status, the ids that need attention, and the labels a reader needs to weigh them.
    pub(crate) fn property_summary(&self, run: &RunRecord) -> Result<Option<PropertySummary>> {
        if run.catalog_size == 0 {
            return Ok(None);
        }
        let stats = self.property_stats(&run.id)?;
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        for s in &stats {
            *counts.entry(status_name(s.status).to_owned()).or_default() += 1;
        }
        let listed = |want: PropertyStatus| -> Vec<String> {
            stats
                .iter()
                .filter(|s| s.status == want)
                .take(MAX_LISTED_IDS)
                .map(|s| s.id.clone())
                .collect()
        };
        let first = stats
            .iter()
            .filter(|s| s.status == PropertyStatus::Violation)
            .filter_map(|s| s.first_failure.as_ref().map(|f| f.seed))
            .min();
        let trusted = self.run_trust(&run.id)?.is_some_and(|t| t.all_trusted);
        let completion = completion_of(run);
        Ok(Some(PropertySummary {
            catalog_size: run.catalog_size,
            counts,
            first_failing_seed: first.map(|s| s.to_string()),
            stopped_at_first_failure: run.stopped_at_first_failure,
            ingest_incomplete: run.ingest_incomplete,
            unverified_worker_data: !trusted,
            completion,
            completion_label: completion.label(),
            guest: GuestStrings {
                note: "ids come from the program under test and are untrusted text".to_owned(),
                violation_ids: listed(PropertyStatus::Violation),
                unmet_ids: listed(PropertyStatus::Unmet),
            },
        }))
    }
}

/// The run summary as JSON, with the rated and claimable trial counts beside it (the stored summary is still a valid
/// [`RunSummary`]: readers that do not know the extra fields ignore them). The kind-mismatch details are left out:
/// they carry ids and failure text from the program under test, and nothing that reads a stored run should meet that
/// text outside a marked `guest` object. The count of mismatches is kept.
fn summary_json(folded: &Folded) -> Option<String> {
    let plain = StoredSummary {
        summary: RunSummary {
            kind_mismatch_details: Vec::new(),
            ..folded.summary.clone()
        },
        counts: StoredCounts {
            rated_trials: Some(folded.rated_trials),
            claimable_trials: Some(folded.claimable_trials),
        },
    };
    serde_json::to_string(&plain)
        .ok()
        .filter(|s| s.len() <= MAX_SUMMARY_BYTES)
}

/// The counts a run summary written before they were recorded does not have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct StoredCounts {
    #[serde(default)]
    pub rated_trials: Option<u64>,
    #[serde(default)]
    pub claimable_trials: Option<u64>,
}

#[derive(serde::Serialize)]
struct StoredSummary {
    #[serde(flatten)]
    summary: RunSummary,
    #[serde(flatten)]
    counts: StoredCounts,
}

/// A stored run's summary and the counts beside it. Both are `None`/default for a run with no stored summary.
pub(crate) fn stored_summary(run: &RunRecord) -> (RunSummary, StoredCounts) {
    let Some(json) = run.property_summary_json.as_deref() else {
        return Default::default();
    };
    (
        serde_json::from_str(json).unwrap_or_default(),
        serde_json::from_str(json).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with(json: Option<&str>) -> RunRecord {
        RunRecord {
            id: "run_0000000000000000".into(),
            target: "t".into(),
            backend: "sim".into(),
            state: hairspring_store::RunState::Passed,
            seed: 1,
            config_json: "{}".into(),
            created_at: 0,
            started_at: None,
            finished_at: None,
            steps: 0,
            trials: 0,
            error: None,
            inconclusive: 0,
            distinct_schedules: 0,
            work: 0,
            mode: "search".into(),
            stopped_at_first_failure: false,
            property_summary_json: json.map(str::to_owned),
            catalog_size: 0,
            ingest_incomplete: false,
            ingest_attempt: 0,
            build_id: None,
        }
    }

    #[test]
    fn counts_beside_a_summary_are_read_back() {
        let json = r#"{"trials":6,"perturbing_trials":0,"degraded_trials":1,"report_missing_trials":0,"conflict_trials":0,"kind_mismatches":0,"kind_mismatch_details":[],"rated_trials":5,"claimable_trials":4}"#;
        let (summary, counts) = stored_summary(&run_with(Some(json)));
        assert_eq!(summary.trials, 6);
        assert_eq!(counts.rated_trials, Some(5));
        assert_eq!(counts.claimable_trials, Some(4));
    }

    #[test]
    fn a_summary_stored_before_the_counts_existed_reports_them_as_unknown_not_zero() {
        let json = r#"{"trials":6,"perturbing_trials":0,"degraded_trials":1,"report_missing_trials":0,"conflict_trials":0,"kind_mismatches":0,"kind_mismatch_details":[]}"#;
        let (summary, counts) = stored_summary(&run_with(Some(json)));
        assert_eq!(summary.trials, 6);
        assert_eq!(counts, StoredCounts::default());
        assert_eq!(counts.rated_trials, None);
    }

    #[test]
    fn a_run_with_no_stored_summary_has_unknown_counts() {
        let (summary, counts) = stored_summary(&run_with(None));
        assert_eq!(summary, RunSummary::default());
        assert_eq!(counts.rated_trials, None);
    }
}

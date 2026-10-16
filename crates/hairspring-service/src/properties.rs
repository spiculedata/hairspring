//! Property statistics: folded by the server from the stored per-trial digests, written when a job finishes, and
//! read back as the same figures.
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
}

fn status_name(s: PropertyStatus) -> &'static str {
    match s {
        PropertyStatus::Pass => "pass",
        PropertyStatus::Violation => "violation",
        PropertyStatus::Unmet => "unmet",
        PropertyStatus::Unrated => "unrated",
    }
}

fn parse_status(s: &str) -> Option<PropertyStatus> {
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
    stats
        .into_iter()
        .map(|s| match s.status {
            PropertyStatus::Unmet => PropertyStat {
                status: PropertyStatus::Unrated,
                ..s
            },
            _ => s,
        })
        .collect()
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
        let mut after = None;
        loop {
            let page = self.store.list_trial_digests(run_id, after, PAGE)?;
            let Some(last) = page.last() else { break };
            after = Some(last.seq);
            for row in &page {
                let digest = TrialDigest::decode(&row.digest)
                    .map_err(|e| ServiceError::Internal(format!("trial {}: {e}", row.seq)))?;
                run = run.add_digest(row.seed, &digest, &catalog);
            }
        }
        let summary = run.summary();
        Ok(Folded {
            stats: run.finish(),
            summary,
            catalog,
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
        (rows, summary_json(&folded.summary))
    }

    /// The statistics stored for a run, with intervals recomputed from the stored counts.
    pub fn property_stats(&self, run_id: &str) -> Result<Vec<PropertyStat>> {
        let catalog = self.store.get_catalog(run_id)?;
        let by_idx: BTreeMap<u32, &CatalogRowRecord> = catalog.iter().map(|c| (c.idx, c)).collect();
        let mut stats: Vec<PropertyStat> = self
            .store
            .list_run_properties(run_id)?
            .iter()
            .filter_map(|row| by_idx.get(&row.idx).and_then(|c| from_row(row, c)))
            .collect();
        stats.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(stats)
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
        Ok(Some(PropertySummary {
            catalog_size: run.catalog_size,
            counts,
            first_failing_seed: first.map(|s| s.to_string()),
            stopped_at_first_failure: run.stopped_at_first_failure,
            ingest_incomplete: run.ingest_incomplete,
            unverified_worker_data: !trusted,
            guest: GuestStrings {
                note: "ids come from the program under test and are untrusted text".to_owned(),
                violation_ids: listed(PropertyStatus::Violation),
                unmet_ids: listed(PropertyStatus::Unmet),
            },
        }))
    }
}

/// The run summary as JSON. The kind-mismatch details are left out: they carry ids and failure text from the program
/// under test, and nothing that reads a stored run should meet that text outside a marked `guest` object. The count
/// of mismatches is kept.
fn summary_json(summary: &RunSummary) -> Option<String> {
    let plain = RunSummary {
        kind_mismatch_details: Vec::new(),
        ..summary.clone()
    };
    serde_json::to_string(&plain)
        .ok()
        .filter(|s| s.len() <= MAX_SUMMARY_BYTES)
}

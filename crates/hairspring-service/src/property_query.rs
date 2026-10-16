//! The property table of a run and the detail of one property.
use crate::completion::{headline_qualifier, WorkerData};
use crate::properties::{status_name, stored_summary};
use crate::query::*;
use crate::scan::{presence, Flow};
use crate::view::*;
use crate::{completion_of, Result, RunCompletion, Service, ServiceError};
use hairspring_props::claim_text::Label;
use hairspring_props::honesty::{headline, rate_cell, unmet_text};
use hairspring_props::{PropertyStat, PropertyStatus, TrialDigest};
use hairspring_store::RunRecord;
use std::collections::BTreeMap;

/// The headline with its qualifier, if the run needs one, after it.
fn qualified(headline: String, qualifier: Option<String>) -> String {
    match qualifier {
        Some(q) => format!("{headline} {q}"),
        None => headline,
    }
}

fn status_rank(s: PropertyStatus) -> u8 {
    match s {
        PropertyStatus::Violation => 0,
        PropertyStatus::Unmet => 1,
        PropertyStatus::Unrated => 2,
        PropertyStatus::Pass => 3,
    }
}

/// One property as a table row. The statistic's own figures are copied; the wording is generated here.
fn property_row(
    idx: u32,
    s: &PropertyStat,
    run: &RunRecord,
    completion: &RunCompletion,
) -> PropertyRow {
    let cell = rate_cell(s, run.stopped_at_first_failure);
    let unmet = (s.status == PropertyStatus::Unmet && completion.allows_unmet())
        .then(|| unmet_text(s.trials_claimable));
    PropertyRow {
        idx,
        status: status_name(s.status),
        kind: s.kind.as_str(),
        must_hit: s.must_hit,
        rate: cell,
        rate_text: cell.text(),
        unmet,
        trials: s.trials,
        trials_rated: s.trials_rated,
        trials_claimable: s.trials_claimable,
        trials_unreported: s.trials_unreported,
        trials_degraded: s.trials_degraded,
        trials_perturbing: s.trials_perturbing,
        trials_conflicted: s.trials_conflicted,
        failures_seen_in_inconclusive_trials: s.trials_inconclusive_with_failures,
        trials_hit: s.trials_hit,
        trials_true: s.trials_true,
        trials_false: s.trials_false,
        evals: s.evals,
        first_failing_seed: s.first_failure.as_ref().map(|f| f.seed.to_string()),
        guest: GuestProperty {
            id: s.id.clone(),
            msg: s.msg.clone(),
            loc: s.loc.as_ref().map(|l| GuestLoc {
                file: l.file.clone(),
                line: l.line,
                func: l.func.clone(),
            }),
        },
    }
}

fn order(
    sort: PropertySort,
    a: &(u32, PropertyStat),
    b: &(u32, PropertyStat),
) -> std::cmp::Ordering {
    let by_id = a.1.id.cmp(&b.1.id);
    match sort {
        PropertySort::Status => status_rank(a.1.status)
            .cmp(&status_rank(b.1.status))
            .then(by_id),
        PropertySort::Id => by_id,
        PropertySort::Rate => {
            let point = |s: &PropertyStat| s.rate.map(|r| r.point);
            point(&b.1)
                .partial_cmp(&point(&a.1))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(by_id)
        }
        PropertySort::Evals => b.1.evals.cmp(&a.1.evals).then(by_id),
    }
}

fn notes(
    run: &RunRecord,
    completion: &RunCompletion,
    summary: &hairspring_props::RunSummary,
) -> Vec<String> {
    let mut out = Vec::new();
    match completion {
        RunCompletion::StoppedAtFirstFailure => out.push(
            "Stopped at first failure: no rates, no UNMET claim. The stop decided how many trials ran."
                .to_owned(),
        ),
        RunCompletion::EndedEarly => out.push(
            "The run ended early or its trial rows do not match what the worker reported: its trial data is incomplete, so no UNMET result is stated."
                .to_owned(),
        ),
        RunCompletion::TimeOrStepLimited { .. } => out.push(format!(
            "The run was {}: results are for the trials it ran, and UNMET is stated for those trials.",
            completion.label()
        )),
        RunCompletion::Running => out.push(
            "The run has not finished: property figures are written when it does.".to_owned(),
        ),
        RunCompletion::Aborted => out.push(
            "The run ended in an error or was cancelled: its property figures are not a result."
                .to_owned(),
        ),
        RunCompletion::Complete => {}
    }
    if summary.ids_dropped > 0 {
        out.push(format!(
            "{} property id occurrences were dropped because the run tracked the most ids it allows, so some properties are not shown.",
            summary.ids_dropped
        ));
    }
    if summary.conflict_trials > 0 || summary.kind_mismatches > 0 {
        out.push(format!(
            "{} trials declared a property with conflicting kinds and {} declared a kind that differs from the first trial's. These are counts only, not findings.",
            summary.conflict_trials, summary.kind_mismatches
        ));
    }
    if run.ingest_incomplete && !matches!(completion, RunCompletion::EndedEarly) {
        out.push("The stored trial rows may be incomplete.".to_owned());
    }
    out
}

impl Service {
    /// One page of a run's property table, with the run header and the claim. `None` for an unknown run.
    ///
    /// `label` is the run's declared honesty label; `None` means none was declared, which makes no claim.
    pub fn property_page(
        &self,
        run_id: &str,
        q: &PropertyQuery,
        label: Option<&Label>,
    ) -> Result<Option<PropertyPage>> {
        let status = parse_status(q.status.as_deref())?;
        let kind = parse_kind(q.kind.as_deref())?;
        let prefix = parse_prefix(q.q.as_deref())?;
        let sort = parse_sort(q.sort.as_deref())?;
        let offset = parse_cursor(q.cursor.as_deref())?.unwrap_or(0) as usize;
        let limit = page_size(q.limit, DEFAULT_PAGE, MAX_PAGE);
        let Some(run) = self.store.get_run(run_id)? else {
            return Ok(None);
        };
        let completion = completion_of(&run);
        let (summary, counts) = stored_summary(&run);
        let mut indexed = self.indexed_stats(run_id)?;
        let all: Vec<PropertyStat> = indexed.iter().map(|(_, s)| s.clone()).collect();
        let mut counts_by_status: BTreeMap<String, u64> = BTreeMap::new();
        for s in &all {
            *counts_by_status
                .entry(status_name(s.status).to_owned())
                .or_default() += 1;
        }
        indexed.retain(|(_, s)| {
            status.is_none_or(|w| s.status == w)
                && kind.is_none_or(|w| s.kind == w)
                && prefix.as_ref().is_none_or(|p| s.id.starts_with(p.as_str()))
        });
        indexed.sort_by(|a, b| order(sort, a, b));
        let total = indexed.len();
        let properties: Vec<PropertyRow> = indexed
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(idx, s)| property_row(*idx, s, &run, &completion))
            .collect();
        let next_cursor = (offset + limit < total).then(|| (offset + limit).to_string());

        let violations = counts_by_status.get("violation").copied().unwrap_or(0);
        let first_seed = all
            .iter()
            .filter(|s| s.status == PropertyStatus::Violation)
            .filter_map(|s| s.first_failure.as_ref().map(|f| f.seed))
            .min();
        let trust = self.run_trust(run_id)?;
        let workers = trust.as_ref().map(|t| t.workers.as_slice()).unwrap_or(&[]);
        let claim = self.claim_view(&run, &all, &completion, counts.rated_trials, label)?;
        let unverified_worker_data = !trust.as_ref().is_some_and(|t| t.all_trusted);
        let data = match trust.as_ref() {
            Some(t) if t.workers.is_empty() => WorkerData::NoTrialData,
            _ if unverified_worker_data => WorkerData::Unverified,
            _ => WorkerData::Trusted,
        };
        let headline = qualified(
            headline(counts.rated_trials, violations, first_seed),
            headline_qualifier(&completion, data),
        );
        Ok(Some(PropertyPage {
            run_id: run.id.clone(),
            headline,
            header: RunHeader {
                trials_judged: self.store.count_trials(run_id)?,
                trials_rated: counts.rated_trials,
                trials_claimable: counts.claimable_trials,
                distinct_schedules: run.distinct_schedules,
                excluded: Excluded {
                    degraded: summary.degraded_trials,
                    perturbing: summary.perturbing_trials,
                    inconclusive: summary.inconclusive_trials,
                    report_missing: summary.report_missing_trials,
                },
                completion,
                completion_label: completion.label(),
                stopped_at_first_failure: run.stopped_at_first_failure,
                ingest_incomplete: run.ingest_incomplete,
                workers_contributing: workers.len() as u64,
                workers_untrusted: workers.iter().filter(|w| !w.trusted).count() as u64,
                unverified_worker_data,
            },
            claim,
            notes: notes(&run, &completion, &summary),
            counts_by_status,
            total: total as u64,
            next_cursor,
            properties,
            guest_note: GUEST_NOTE,
        }))
    }

    /// One property of a run, and a page of the seeds that failed it, reached it, or did not reach it. `None` for an
    /// unknown run or index.
    pub fn property_detail(
        &self,
        run_id: &str,
        idx: u32,
        q: &SeedQuery,
    ) -> Result<Option<PropertyDetail>> {
        let seeds_kind = match q.seeds.as_deref() {
            None | Some("failing") => "failing",
            Some("hit") => "hit",
            Some("never") => "never",
            Some(_) => {
                return Err(ServiceError::InvalidRequest("`seeds` is not valid".into()));
            }
        };
        let after = parse_cursor(q.cursor.as_deref())?;
        let limit = page_size(q.limit, DEFAULT_PAGE, MAX_SEED_PAGE);
        let Some(run) = self.store.get_run(run_id)? else {
            return Ok(None);
        };
        let completion = completion_of(&run);
        let Some((_, stat)) = self
            .indexed_stats(run_id)?
            .into_iter()
            .find(|(i, _)| *i == idx)
        else {
            return Ok(None);
        };
        let mut seeds: Vec<SeedRef> = Vec::new();
        let mut more = false;
        let end = self.scan_trials(run_id, after, |t| {
            let digest = TrialDigest::decode(&t.digest)
                .map_err(|e| ServiceError::Internal(format!("trial {}: {e}", t.seq)))?;
            let wanted = presence(&digest, idx).is_some_and(|p| match seeds_kind {
                "failing" => p.failed,
                "hit" => p.hit,
                _ => !p.hit,
            });
            if !wanted {
                return Ok(Flow::Continue);
            }
            if seeds.len() == limit {
                more = true;
                return Ok(Flow::Stop);
            }
            seeds.push(SeedRef {
                seed: t.seed.to_string(),
                seq: t.seq,
            });
            Ok(Flow::Continue)
        })?;
        let next_cursor = if more {
            seeds.last().map(|s| s.seq.to_string())
        } else if end.exhausted {
            None
        } else {
            end.last_seq.map(|s| s.to_string())
        };
        Ok(Some(PropertyDetail {
            run_id: run.id.clone(),
            property: property_row(idx, &stat, &run, &completion),
            seeds_kind,
            seeds,
            next_cursor,
            scanned_trials: end.scanned,
            completion_label: completion.label(),
            guest_note: GUEST_NOTE,
        }))
    }
}

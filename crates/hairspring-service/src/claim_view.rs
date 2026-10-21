//! Wiring of the claim gate and the claim generator to a stored run.
//!
//! A sentence is generated only when every condition holds: the label is declared, every trial came from a trusted
//! worker, every failure it rests on was reproduced on a trusted worker, and the run finished in a state that can back
//! it. Whatever fails is listed in `withheld`; the figures themselves are still shown, labelled.
use crate::claims::{claim_gate, FailureEvidence};
use crate::{Result, RunCompletion, Service};
use hairspring_props::claim_text::{generate, FindingState, Label, Subject};
use hairspring_props::{PropertyStat, PropertyStatus};
use hairspring_store::RunRecord;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const CHIP_UNVERIFIED: &str = "unverified worker data";
/// Trial rows read per page while looking for the worker that reported a failing trial.
const REPORTER_PAGE: u32 = 500;

/// Whether a claim may be made about a run, the sentence if so, and what to show beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimView {
    /// The claim gate's answer: trusted workers, and a trusted reproduction of every failure.
    pub allowed: bool,
    pub text: Option<String>,
    pub chips: Vec<String>,
    pub caveats: Vec<&'static str>,
    /// Why there is no sentence, in words fit for a results page.
    pub withheld: Vec<String>,
}

fn completion_reason(c: &RunCompletion) -> Option<String> {
    match c {
        RunCompletion::Complete
        | RunCompletion::TimeOrStepLimited { .. }
        | RunCompletion::StoppedAtFirstFailure => None,
        other => Some(format!("the run did not complete: {}", other.label())),
    }
}

impl Service {
    /// The worker that reported each of the given failing seeds. A seed whose row cannot be found maps to nothing.
    fn reporters_of(&self, run_id: &str, seeds: &BTreeSet<u64>) -> Result<BTreeMap<u64, String>> {
        let mut found = BTreeMap::new();
        let mut after = None;
        while found.len() < seeds.len() {
            let page = self.store.list_trials(run_id, after, REPORTER_PAGE)?;
            let Some(last) = page.last() else { break };
            after = Some(last.seq);
            for t in page.iter().filter(|t| seeds.contains(&t.seed)) {
                found.entry(t.seed).or_insert_with(|| t.worker_id.clone());
            }
        }
        Ok(found)
    }

    /// What reproduces each violated property. No confirmation replays are stored yet, so every failure is
    /// unconfirmed and the gate refuses any run that has one.
    fn failure_evidence(
        &self,
        run_id: &str,
        stats: &[PropertyStat],
    ) -> Result<Vec<FailureEvidence>> {
        let seeds: BTreeSet<u64> = stats
            .iter()
            .filter(|s| s.status == PropertyStatus::Violation)
            .filter_map(|s| s.first_failure.as_ref().map(|f| f.seed))
            .collect();
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let reporters = self.reporters_of(run_id, &seeds)?;
        Ok(seeds
            .iter()
            .map(|seed| FailureEvidence {
                reporter: reporters.get(seed).cloned().unwrap_or_default(),
                confirmations: Vec::new(),
            })
            .collect())
    }

    /// The claim for a run, under `label` (`None` for a run whose label was never declared).
    pub(crate) fn claim_view(
        &self,
        run: &RunRecord,
        stats: &[PropertyStat],
        completion: &RunCompletion,
        rated_trials: Option<u64>,
        label: Option<&Label>,
    ) -> Result<ClaimView> {
        let trust = self.run_trust(&run.id)?.unwrap_or(crate::RunTrust {
            workers: Vec::new(),
            all_trusted: false,
        });
        let evidence = self.failure_evidence(&run.id, stats)?;
        let trusted_workers = self.list_workers()?.iter().filter(|w| w.trusted).count();
        let gate = claim_gate(&trust.workers, &evidence, trusted_workers);

        let fallback = Label::undeclared();
        let label = label.unwrap_or(&fallback);
        let mut chips = label.chips();
        if !gate.allowed && !trust.workers.is_empty() {
            chips.push(CHIP_UNVERIFIED.to_owned());
        }
        let mut withheld: Vec<String> = gate.reasons.iter().map(|r| (*r).to_owned()).collect();
        withheld.extend(completion_reason(completion));

        let first_failure = stats
            .iter()
            .filter(|s| s.status == PropertyStatus::Violation)
            .filter_map(|s| s.first_failure.as_ref().map(|f| f.seed))
            .min();
        let subject = match (first_failure, rated_trials) {
            (Some(seed), _) => Some(Subject::Failure {
                seed,
                // The gate only opens when every failure was reproduced on a trusted worker.
                state: if gate.allowed {
                    FindingState::Confirmed
                } else {
                    FindingState::SingleRun
                },
            }),
            (None, Some(rated)) => Some(Subject::NoFailure {
                rated,
                // Every trial the rate leaves out: inconclusive, degraded, unreported and perturbing alike.
                unrated: run.trials.saturating_sub(rated),
            }),
            (None, None) => {
                withheld.push("the rated-trial count is not recorded for this run".to_owned());
                None
            }
        };
        let generated = subject.map(|s| generate(label, &s)).unwrap_or_default();
        withheld.extend(generated.withheld.iter().map(|w| (*w).to_owned()));
        let text = if withheld.is_empty() {
            generated.text
        } else {
            None
        };
        Ok(ClaimView {
            allowed: gate.allowed,
            text,
            chips,
            caveats: generated.caveats,
            withheld,
        })
    }
}

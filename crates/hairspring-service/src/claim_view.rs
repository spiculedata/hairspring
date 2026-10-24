//! Wiring of the claim gate and the claim generator to a stored run.
//!
//! A sentence is generated only when every condition holds: the label is declared, every trial came from a trusted
//! worker, every failure it rests on was reproduced on a trusted worker, and the run finished in a state that can back
//! it. Whatever fails is listed in `withheld`; the figures themselves are still shown, labelled.
use crate::claims::{claim_gate, Confirmation, FailureEvidence};
use crate::confirm_claims::CheckedFailure;
use crate::{Result, RunCompletion, Service};
use hairspring_props::claim_text::{generate, FindingState, Label, Subject};
use hairspring_props::{PropertyStat, PropertyStatus};
use hairspring_store::RunRecord;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const CHIP_UNVERIFIED: &str = "unverified worker data";

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
    /// The store answers from the seed and worker columns only, so no trial row is read whole.
    fn reporters_of(&self, run_id: &str, seeds: &BTreeSet<u64>) -> Result<BTreeMap<u64, String>> {
        let wanted: Vec<u64> = seeds.iter().copied().collect();
        Ok(self.store.reporters_of(run_id, &wanted)?)
    }

    /// What reproduces each violated property: the stored checks of the failure at its seed, if any. A failure with
    /// none is unconfirmed, and the gate refuses any run that has one.
    fn failure_evidence(
        &self,
        run_id: &str,
        stats: &[PropertyStat],
        checked: &BTreeMap<u64, CheckedFailure>,
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
                confirmations: checked
                    .get(seed)
                    .map(|c| {
                        vec![Confirmation {
                            replayer: c.runner.clone(),
                            replayer_trusted: c.runner_trusted,
                            // Only a failure the checks derive confirmed or verified counts as reproduced.
                            reproduced: c.status.claim_state() != FindingState::SingleRun,
                        }]
                    })
                    .unwrap_or_default(),
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
        let checked = self.checked_failures(&run.id)?;
        let evidence = self.failure_evidence(&run.id, stats, &checked)?;
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
            (Some(seed), _) => Some(match checked.get(&seed) {
                // The state is what the stored checks derive, and only where the gate opens: every failure was
                // reproduced on a trusted worker.
                Some(c) => Subject::CheckedFailure {
                    seed,
                    state: if gate.allowed {
                        c.status.claim_state()
                    } else {
                        FindingState::SingleRun
                    },
                    pinned: c.pinned,
                },
                None => Subject::Failure {
                    seed,
                    state: FindingState::SingleRun,
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

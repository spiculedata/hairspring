//! The confirmation rerun of a violating seed: what a rerun showed and how the reruns are counted.
use super::{FailureKind, HermitBackend};
use crate::exec::{ProcessSpec, Trial};
use crate::property_verdict;
use crate::trace::{behaviour_hash, Verdict};
use std::time::Duration;

/// How the confirmation reruns of a violating seed went.
#[derive(Clone)]
pub(super) struct Confirmation {
    pub(super) reproduced: u32,
    /// Reruns that could not be judged (a timeout, a host or events fault): they say nothing about the program.
    pub(super) inconclusive: u32,
    /// Why the first rerun that failed to start could not, so the report says what went wrong on the host.
    pub(super) first_error: Option<String>,
}

/// The failure message's account of reruns that could not be judged.
pub(super) fn inconclusive_text(count: u32, first_error: Option<&str>) -> String {
    let reruns = if count == 1 { "rerun" } else { "reruns" };
    let cause = first_error.map(|e| format!(": {e}")).unwrap_or_default();
    format!("; {count} {reruns} inconclusive (host or infrastructure, not the program{cause})")
}

/// What one confirmation rerun showed.
enum Rerun {
    Reproduced,
    Different,
    Inconclusive,
    /// The rerun could not start; the text is why.
    NotStarted(String),
}

impl HermitBackend {
    /// Re-runs a violating seed. A rerun reproduces it when it is a violation with the same behaviour hash (which
    /// includes the guest's reported events) and the same failing properties, by id, kind and reason. A rerun that
    /// fails a different property is a different bug, not a reproduction. A rerun that could not be judged is counted
    /// apart, because it says nothing about whether the program is deterministic.
    pub(super) fn confirm_violation(
        &self,
        p: &ProcessSpec,
        seed: u64,
        reference: &Trial,
        kind: FailureKind,
        timeout: Duration,
    ) -> Confirmation {
        let hash = behaviour_hash(reference);
        let failing = property_verdict::failing_keys(reference);
        let reruns: Vec<Rerun> = (0..self.settings.repro_runs)
            .map(|_| match self.trial(p, seed, timeout) {
                Err(e) => Rerun::NotStarted(e.to_string()),
                Ok(t) => match self.verdict(p, &t, timeout) {
                    ref v @ (Verdict::Violation(_) | Verdict::Deadlock(_))
                        if FailureKind::of(v) == Some(kind)
                            && property_verdict::reproduces(&hash, &failing, &t) =>
                    {
                        Rerun::Reproduced
                    }
                    Verdict::Violation(_) | Verdict::Deadlock(_) | Verdict::Pass => {
                        Rerun::Different
                    }
                    Verdict::Hang(_) | Verdict::Inconclusive(_) | Verdict::Infra(_) => {
                        Rerun::Inconclusive
                    }
                },
            })
            .collect();
        let count = |want: fn(&Rerun) -> bool| reruns.iter().filter(|r| want(r)).count() as u32;
        Confirmation {
            reproduced: count(|r| matches!(r, Rerun::Reproduced)),
            inconclusive: count(|r| matches!(r, Rerun::Inconclusive | Rerun::NotStarted(_))),
            first_error: reruns.iter().find_map(|r| match r {
                Rerun::NotStarted(why) => Some(why.clone()),
                _ => None,
            }),
        }
    }
}

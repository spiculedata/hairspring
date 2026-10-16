//! What a trial's guest-reported properties mean for its verdict: which properties failed, how to name them, and
//! whether the event data is complete enough for the trial to count as a pass.
use crate::events::EventsFault;
use crate::exec::Trial;
use hairspring_props::{AssertKind, PropertyFailure, TrialProps};
use std::collections::BTreeSet;

/// Failures named in a verdict message before the rest are summarised as a count.
const MAX_NAMED_FAILURES: usize = 3;

/// The identity of a failing property: what a confirmation or replay must reproduce.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FailureKey {
    pub id: String,
    pub kind: AssertKind,
}

fn key_of(f: PropertyFailure) -> FailureKey {
    FailureKey {
        id: f.id,
        kind: f.kind,
    }
}

/// Trial-level property failures (an `always` that was false, an `unreachable` that was hit, a catalog conflict),
/// whatever else is wrong with the trial's data: a false that was read is a false.
pub fn failures(t: &Trial) -> Vec<PropertyFailure> {
    t.events
        .as_ref()
        .map(TrialProps::verdict)
        .unwrap_or_default()
}

/// Every failing property of the trial, as a set.
pub fn failing_keys(t: &Trial) -> BTreeSet<FailureKey> {
    failures(t).into_iter().map(key_of).collect()
}

/// The property a bundle records for the trial: the first failure, in id order.
pub fn primary_key(t: &Trial) -> Option<FailureKey> {
    failures(t).into_iter().next().map(key_of)
}

/// A one-line reason naming the id, thread and ordinal of each failure (the first few).
pub fn describe(failures: &[PropertyFailure]) -> String {
    let named: Vec<String> = failures
        .iter()
        .take(MAX_NAMED_FAILURES)
        .map(PropertyFailure::describe)
        .collect();
    match failures.len().saturating_sub(MAX_NAMED_FAILURES) {
        0 => named.join("; "),
        n => format!("{}; and {n} more", named.join("; ")),
    }
}

/// Why a trial that otherwise looks fine cannot be called a pass: its event data was unusable, lost or contradictory,
/// or a process never sent its end report.
pub fn unusable_events(t: &Trial) -> Option<String> {
    if let Some(EventsFault::Guest(m)) = &t.events_error {
        return Some(format!("guest events are incomplete: {m}"));
    }
    let props = t.events.as_ref()?;
    if props.is_degraded() {
        return Some("guest events were lost or contradictory".to_string());
    }
    let missing = props.report_missing();
    (!missing.is_empty()).then(|| {
        format!(
            "process {} sent no end report (cut off, or it crashed before reporting)",
            missing.join(", ")
        )
    })
}

/// The trial's properties as the run should count them. A guest fault marks the trial degraded, so it is never rated
/// and cannot back a must-hit claim; a trial whose events were lost entirely still counts as a degraded trial.
/// `None` for trials that must not be counted at all (no events, or a host fault that makes them untrustworthy).
pub fn for_run(t: &Trial) -> Option<TrialProps> {
    match (&t.events_error, &t.events) {
        (Some(EventsFault::Config(_) | EventsFault::Infra(_)), _) => None,
        (Some(EventsFault::Guest(_)), props) => {
            Some(props.clone().unwrap_or_default().with_host_fault())
        }
        (None, props) => props.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trial_without_events_has_nothing_to_count() {
        assert!(for_run(&Trial::default()).is_none());
        assert!(failures(&Trial::default()).is_empty());
        assert!(unusable_events(&Trial::default()).is_none());
    }

    #[test]
    fn a_guest_fault_counts_as_a_degraded_trial_even_without_events() {
        let t = Trial {
            events_error: Some(EventsFault::Guest("planted symlink".into())),
            ..Trial::default()
        };
        assert!(for_run(&t).unwrap().is_degraded());
        assert!(unusable_events(&t).unwrap().contains("planted symlink"));
    }

    #[test]
    fn host_faults_keep_a_trial_out_of_the_run_counts() {
        for fault in [
            EventsFault::Config("c".into()),
            EventsFault::Infra("i".into()),
        ] {
            let t = Trial {
                events_error: Some(fault),
                events: Some(TrialProps::default()),
                ..Trial::default()
            };
            assert!(for_run(&t).is_none());
        }
    }

    #[test]
    fn failure_keys_compare_by_id_and_kind() {
        let key = |id: &str, kind| FailureKey {
            id: id.into(),
            kind,
        };
        assert_ne!(
            key("a", AssertKind::Always),
            key("b", AssertKind::Always),
            "a rerun that fails a different property is not a reproduction"
        );
        assert_ne!(
            key("a", AssertKind::Always),
            key("a", AssertKind::Unreachable)
        );
    }
}

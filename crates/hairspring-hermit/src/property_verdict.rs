//! What a trial's guest-reported properties mean for its verdict: which properties failed, how to name them, and
//! whether the event data is complete enough for the trial to count as a pass.
use crate::events::EventsFault;
use crate::exec::Trial;
use crate::trace::behaviour_hash;
use hairspring_props::{AssertKind, FailureReason, PropertyFailure, TrialProps};
use std::collections::BTreeSet;

/// Failures named in a verdict message before the rest are summarised as a count.
const MAX_NAMED_FAILURES: usize = 3;

/// The identity of a failing property: what a confirmation or replay must reproduce.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FailureKey {
    pub id: String,
    pub kind: AssertKind,
    /// Why it failed: a conflict on `a` and an `always` that was false on `a` are different failures.
    pub reason: FailureReason,
}

fn key_of(f: PropertyFailure) -> FailureKey {
    FailureKey {
        id: f.id,
        kind: f.kind,
        reason: f.reason,
    }
}

/// The wire-style name of a failure reason, as recorded in bundles.
pub fn reason_name(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::AlwaysFalse => "always_false",
        FailureReason::UnreachableHit => "unreachable_hit",
        FailureReason::CatalogConflict => "catalog_conflict",
    }
}

/// Trial-level property failures (an `always` that was false, an `unreachable` that was hit, a catalog conflict),
/// sorted by id, then kind, then reason, whatever else is wrong with the trial's data: a false that was read is a
/// false. A trial that timed out has none: the report of a killed trial decides nothing, and the timeout is the
/// verdict.
pub fn failures(t: &Trial) -> Vec<PropertyFailure> {
    if t.timed_out {
        return Vec::new();
    }
    let mut all = t
        .events
        .as_ref()
        .map(TrialProps::verdict)
        .unwrap_or_default();
    all.sort_by(|a, b| (&a.id, a.kind, a.reason).cmp(&(&b.id, b.kind, b.reason)));
    all
}

/// Whether a rerun reproduces a reference trial: the same behaviour hash (which includes the reported events) and
/// the same failing properties, by id, kind and reason. A rerun that fails a different property is a different bug.
pub fn reproduces(
    reference_hash: &str,
    reference_keys: &BTreeSet<FailureKey>,
    rerun: &Trial,
) -> bool {
    behaviour_hash(rerun) == reference_hash && failing_keys(rerun) == *reference_keys
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
        // A target that sent no events at all has no events to blame; its output capture failed.
        return Some(match t.events {
            Some(_) => format!("guest events are incomplete: {m}"),
            None => format!("the output capture failed: {m}"),
        });
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
/// and cannot back a must-hit claim. `None` for trials that must not be counted at all: no events (including a
/// capture fault on a target that sends none), or a host fault that makes them untrustworthy.
pub fn for_run(t: &Trial) -> Option<TrialProps> {
    match (&t.events_error, &t.events) {
        (Some(EventsFault::Config(_) | EventsFault::Infra(_)), _) => None,
        // The capture failed but the guest sent no events: a target without an SDK has nothing to count.
        (Some(EventsFault::Guest(_)), None) => None,
        (Some(EventsFault::Guest(_)), Some(props)) => Some(props.clone().with_host_fault()),
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
    fn a_guest_fault_degrades_a_trial_that_sent_events() {
        let t = Trial {
            events_error: Some(EventsFault::Guest("cut".into())),
            events: Some(TrialProps::default()),
            ..Trial::default()
        };
        assert!(for_run(&t).unwrap().is_degraded());
        assert!(unusable_events(&t).unwrap().starts_with("guest events"));
    }

    #[test]
    fn a_capture_fault_on_a_target_without_events_is_not_blamed_on_guest_events() {
        let t = Trial {
            events_error: Some(EventsFault::Guest("incomplete capture".into())),
            ..Trial::default()
        };
        assert!(for_run(&t).is_none());
        let why = unusable_events(&t).unwrap();
        assert!(
            why.contains("output capture") && !why.contains("guest events"),
            "{why}"
        );
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

    fn lines(proc: &str, ids: &[(&str, &str)]) -> String {
        let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
        let mut out = ev(format!(
            r#"{{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"{proc}","catalog_hash":"x","signal_handlers":false}}"#
        ));
        for (id, kind) in ids {
            out += &ev(format!(
                r#"{{"v":1,"kind":"catalog","proc":"{proc}","id":"{id}","type":"{kind}","must_hit":{}}}"#,
                *kind != "unreachable"
            ));
        }
        for (id, kind) in ids {
            let (trues, falses) = if *kind == "unreachable" {
                (0, 0)
            } else {
                (0, 1)
            };
            out += &ev(format!(
                r#"{{"v":1,"kind":"assert","proc":"{proc}","id":"{id}","hits":1,"trues":{trues},"falses":{falses},"first_false":null,"first_true":null}}"#
            ));
        }
        out += &ev(format!(
            r#"{{"v":1,"kind":"end","proc":"{proc}","reason":"exit","status":0,"sites":{},"evals":1,"dropped":0,"threads":0}}"#,
            ids.len()
        ));
        out
    }

    /// Every trial built here has the same exit status, stdout and events digest, so the behaviour hash is equal and
    /// only the failing properties can tell reruns apart.
    fn trial_failing(text: &str) -> Trial {
        Trial {
            exit_code: Some(0),
            events: Some(hairspring_props::TrialEvents::from_output(text.as_bytes()).fold()),
            events_digest: Some(7),
            ..Trial::default()
        }
    }

    #[test]
    fn a_rerun_reproduces_only_with_the_same_failing_properties_even_when_the_hash_is_equal() {
        let reference = trial_failing(&lines("p1", &[("a", "always")]));
        let hash = behaviour_hash(&reference);
        let keys = failing_keys(&reference);
        assert_eq!(keys.len(), 1, "{keys:?}");
        let same = trial_failing(&lines("p1", &[("a", "always")]));
        assert!(reproduces(&hash, &keys, &same));
        for (why, other) in [
            ("a different id", lines("p1", &[("b", "always")])),
            (
                "the same id with a different kind",
                lines("p1", &[("a", "unreachable")]),
            ),
            (
                "a superset of ids",
                lines("p1", &[("a", "always"), ("b", "always")]),
            ),
        ] {
            let rerun = trial_failing(&other);
            assert_eq!(
                behaviour_hash(&rerun),
                hash,
                "{why}: the hash must not tell them apart"
            );
            assert!(!reproduces(&hash, &keys, &rerun), "{why}");
        }
        let subset_ref = trial_failing(&lines("p1", &[("a", "always"), ("b", "always")]));
        assert!(!reproduces(
            &behaviour_hash(&subset_ref),
            &failing_keys(&subset_ref),
            &reference
        ));
    }

    #[test]
    fn failures_are_listed_in_id_order_with_conflicts_among_them() {
        // `a` is declared as two kinds by two processes (a conflict); `b` is an always that was false.
        let text = format!(
            "{}{}",
            lines("p1", &[("a", "always"), ("b", "always")]),
            lines("p2", &[("a", "sometimes")])
        );
        let t = trial_failing(&text);
        let listed: Vec<_> = failures(&t)
            .iter()
            .map(|f| (f.id.clone(), f.reason))
            .collect();
        let mut sorted = listed.clone();
        sorted.sort();
        assert_eq!(listed, sorted, "{listed:?}");
        assert_eq!(primary_key(&t).unwrap().id, "a");
        assert!(failing_keys(&t)
            .iter()
            .any(|k| k.id == "a" && k.reason == FailureReason::CatalogConflict));
    }

    #[test]
    fn a_timed_out_trial_has_no_property_failures() {
        let t = Trial {
            timed_out: true,
            ..trial_failing(&lines("p1", &[("a", "always")]))
        };
        assert!(failures(&t).is_empty() && primary_key(&t).is_none());
    }

    #[test]
    fn failure_keys_compare_by_id_and_kind() {
        let key = |id: &str, kind| FailureKey {
            id: id.into(),
            kind,
            reason: FailureReason::AlwaysFalse,
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

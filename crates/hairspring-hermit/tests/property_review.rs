//! Review fixes for property verdicts: inconclusive trials, early-stopped runs, confirmation and bundles.
mod common;

use common::events::{rig, Rig};
use common::multi::report_many;
use common::props::*;
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::TargetKind;
use hairspring_engine::engine::RunRequest;
use hairspring_engine::format::to_text;
use hairspring_engine::report::RunStatus;
use hairspring_hermit::{Trial, TrialObserver};
use hairspring_props::{PropertyStatus, TrialProps};
use std::sync::{Arc, Mutex};

fn hang_is_violation(r: &mut Rig) {
    if let TargetKind::Process {
        hang_is_violation, ..
    } = &mut r.spec.kind
    {
        *hang_is_violation = true;
    }
}

/// Two processes: `p1` reports a false and ends, `p2` never ends and the guest deadlocks.
fn deadlocked_with_a_false() -> String {
    let dead = format!(
        "{}{}",
        report_many("p1", &[("w.id", "always", 1, 1)], true),
        report_many("p2", &[("q.id", "always", 2, 0)], false)
    );
    let fine = format!(
        "{}{}",
        report_many("p1", &[("w.id", "always", 2, 0)], true),
        report_many("p2", &[("q.id", "always", 2, 0)], true)
    );
    format!(
        "case \"$HAIRSPRING_SEED\" in\n1)\n{}\nsleep 30\n;;\n*)\n{}\n;;\nesac\n",
        emit(&dead),
        emit(&fine)
    )
}

#[test]
fn a_false_read_in_a_timed_out_trial_is_not_a_finding_of_the_run() {
    let mut r = rig("hang-false", &deadlocked_with_a_false(), false, vec![]);
    configure(&mut r, 3, 1);
    r.set_timeout(1);
    let o = run(r);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    assert_eq!(o.inconclusive, 1);
    let w = o.properties.iter().find(|p| p.id == "w.id").unwrap();
    assert_ne!(w.status, PropertyStatus::Violation, "{w:?}");
    assert_eq!(w.trials_inconclusive_with_failures, 1);
    assert!(w.first_failure.is_none());
    let text = to_text("svc", &o);
    assert!(!text.contains("VIOLATION"), "{text}");
    assert!(text.contains("seen in an inconclusive trial"), "{text}");
}

#[test]
fn a_timeout_that_is_a_violation_records_no_property_in_the_bundle() {
    let mut r = rig("hang-bundle", &deadlocked_with_a_false(), false, vec![]);
    configure(&mut r, 1, 1);
    r.set_timeout(1);
    hang_is_violation(&mut r);
    let o = run(r);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    let f = o.failure.as_ref().unwrap();
    assert!(f.message.contains("timed out"), "{}", f.message);
    assert!(!f.message.contains("w.id"), "{}", f.message);
    let b = bundle_of(&o);
    assert!(!b.params.contains_key("property"), "{:?}", b.params);
    assert!(o
        .properties
        .iter()
        .all(|p| p.status != PropertyStatus::Violation));
}

#[test]
fn an_early_stopped_run_makes_no_unmet_claim() {
    let both = report_many(
        "p1",
        &[("a.id", "always", 1, 1), ("b.id", "sometimes", 0, 3)],
        true,
    );
    let o = run_script("early-stop", &emit(&both), 3);
    assert_eq!(o.status, RunStatus::Failed, "{o:?}");
    assert!(o.unmet().is_empty());
    let statuses: Vec<_> = o
        .properties
        .iter()
        .map(|p| (p.id.as_str(), p.status))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("a.id", PropertyStatus::Violation),
            ("b.id", PropertyStatus::Unrated)
        ]
    );
    assert!(!serde_json::to_string(&o).unwrap().contains("Unmet"));
    assert!(!to_text("svc", &o).contains("UNMET"));
}

#[test]
fn when_every_trial_is_perturbing_nothing_is_unmet_only_unrated() {
    let perturbing = format!(
        "{}{}",
        report("s.id", "sometimes", 0, 3, true),
        ev(r#"{"v":1,"kind":"eval","id":"s.id","type":"sometimes","ok":false}"#)
    );
    let o = run_script("all-perturbing", &emit(&perturbing), 2);
    assert_eq!(o.status, RunStatus::Passed, "{o:?}");
    assert_eq!(o.properties[0].status, PropertyStatus::Unrated);
    assert!(o.unmet().is_empty());
    let text = to_text("svc", &o);
    assert!(
        !text.contains("UNMET") && text.contains("UNRATED"),
        "{text}"
    );
}

#[test]
fn a_target_without_events_whose_output_capture_failed_has_no_property_data() {
    if std::process::Command::new("perl")
        .arg("-v")
        .output()
        .is_err()
    {
        return;
    }
    let o = run_script(
        "no-events-lingering",
        "perl -e 'use POSIX; POSIX::setsid(); exec \"sleep\", \"6\"' 2>/dev/null & sleep 0.5; echo done",
        1,
    );
    let json = serde_json::to_string(&o).unwrap();
    assert!(
        !json.contains("property_run") && !json.contains("properties"),
        "{json}"
    );
    assert_eq!(
        o.inconclusive, 1,
        "a lost capture is still not a pass: {o:?}"
    );
    let message = o.error.unwrap_or_default();
    assert!(!message.contains("guest events"), "{message}");
}

/// First run reports a false; every later run (the confirmations) does `later`.
fn first_fails_then(later: &str) -> String {
    format!(
        "n=$(cat \"$EV_LOG/n\" 2>/dev/null || echo 0); echo $((n+1)) > \"$EV_LOG/n\"\nif [ \"$n\" = 0 ]; then {}\nelse {later}\nfi",
        emit(&report("a.id", "always", 0, 1, true))
    )
}

#[test]
fn reruns_that_are_only_inconclusive_are_not_blamed_on_the_program() {
    let degraded = format!(
        "echo 'HAIRSPRING_EVENT {{not json'; {}",
        emit(&report("a.id", "always", 2, 0, true))
    );
    for (name, later, timeout) in [
        ("rerun-degraded", degraded.as_str(), None),
        ("rerun-hang", "sleep 30", Some(1)),
    ] {
        let mut r = rig(name, &first_fails_then(later), false, vec![]);
        if let Some(secs) = timeout {
            r.set_timeout(secs);
        }
        let o = run(r);
        let msg = o.failure.unwrap().message;
        assert!(msg.contains("reproduced 0/3"), "{name}: {msg}");
        assert!(msg.contains("3 reruns inconclusive"), "{name}: {msg}");
        assert!(!msg.contains("non-deterministic"), "{name}: {msg}");
    }
}

#[test]
fn reruns_that_behave_differently_are_still_called_non_deterministic() {
    let o = run_script(
        "rerun-clean",
        &first_fails_then(&emit(&report("a.id", "always", 2, 0, true))),
        1,
    );
    let msg = o.failure.unwrap().message;
    assert!(
        msg.contains("reproduced 0/3") && msg.contains("non-deterministic"),
        "{msg}"
    );
    assert!(!msg.contains("inconclusive"), "{msg}");
}

#[derive(Default)]
struct Seen(Mutex<Vec<u64>>);

impl TrialObserver for Seen {
    fn on_trial(&self, seed: u64, _trial: &Trial, _props: &TrialProps) {
        self.0.lock().unwrap().push(seed);
    }
}

#[test]
fn trials_after_the_failing_seed_in_a_parallel_batch_are_not_reported() {
    let bad = report("w.id", "always", 1, 1, true);
    let good = report("w.id", "always", 2, 0, true);
    let seen = Arc::new(Seen::default());
    let mut r = rig(
        "observer-batch",
        &by_seed(&[(2, bad)], &good),
        false,
        vec![],
    );
    configure(&mut r, 6, 3);
    r.backend = r.backend.with_observer(seen.clone());
    let o = run(r);
    assert_eq!(o.status, RunStatus::Failed);
    assert_eq!(*seen.0.lock().unwrap(), vec![1, 2]);
}

#[test]
fn a_bundle_with_property_fields_survives_encode_and_decode() {
    let o = run_script(
        "bundle-trip",
        &emit(&report("r.id", "always", 0, 1, true)),
        1,
    );
    let bundle = bundle_of(&o);
    let again = Bundle::decode(&bundle.encode()).unwrap();
    assert_eq!(again, bundle);
    for (key, want) in [
        ("property", "r.id"),
        ("property_kind", "always"),
        ("property_reason", "always_false"),
    ] {
        assert_eq!(again.params.get(key).map(String::as_str), Some(want));
    }
}

#[test]
fn a_bundle_naming_a_kind_we_cannot_parse_does_not_match_any_property() {
    let (engine, _) = {
        let mut r = rig(
            "bogus-kind",
            &emit(&report("r.id", "always", 0, 1, true)),
            false,
            vec![],
        );
        configure(&mut r, 1, 1);
        r.engine()
    };
    let o = engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap();
    let mut bundle = bundle_of(&o);
    assert!(engine.replay(&bundle).unwrap().reproduced);
    bundle.params.insert("property_kind".into(), "bogus".into());
    assert!(!engine.replay(&bundle).unwrap().reproduced);
}

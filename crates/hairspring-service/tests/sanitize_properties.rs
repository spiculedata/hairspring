//! Worker-reported property data is untrusted until E6 checks it: the service drops it.
use hairspring_engine::report::RunOutcome;
use hairspring_props::{AssertKind, PropertyStat, RunProps, TrialEvents};

fn hostile_outcome() -> RunOutcome {
    let report = concat!(
        "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\",\"sdk\":\"c\",\"sdk_version\":\"0.1.0\",\"proc\":\"p1\",\"catalog_hash\":\"x\",\"signal_handlers\":false}\n",
        "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"a.id\",\"type\":\"always\",\"must_hit\":true}\n",
        "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"a.id\",\"hits\":1,\"trues\":1,\"falses\":0}\n",
        "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\",\"status\":0,\"sites\":1,\"evals\":1,\"dropped\":0,\"threads\":0}\n",
    );
    let run = RunProps::new().add(1, &TrialEvents::from_output(report.as_bytes()).fold());
    let summary = run.summary();
    let mut stats = run.finish();
    // A lie: more trials than the run had, and a kind the id never had.
    stats[0] = PropertyStat {
        trials: u64::MAX,
        kind: AssertKind::Unreachable,
        ..stats[0].clone()
    };
    RunOutcome::passed(1, 1, 1).with_properties(stats, summary)
}

#[test]
fn property_data_from_a_worker_is_dropped_not_passed_through() {
    let sent = hostile_outcome();
    assert!(!sent.properties.is_empty() && sent.property_run.is_some());
    let kept = hairspring_service::sanitize::outcome(&sent, "svc").unwrap();
    assert!(kept.properties.is_empty());
    assert!(kept.property_run.is_none());
    assert_eq!(
        (kept.trials, kept.steps_executed),
        (1, 1),
        "the rest survives"
    );
}

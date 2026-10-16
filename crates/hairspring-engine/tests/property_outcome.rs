//! `RunOutcome` carries property statistics additively: old stored outcomes still decode, new ones round-trip.
use hairspring_engine::report::RunOutcome;
use hairspring_props::{AssertKind, PropertyStatus, RunProps, TrialEvents};

#[test]
fn an_outcome_stored_before_properties_existed_still_decodes() {
    let old = r#"{"status":"passed","steps_executed":3,"trials":3,"elapsed_ms":1,"failure":null,"error":null}"#;
    let o: RunOutcome = serde_json::from_str(old).unwrap();
    assert!(o.properties.is_empty());
    assert!(o.property_run.is_none());
}

#[test]
fn an_outcome_without_properties_serialises_exactly_as_before() {
    let json = serde_json::to_string(&RunOutcome::passed(1, 1, 1)).unwrap();
    assert!(
        !json.contains("properties") && !json.contains("property_run"),
        "{json}"
    );
}

fn report(kind: AssertKind) -> String {
    format!(
        concat!(
            "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"hello\",\"sdk\":\"c\",\"sdk_version\":\"0.1.0\",\"proc\":\"p1\",\"catalog_hash\":\"abc\",\"signal_handlers\":false}}\n",
            "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"a.id\",\"type\":\"{k}\",\"must_hit\":true}}\n",
            "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"a.id\",\"hits\":0,\"trues\":0,\"falses\":0}}\n",
            "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\",\"status\":0,\"sites\":1,\"evals\":0,\"dropped\":0,\"threads\":0}}\n"
        ),
        k = kind.as_str()
    )
}

#[test]
fn property_statistics_round_trip_and_unmet_lists_only_unmet_properties() {
    let trial = TrialEvents::from_output(report(AssertKind::Sometimes).as_bytes()).fold();
    let run = RunProps::new().add(1, &trial);
    let summary = run.summary();
    let outcome = RunOutcome::passed(1, 1, 1).with_properties(run.finish(), summary);
    assert_eq!(outcome.unmet().len(), 1);
    assert_eq!(outcome.properties[0].status, PropertyStatus::Unmet);
    let json = serde_json::to_string(&outcome).unwrap();
    let back: RunOutcome = serde_json::from_str(&json).unwrap();
    assert_eq!(back, outcome);
}

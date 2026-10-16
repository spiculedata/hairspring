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
    // serde_json's default float parsing may differ in the last bit, so intervals are compared with a tolerance and
    // everything else exactly.
    let (want, got) = (
        outcome.properties[0].rate.unwrap(),
        back.properties[0].rate.unwrap(),
    );
    assert!((want.upper - got.upper).abs() < 1e-12 && (want.lower - got.lower).abs() < 1e-12);
    let strip = |o: &RunOutcome| {
        let mut o = o.clone();
        o.properties[0].rate = None;
        o
    };
    assert_eq!(strip(&back), strip(&outcome));
}

#[test]
fn without_properties_drops_exactly_the_property_fields() {
    let trial = TrialEvents::from_output(report(AssertKind::Sometimes).as_bytes()).fold();
    let run = RunProps::new().add(1, &trial);
    let summary = run.summary();
    let full = RunOutcome::passed(4, 3, 9).with_properties(run.finish(), summary);
    assert_eq!(
        full.clone().without_properties(),
        RunOutcome::passed(4, 3, 9)
    );
}

#[test]
fn a_run_whose_only_trials_are_perturbing_reports_unrated_and_no_unmet_section() {
    let mut trial = TrialEvents::from_output(report(AssertKind::Sometimes).as_bytes()).fold();
    trial.perturbing = true;
    let run = RunProps::new().add(1, &trial);
    let summary = run.summary();
    let outcome = RunOutcome::passed(1, 1, 1).with_properties(run.finish(), summary);
    assert!(outcome.unmet().is_empty());
    let text = hairspring_engine::format::to_text("svc", &outcome);
    assert!(
        text.contains("UNRATED") && !text.contains("UNMET"),
        "{text}"
    );
}

#[test]
fn a_failed_outcome_never_prints_an_unmet_section_even_if_statuses_say_unmet() {
    let trial = TrialEvents::from_output(report(AssertKind::Sometimes).as_bytes()).fold();
    let run = RunProps::new().add(1, &trial);
    let summary = run.summary();
    let mut outcome = RunOutcome::passed(1, 1, 1).with_properties(run.finish(), summary);
    outcome.status = hairspring_engine::report::RunStatus::Failed;
    let text = hairspring_engine::format::to_text("svc", &outcome);
    assert!(!text.contains("UNMET (run-level"), "{text}");
}

fn report_with_two_ids() -> String {
    let line = |body: &str| format!("HAIRSPRING_EVENT {body}\n");
    let mut out = line(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"abc","signal_handlers":false}"#);
    for id in ["a.id", "b.id"] {
        out.push_str(&line(&format!(
            r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"always","must_hit":true}}"#
        )));
        out.push_str(&line(&format!(
            r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":1,"trues":1,"falses":0}}"#
        )));
    }
    out.push_str(&line(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":2,"evals":2,"dropped":0,"threads":0}"#));
    out
}

#[test]
fn the_text_report_separates_timed_out_trials_and_flags_dropped_ids() {
    let trial = TrialEvents::from_output(report_with_two_ids().as_bytes()).fold();
    let run = RunProps::with_id_cap(1)
        .add_inconclusive(1, &trial)
        .add(2, &trial);
    let summary = run.summary();
    let outcome = RunOutcome::passed(2, 2, 1).with_properties(run.finish(), summary);
    let text = hairspring_engine::format::to_text("svc", &outcome);
    assert!(text.contains("0 without an end report"), "{text}");
    assert!(text.contains("1 timed out"), "{text}");
    assert!(
        text.contains("2 id(s) dropped") && text.contains("run-wide id limit"),
        "{text}"
    );
}

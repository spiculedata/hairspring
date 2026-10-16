//! Keeps `sdks/conformance/schema/v1.json` and the golden corpora in step with the parser.

use hairspring_props::limits::*;
use hairspring_props::*;
use serde_json::Value;

const SCHEMA: &str = include_str!("../../../sdks/conformance/schema/v1.json");
const VALID: &str = include_str!("../../../sdks/conformance/goldens/valid.events");
const INVALID: &str = include_str!("../../../sdks/conformance/goldens/invalid.events");
const IGNORED: &str = include_str!("../../../sdks/conformance/goldens/ignored.events");

fn schema() -> Value {
    serde_json::from_str(SCHEMA).expect("the schema is valid JSON")
}

fn limit(schema: &Value, name: &str) -> u64 {
    schema["x-hairspring-limits"][name]
        .as_u64()
        .unwrap_or_else(|| panic!("missing limit {name}"))
}

#[test]
fn schema_limits_match_the_crate_constants() {
    let s = schema();
    assert_eq!(s["x-hairspring-limits"]["event_prefix"], EVENT_PREFIX);
    let expected = [
        ("max_line_bytes", MAX_LINE_BYTES),
        ("max_id_bytes", MAX_ID_BYTES),
        ("max_thread_key_bytes", MAX_THREAD_KEY_BYTES),
        ("max_proc_bytes", MAX_PROC_BYTES),
        ("max_msg_bytes", MAX_MSG_BYTES),
        ("vals_len", VALS_LEN),
        ("max_ids_per_proc", MAX_IDS_PER_PROC),
        ("max_threads_per_proc", MAX_THREADS_PER_PROC),
        ("max_procs_per_trial", MAX_PROCS_PER_TRIAL),
        ("max_event_bytes_per_trial", MAX_EVENT_BYTES_PER_TRIAL),
        ("max_json_depth", MAX_JSON_DEPTH),
    ];
    for (name, value) in expected {
        assert_eq!(limit(&s, name), value as u64, "{name}");
    }
}

#[test]
fn schema_definitions_use_the_same_limits() {
    let s = schema();
    let d = &s["$defs"];
    assert_eq!(d["id"]["maxLength"], MAX_ID_BYTES);
    assert_eq!(d["thread"]["maxLength"], MAX_THREAD_KEY_BYTES);
    assert_eq!(d["proc"]["maxLength"], MAX_PROC_BYTES);
    assert_eq!(
        d["catalog"]["properties"]["msg"]["maxLength"],
        MAX_MSG_BYTES
    );
    assert_eq!(
        d["hello"]["properties"]["sdk"]["maxLength"],
        MAX_SDK_STR_BYTES
    );
    assert_eq!(
        d["hello"]["properties"]["catalog_hash"]["maxLength"],
        MAX_HASH_BYTES
    );
    assert_eq!(
        d["loc"]["properties"]["file"]["maxLength"],
        MAX_LOC_FILE_BYTES
    );
    assert_eq!(
        d["loc"]["properties"]["func"]["maxLength"],
        MAX_LOC_FUNC_BYTES
    );
    assert_eq!(d["vals"]["minItems"], VALS_LEN);
    assert_eq!(d["vals"]["maxItems"], VALS_LEN);
    assert_eq!(
        d["end"]["properties"]["threads"]["maxItems"],
        MAX_THREADS_PER_PROC
    );
    assert_eq!(s["properties"]["v"]["const"], PROTOCOL_VERSION);
}

#[test]
fn schema_kinds_match_the_assertion_kinds() {
    let s = schema();
    let names: Vec<&str> = s["$defs"]["assert_kind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let expected: Vec<&str> = AssertKind::ALL.iter().map(|k| k.as_str()).collect();
    assert_eq!(names, expected);
}

#[test]
fn schema_declares_every_message_kind_with_its_required_fields() {
    let s = schema();
    let kinds: Vec<&str> = s["allOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["if"]["properties"]["kind"]["const"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["hello", "catalog", "assert", "work", "end", "eval"]);
    for kind in kinds {
        assert!(s["$defs"][kind]["required"].is_array(), "{kind}");
    }
}

#[test]
fn every_valid_golden_line_parses_as_an_event() {
    let mut count = 0;
    for l in VALID.lines() {
        assert!(
            matches!(parse_line(l.as_bytes()), LineOutcome::Event { .. }),
            "{l}"
        );
        count += 1;
    }
    assert!(count >= 10);
}

#[test]
fn every_invalid_golden_line_is_rejected() {
    for l in INVALID.lines() {
        assert!(
            matches!(parse_line(l.as_bytes()), LineOutcome::Rejected(_)),
            "{l}"
        );
    }
}

#[test]
fn every_ignored_golden_line_is_ignored() {
    for l in IGNORED.lines() {
        assert!(
            matches!(parse_line(l.as_bytes()), LineOutcome::Ignored(_)),
            "{l}"
        );
    }
}

#[test]
fn the_valid_golden_trial_folds_to_the_expected_verdict() {
    let t = TrialEvents::from_output(VALID.as_bytes()).fold();
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert!(t.perturbing, "the corpus includes an eval line");
    let failures = t.verdict();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].id, "wal.ckpt.nbackfill_le_mxframe");
    let w = failures[0].witness.as_ref().unwrap();
    assert_eq!((w.thread.as_str(), w.ord), ("writer-0", 675));
    assert_eq!(t.thread_reports().len(), 2);
    assert!(t.report_missing().is_empty());
}

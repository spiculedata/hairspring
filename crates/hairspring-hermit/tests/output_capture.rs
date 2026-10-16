//! Bounded output capture keeps both ends of an over-limit stream, so a report printed last is never lost.
mod common;

use common::events::{rig, Rig};
use common::props::{configure, run};

const MIB: usize = 1024 * 1024;
const REPORT_LINE: &str = "FINAL REPORT: all invariants held";

/// `n` bytes of `c`, with no newline.
fn filler(n: usize, c: &str) -> String {
    format!("head -c {n} /dev/zero | tr '\\0' '{c}'; ")
}

#[test]
fn a_report_line_after_more_than_a_mebibyte_of_output_is_kept() {
    let script = format!("{}echo; echo '{REPORT_LINE}'", filler(3_000_000, "x"));
    let t = rig("cap-tail", &script, false, vec![]).run(1);
    assert!(
        t.stdout.trim_end().ends_with(REPORT_LINE),
        "the report line must survive: {:?}",
        &t.stdout[t.stdout.len().saturating_sub(80)..]
    );
    assert!(t.stdout.starts_with("xxxx"), "the head is kept too");
}

#[test]
fn the_stream_is_cut_in_the_middle_with_a_marker_and_exact_counts() {
    let script = format!("{}echo; echo '{REPORT_LINE}'", filler(3_000_000, "x"));
    let t = rig("cap-counts", &script, false, vec![]).run(1);
    let total = 3_000_000 + 1 + REPORT_LINE.len() + 1;
    let omitted = (total - MIB) as u64;
    assert_eq!(t.stdout_omitted.bytes, omitted);
    assert!(t.stdout_omitted.is_truncated());
    assert!(
        t.stdout
            .contains(&format!("[... {omitted} bytes omitted ...]")),
        "marker with the count is in the text"
    );
    assert!(
        t.stdout.len() < MIB + 100,
        "the kept text stays within the cap plus the marker: {}",
        t.stdout.len()
    );
    assert!(!t.stderr_omitted.is_truncated());
}

#[test]
fn a_small_stream_is_unchanged() {
    let t = rig("cap-small", "echo hello; echo bye >&2", false, vec![]).run(1);
    assert_eq!(t.stdout, "hello\n");
    assert_eq!(t.stderr, "bye\n");
    assert_eq!(t.stdout_omitted.bytes, 0);
    assert_eq!(t.stderr_omitted.bytes, 0);
}

#[test]
fn a_stream_exactly_at_the_cap_is_kept_whole() {
    let t = rig("cap-exact", &filler(MIB, "y"), false, vec![]).run(1);
    assert_eq!(t.stdout.len(), MIB);
    assert_eq!(t.stdout_omitted.bytes, 0);
}

#[test]
fn stderr_is_bounded_the_same_way() {
    let script = format!(
        "{{ {}echo; echo '{REPORT_LINE}'; }} >&2",
        filler(2_000_000, "e")
    );
    let t = rig("cap-stderr", &script, false, vec![]).run(1);
    assert!(
        t.stderr.trim_end().ends_with(REPORT_LINE),
        "{:?}",
        &t.stderr[t.stderr.len().saturating_sub(80)..]
    );
    assert!(t.stderr_omitted.bytes > 0);
    assert_eq!(t.stdout_omitted.bytes, 0);
}

#[test]
fn guest_events_after_a_mebibyte_of_chatter_still_all_arrive() {
    // The chatter overflows the plain-output cap; the event lines have their own budget and are not part of it.
    let script = format!("{}echo; cat $REPORT", filler(3_000_000, "x"));
    let t = rig("cap-events", &script, false, vec![]).run(1);
    let props = t.events.expect("events");
    assert!(!props.is_degraded(), "{:?}", props.health);
    assert_eq!(props.health.over_limit, 0, "{:?}", props.health);
    assert!(t.stdout_omitted.is_truncated());
    assert!(!t.stdout.contains("HAIRSPRING_EVENT"));
}

/// One trial per seed, where the middle of the stream depends on the seed and both ends do not.
fn middle_differs_by_seed(name: &str) -> Rig {
    let script = format!(
        "c=a; [ \"$HAIRSPRING_SEED\" = 2 ] && c=b; {}head -c 1000000 /dev/zero | tr '\\0' \"$c\"; {}echo END",
        filler(1_500_000, "z"),
        filler(1_500_000, "z"),
    );
    let mut r = rig(name, &script, false, vec![]);
    configure(&mut r, 2, 1);
    r
}

#[test]
fn streams_that_differ_only_in_the_omitted_middle_hash_differently() {
    let out = run(middle_differs_by_seed("cap-hash"));
    let hashes: Vec<_> = out
        .trial_log
        .iter()
        .map(|t| t.behaviour_hash.clone().unwrap())
        .collect();
    assert_eq!(hashes.len(), 2);
    assert_ne!(
        hashes[0], hashes[1],
        "the omitted bytes differ, so must the hash"
    );
}

#[test]
fn the_stored_output_says_how_much_was_omitted() {
    let script = format!("{}echo; echo '{REPORT_LINE}'", filler(3_000_000, "x"));
    let mut r = rig("cap-stored", &script, false, vec![]);
    configure(&mut r, 1, 1);
    let out = run(r);
    let stored = out.trial_log[0].output.clone().expect("output kept");
    let json: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert!(json["stdout_omitted_bytes"].as_u64().unwrap() > 0, "{json}");
    assert!(json["stdout"].as_str().unwrap().contains(REPORT_LINE));
    assert!(json.get("stderr_omitted_bytes").is_none(), "{json}");
}

#[test]
fn a_clean_trial_stores_the_same_output_record_as_before() {
    let mut r = rig("cap-stored-small", "echo hi", false, vec![]);
    configure(&mut r, 1, 1);
    let out = run(r);
    let stored = out.trial_log[0].output.clone().expect("output kept");
    let json: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(json, serde_json::json!({"stdout": "hi\n", "stderr": ""}));
}

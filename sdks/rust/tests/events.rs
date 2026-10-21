//! The SDK's output as written by real programs: the shape of the report, the output channel, the exit status.
//! (The reference parser folds the same programs in `hairspring-props/tests/rust_sdk.rs`.)

mod common;

use common::{events, number, of_kind, run, scratch, LINE_MAX};

const BASIC: &str = env!("CARGO_BIN_EXE_hs_sample_basic");

#[test]
fn a_report_is_one_hello_the_catalog_one_assert_per_id_and_an_end() {
    let r = run(BASIC, &[], &[]);
    assert!(r.status.success());
    let ev = events(&r.stderr);
    assert!(ev[0].contains("\"kind\":\"hello\""), "{}", ev[0]);
    assert!(ev.last().unwrap().contains("\"kind\":\"end\""));
    assert_eq!(of_kind(&ev, "catalog").len(), 7);
    assert_eq!(of_kind(&ev, "assert").len(), 7);
    assert_eq!(of_kind(&ev, "work").len(), 1);
    assert_eq!(of_kind(&ev, "thread").len(), 1);
    assert_eq!(
        r.stderr.lines().count(),
        ev.len(),
        "nothing but protocol lines"
    );
}

#[test]
fn every_line_is_one_json_object_within_the_line_cap() {
    let r = run(BASIC, &[], &[]);
    for line in r.stderr.lines() {
        assert!(line.len() < LINE_MAX, "{} bytes", line.len());
        assert!(
            line.starts_with("HAIRSPRING_EVENT {\"v\":1,\"kind\":\""),
            "{line}"
        );
        assert!(line.ends_with('}'), "{line}");
    }
}

#[test]
fn hello_names_the_rust_sdk_and_the_same_catalog_hash_the_c_sdk_computes() {
    let r = run(BASIC, &[], &[]);
    let hello = events(&r.stderr)[0];
    assert!(hello.contains("\"sdk\":\"rust\""), "{hello}");
    assert!(hello.contains("\"proc\":\"p1\""), "{hello}");
    assert!(
        hello.contains("\"catalog_hash\":\"fb6cf21a75aad6ef\""),
        "{hello}"
    );
    assert!(hello.contains("\"signal_handlers\":false"), "{hello}");
}

#[test]
fn counts_and_first_witnesses_are_those_of_the_evaluations() {
    let r = run(BASIC, &[], &[]);
    let ev = events(&r.stderr);
    let asserts = of_kind(&ev, "assert");
    let bad = asserts
        .iter()
        .find(|l| l.contains("\"basic.always.bad\""))
        .unwrap();
    assert_eq!(
        (
            number(bad, "hits"),
            number(bad, "trues"),
            number(bad, "falses")
        ),
        (1, 0, 1)
    );
    assert!(
        bad.contains("\"first_false\":{\"thread\":\"t#1\",\"ord\":"),
        "{bad}"
    );
    assert!(bad.contains("\"vals\":[7,8,0,0]"), "{bad}");
    let end = of_kind(&ev, "end")[0];
    assert_eq!(
        (
            number(end, "sites"),
            number(end, "evals"),
            number(end, "threads")
        ),
        (7, 11, 1)
    );
    assert_eq!(number(of_kind(&ev, "work")[0], "ops"), 300);
}

#[test]
fn events_go_to_the_file_named_by_the_environment_and_not_to_stderr() {
    let file = scratch("events_file");
    let r = run(
        BASIC,
        &[],
        &[("HAIRSPRING_EVENTS_FILE", file.to_str().unwrap())],
    );
    assert_eq!(r.stderr, "");
    let written = std::fs::read_to_string(&file).expect("events file written");
    let _ = std::fs::remove_file(&file);
    assert_eq!(events(&written).len(), 18);
}

#[test]
fn an_events_file_that_is_a_symlink_is_refused_and_events_fall_back_to_stderr() {
    let target = scratch("symlink_target");
    let link = scratch("symlink");
    std::fs::write(&target, "").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let r = run(
        BASIC,
        &[],
        &[("HAIRSPRING_EVENTS_FILE", link.to_str().unwrap())],
    );
    let target_text = std::fs::read_to_string(&target).unwrap();
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_file(&target);
    assert_eq!(target_text, "");
    let ev = events(&r.stderr);
    assert_eq!(ev.len(), 18);
    assert_eq!(
        number(of_kind(&ev, "end")[0], "dropped"),
        1,
        "the refused file is counted"
    );
}

#[test]
fn events_off_records_but_writes_nothing() {
    let r = run(BASIC, &[], &[("HAIRSPRING_EVENTS", "off")]);
    assert!(r.status.success());
    assert_eq!(r.stderr, "");
}

#[test]
fn an_invalid_process_tag_from_the_harness_is_refused_and_counted() {
    let r = run(BASIC, &[], &[("HAIRSPRING_PROC", "bad tag")]);
    let ev = events(&r.stderr);
    assert!(ev[0].contains("\"proc\":\"p1\""));
    assert_eq!(number(of_kind(&ev, "end")[0], "dropped"), 1);
}

#[test]
fn the_harness_can_choose_the_process_tag() {
    let r = run(BASIC, &[], &[("HAIRSPRING_PROC", "worker.7")]);
    assert!(events(&r.stderr)
        .iter()
        .all(|l| l.contains("\"proc\":\"worker.7\"") || l.contains("\"kind\":\"eval\"")));
}

#[test]
fn the_exit_status_is_recorded_when_exit_is_used() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_exit_status"), &[], &[]);
    assert_eq!(r.status.code(), Some(3));
    let ev = events(&r.stderr);
    assert!(of_kind(&ev, "end")[0].contains(&format!("\"status\":{:>20}", 3)));
}

#[test]
fn a_program_with_no_assertions_still_reports_work() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_empty"), &[], &[]);
    let ev = events(&r.stderr);
    assert_eq!(of_kind(&ev, "catalog").len(), 0);
    assert_eq!(of_kind(&ev, "assert").len(), 0);
    assert_eq!(number(of_kind(&ev, "work")[0], "ops"), 1);
    assert_eq!(number(of_kind(&ev, "end")[0], "sites"), 0);
}

#[test]
fn one_id_at_two_call_sites_is_one_catalogued_id_with_summed_counts() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_merge"), &[], &[]);
    let ev = events(&r.stderr);
    let shared: Vec<&str> = of_kind(&ev, "assert")
        .into_iter()
        .filter(|l| l.contains("\"merge.shared\""))
        .collect();
    assert_eq!(shared.len(), 1);
    assert_eq!(
        (
            number(shared[0], "hits"),
            number(shared[0], "trues"),
            number(shared[0], "falses")
        ),
        (4, 3, 1)
    );
    assert_eq!(number(of_kind(&ev, "end")[0], "sites"), 2);
}

#[test]
fn a_call_site_in_a_generic_function_is_one_record_for_every_instantiation() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_generic"), &[], &[]);
    let ev = events(&r.stderr);
    assert_eq!(of_kind(&ev, "catalog").len(), 1);
    let asserts = of_kind(&ev, "assert");
    assert_eq!(asserts.len(), 1);
    assert_eq!(
        (
            number(asserts[0], "hits"),
            number(asserts[0], "trues"),
            number(asserts[0], "falses")
        ),
        (3, 2, 1)
    );
    assert_eq!(number(of_kind(&ev, "end")[0], "sites"), 1);
}

#[test]
fn threads_are_named_and_counted_one_line_each() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_threads"), &["100"], &[]);
    let ev = events(&r.stderr);
    assert_eq!(of_kind(&ev, "thread").len(), 101);
    assert_eq!(number(of_kind(&ev, "end")[0], "threads"), 101);
    assert!(of_kind(&ev, "thread")
        .iter()
        .any(|l| l.contains("\"thread\":\"w-99\"") && l.contains("\"key_stable\":true ")));
}

#[test]
fn threads_beyond_the_table_share_the_last_slot_and_are_counted_as_dropped() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_threads"), &["300"], &[]);
    let ev = events(&r.stderr);
    let end = of_kind(&ev, "end")[0];
    assert_eq!(number(end, "threads"), 256);
    assert!(number(end, "dropped") > 0);
    assert!(of_kind(&ev, "thread")
        .iter()
        .any(|l| l.contains("\"t#overflow\"")));
}

#[test]
fn a_call_site_outside_the_catalog_section_is_announced_and_counted() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_audit"), &[], &[]);
    let note = "hairspring: incomplete catalog: 3 evaluation(s) ran at call sites that are missing from the catalog section";
    assert!(r.stderr.lines().any(|l| l == note), "{}", r.stderr);
    let ev = events(&r.stderr);
    assert_eq!(number(of_kind(&ev, "end")[0], "dropped"), 1);
}

#[test]
fn a_complete_catalog_is_not_announced_as_incomplete() {
    let r = run(BASIC, &[], &[]);
    assert!(!r.stderr.contains("incomplete catalog"));
}

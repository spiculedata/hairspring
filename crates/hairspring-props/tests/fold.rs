mod common;

use common::*;
use hairspring_props::limits::*;
use hairspring_props::*;

#[test]
fn complete_report_folds_into_properties_procs_and_threads() {
    let t = fold(&single(AssertKind::Always, 411, 1));
    assert!(!t.is_degraded());
    assert!(!t.perturbing);
    assert_eq!(t.properties.len(), 1);
    let p = &t.properties[0];
    assert_eq!(
        (p.id.as_str(), p.kind, p.must_hit),
        ("x.id", AssertKind::Always, true)
    );
    assert_eq!((p.hits, p.trues, p.falses), (412, 411, 1));
    assert_eq!(p.msg, "m x.id");
    assert_eq!(p.loc.as_ref().unwrap().line, 7);
    assert_eq!(
        p.first_false,
        Some(Witness {
            proc: "p1".into(),
            thread: "w-0".into(),
            ord: 5,
            vals: [1, 2, 3, 4]
        })
    );
    assert_eq!(t.procs.len(), 1);
    assert_eq!(t.procs[0].sdk.as_deref(), Some("c"));
    assert_eq!(t.procs[0].signal_handlers, Some(false));
    assert!(t.report_missing().is_empty());
    assert_eq!(t.total_work(), 9);
    let threads = t.thread_reports();
    assert_eq!(threads.len(), 2);
    assert_eq!(
        (threads[0].1.thread.as_str(), threads[0].1.tape_consumed),
        ("w-0", 10)
    );
    assert!(!threads[1].1.key_stable);
}

#[test]
fn verdict_names_id_thread_and_ordinal() {
    let t = fold(&single(AssertKind::Always, 3, 2));
    let failures = t.verdict();
    assert_eq!(failures.len(), 1);
    let f = &failures[0];
    assert_eq!(f.reason, FailureReason::AlwaysFalse);
    assert_eq!(f.count, 2);
    let text = f.describe();
    assert!(
        text.contains("x.id") && text.contains("w-0") && text.contains("ordinal 5"),
        "{text}"
    );
}

#[test]
fn trial_level_truth_table() {
    use AssertKind::*;
    // (kind, trues, falses, expected failure)
    let cases = [
        (Always, 0, 0, false),
        (Always, 5, 0, false),
        (Always, 0, 5, true),
        (Always, 5, 5, true),
        (AlwaysOrUnreachable, 0, 0, false),
        (AlwaysOrUnreachable, 5, 0, false),
        (AlwaysOrUnreachable, 0, 5, true),
        (AlwaysOrUnreachable, 5, 5, true),
        (Sometimes, 0, 0, false),
        (Sometimes, 5, 0, false),
        (Sometimes, 0, 5, false),
        (Sometimes, 5, 5, false),
        (Reachable, 0, 0, false),
        (Reachable, 5, 0, false),
        (Unreachable, 0, 0, false),
        (Unreachable, 5, 0, true),
    ];
    for (kind, trues, falses, fails) in cases {
        let t = fold(&single(kind, trues, falses));
        assert_eq!(!t.verdict().is_empty(), fails, "{kind:?} {trues}/{falses}");
        assert!(!t.is_degraded(), "{kind:?} {trues}/{falses}");
    }
}

#[test]
fn unreachable_failure_uses_the_hit_witness() {
    let t = fold(&single(AssertKind::Unreachable, 2, 0));
    let f = &t.verdict()[0];
    assert_eq!(f.reason, FailureReason::UnreachableHit);
    assert_eq!(f.witness.as_ref().unwrap().ord, 1);
}

#[test]
fn catalog_without_end_is_report_missing_and_not_a_failure() {
    let out = [hello("p1"), catalog("p1", "x.id", AssertKind::Always)].concat();
    let t = fold(&out);
    assert_eq!(t.report_missing(), vec!["p1"]);
    assert!(t.verdict().is_empty());
    assert_eq!(t.properties[0].hits, 0);
}

#[test]
fn chatter_before_the_report_does_not_hide_it() {
    let chatter = "x".repeat(99) + "\n";
    let mut out = chatter.repeat(30_000); // ~3 MB of ordinary output
    out.push_str(&single(AssertKind::Always, 1, 1));
    let t = fold(&out);
    assert_eq!(t.verdict().len(), 1);
    assert!(!t.is_degraded());
    assert!(t.report_missing().is_empty());
}

#[test]
fn two_processes_are_merged_with_summed_counts_and_first_witness() {
    let out = [
        hello("p1"),
        catalog("p1", "a", AssertKind::Always),
        assert_line("p1", "a", 2, 1, Some(("t1", 9)), Some(("t1", 1))),
        end("p1"),
        hello("p2"),
        catalog("p2", "a", AssertKind::Always),
        assert_line("p2", "a", 5, 4, Some(("t2", 3)), Some(("t2", 2))),
        work("p2", 4),
        end("p2"),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.procs.len(), 2);
    assert_eq!(t.properties.len(), 1);
    let p = &t.properties[0];
    assert_eq!((p.hits, p.trues, p.falses), (12, 7, 5));
    assert_eq!(p.first_false.as_ref().unwrap().proc, "p1");
    assert_eq!(p.first_false.as_ref().unwrap().ord, 9);
    assert_eq!(t.total_work(), 4);
}

#[test]
fn a_kind_conflict_keeps_the_first_kind_and_is_reported() {
    let out = [
        hello("p1"),
        catalog("p1", "a", AssertKind::Always),
        hello("p2"),
        catalog("p2", "a", AssertKind::Sometimes),
        assert_line("p2", "a", 1, 0, None, Some(("t", 1))),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.conflicts.len(), 1);
    let c = &t.conflicts[0];
    assert_eq!(
        (c.kept, c.rejected, c.proc.as_str()),
        (AssertKind::Always, AssertKind::Sometimes, "p2")
    );
    assert_eq!(t.properties[0].kind, AssertKind::Always);
    assert_eq!(
        t.properties[0].hits, 0,
        "the conflicting process's counts are not merged"
    );
}

#[test]
fn the_same_declaration_twice_merges_silently() {
    let out = [
        hello("p1"),
        catalog("p1", "a", AssertKind::Reachable),
        catalog("p1", "a", AssertKind::Reachable),
        assert_line("p1", "a", 3, 0, None, Some(("t", 1))),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    assert!(t.conflicts.is_empty());
    assert!(!t.is_degraded());
    assert_eq!(t.properties.len(), 1);
}

#[test]
fn duplicate_assert_for_one_process_is_dropped_and_counted() {
    let out = [
        hello("p1"),
        catalog("p1", "a", AssertKind::Always),
        assert_line("p1", "a", 3, 0, None, Some(("t", 1))),
        assert_line("p1", "a", 100, 0, None, Some(("t", 1))),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.properties[0].hits, 3);
    assert_eq!(t.health.duplicates, 1);
    assert!(t.is_degraded());
}

#[test]
fn duplicate_hello_work_and_end_are_dropped_and_counted() {
    let out = [
        hello("p1"),
        hello("p1"),
        work("p1", 1),
        work("p1", 2),
        end("p1"),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.health.duplicates, 3);
    assert_eq!(t.total_work(), 1);
}

#[test]
fn assert_for_an_uncatalogued_id_is_dropped_and_counted() {
    let out = [
        hello("p1"),
        assert_line("p1", "ghost", 1, 0, None, Some(("t", 1))),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    assert!(t.properties.is_empty());
    assert_eq!(t.health.uncatalogued, 1);
    assert!(t.is_degraded());
}

#[test]
fn eval_lines_mark_the_trial_perturbing_and_fold_into_counts() {
    let e = |ok: bool| {
        ev(&format!(
            r#"{{"v":1,"kind":"eval","id":"s.id","type":"always","ok":{ok}}}"#
        ))
    };
    let out = [e(true), e(true), e(false)].concat();
    let t = fold(&out);
    assert!(t.perturbing);
    assert!(!t.is_degraded());
    let p = &t.properties[0];
    assert_eq!((p.hits, p.trues, p.falses), (3, 2, 1));
    assert_eq!(t.verdict().len(), 1);
    assert_eq!(t.verdict()[0].witness, None);
}

#[test]
fn eval_with_a_conflicting_kind_is_a_conflict() {
    let out = [
        catalog("p0", "a", AssertKind::Always),
        ev(r#"{"v":1,"kind":"eval","id":"a","type":"sometimes","ok":true}"#),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.conflicts.len(), 1);
}

#[test]
fn newer_versions_unknown_kinds_and_fields_are_counted_but_not_degrading() {
    let out = [
        ev(r#"{"v":2,"kind":"whatever"}"#),
        ev(r#"{"v":1,"kind":"telemetry"}"#),
        ev(r#"{"v":1,"kind":"work","proc":"p","ops":1,"extra":1,"more":2}"#),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.health.unsupported_version, 1);
    assert_eq!(t.health.unknown_kind, 1);
    assert_eq!(t.health.unknown_fields, 2);
    assert_eq!(t.health.accepted, 1);
    assert!(!t.is_degraded());
}

#[test]
fn malformed_event_lines_degrade_the_trial_but_not_other_events() {
    let out = [
        "HAIRSPRING_EVENT {nope\n".to_string(),
        "HAIRSPRING_EVENT\n".to_string(),
        single(AssertKind::Always, 2, 0),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.health.malformed, 2);
    assert!(t.is_degraded());
    assert_eq!(t.properties.len(), 1);
}

#[test]
fn oversize_and_invalid_utf8_lines_are_counted_separately() {
    let mut out = format!("HAIRSPRING_EVENT {}\n", "x".repeat(MAX_LINE_BYTES)).into_bytes();
    out.extend_from_slice(b"HAIRSPRING_EVENT {\"v\":1,\"x\":\"\xff\"}\n");
    let t = TrialEvents::from_output(&out).fold();
    assert_eq!(t.health.too_long, 1);
    assert_eq!(t.health.invalid_utf8, 1);
    assert!(t.is_degraded());
}

#[test]
fn event_byte_budget_is_enforced_per_trial() {
    let line = work("p1", 1);
    let n = MAX_EVENT_BYTES_PER_TRIAL / line.len() + 50;
    let out = line.repeat(n);
    let t = fold(&out);
    assert!(t.health.event_bytes as usize <= MAX_EVENT_BYTES_PER_TRIAL);
    assert!(t.health.over_limit >= 50);
    assert!(t.is_degraded());
}

#[test]
fn processes_per_trial_are_capped() {
    let out: String = (0..MAX_PROCS_PER_TRIAL + 5)
        .map(|i| hello(&format!("p{i}")))
        .collect();
    let t = fold(&out);
    assert_eq!(t.procs.len(), MAX_PROCS_PER_TRIAL);
    assert_eq!(t.health.over_limit, 5);
}

#[test]
fn ids_per_process_are_capped() {
    let out: String = (0..MAX_IDS_PER_PROC + 3)
        .map(|i| catalog_min("p1", &format!("id{i}"), AssertKind::Reachable))
        .collect();
    let t = TrialEvents::from_output(out.as_bytes()).fold();
    assert_eq!(t.properties.len(), MAX_IDS_PER_PROC);
    assert!(t.health.over_limit >= 3);
}

#[test]
fn ids_are_capped_per_process_not_per_trial() {
    let out: String = (0..2)
        .flat_map(|p| {
            (0..MAX_IDS_PER_PROC).map(move |i| {
                catalog_min(&format!("p{p}"), &format!("id{i}"), AssertKind::Reachable)
            })
        })
        .collect();
    let t = fold(&out);
    assert_eq!(t.properties.len(), MAX_IDS_PER_PROC);
    assert_eq!(t.health.over_limit, 0);
}

#[test]
fn from_lines_matches_from_output() {
    let out = single(AssertKind::Always, 2, 1);
    let lines: Vec<&[u8]> = out.lines().map(str::as_bytes).collect();
    assert_eq!(TrialEvents::from_lines(lines).fold(), fold(&out));
}

#[test]
fn empty_and_plain_output_fold_to_an_empty_trial() {
    assert_eq!(fold(""), TrialProps::default());
    assert_eq!(fold("just\nsome\noutput"), TrialProps::default());
}

#[test]
fn properties_are_sorted_by_id() {
    let out = [
        hello("p1"),
        catalog("p1", "zz", AssertKind::Reachable),
        catalog("p1", "aa", AssertKind::Reachable),
        catalog("p1", "mm", AssertKind::Reachable),
    ]
    .concat();
    let ids: Vec<_> = fold(&out).properties.iter().map(|p| p.id.clone()).collect();
    assert_eq!(ids, ["aa", "mm", "zz"]);
}

#[test]
fn eval_only_trials_do_not_look_like_lost_reports() {
    let t = fold(&ev(
        r#"{"v":1,"kind":"eval","id":"s","type":"always","ok":true}"#,
    ));
    assert!(t.procs.is_empty());
    assert!(t.report_missing().is_empty());
}

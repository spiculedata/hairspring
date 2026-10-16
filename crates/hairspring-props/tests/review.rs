//! Tests for the M1 review findings (C1, H1, H2, M1-M6 and the LOW items).

mod common;

use common::*;
use hairspring_props::limits::*;
use hairspring_props::*;

fn parse(json: &str) -> LineOutcome {
    parse_line(format!("{EVENT_PREFIX}{json}").as_bytes())
}

fn rejected(json: &str) -> RejectReason {
    match parse(json) {
        LineOutcome::Rejected(r) => r,
        other => panic!("expected rejection for {json}, got {other:?}"),
    }
}

fn catalog_msg(msg: &str) -> String {
    format!(
        r#"{{"v":1,"kind":"catalog","proc":"p","id":"x","type":"reachable","must_hit":true,"msg":"{msg}"}}"#
    )
}

fn run_of(trials: &[(u64, String)]) -> (RunSummary, Vec<PropertyStat>) {
    let mut run = RunProps::new();
    for (seed, out) in trials {
        run = run.add(*seed, &fold(out));
    }
    (run.summary(), run.finish())
}

// ---- C1: kind conflicts are trial-level errors -------------------------------------------------------------------

fn in_process_conflict() -> String {
    [
        hello("p1"),
        catalog("p1", "x", AssertKind::Always),
        catalog("p1", "x", AssertKind::Sometimes),
        assert_line("p1", "x", 0, 1, Some(("w", 4)), None),
        end("p1"),
    ]
    .concat()
}

#[test]
fn a_conflict_inside_one_process_is_a_failure_and_degrades_the_trial() {
    let t = fold(&in_process_conflict());
    let failures = t.verdict();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].reason, FailureReason::CatalogConflict);
    assert_eq!(failures[0].id, "x");
    assert!(
        failures[0].describe().contains("Sometimes")
            || failures[0].describe().contains("sometimes")
    );
    assert!(t.is_degraded());
    assert_eq!(t.health.conflicts, 1);
    assert_eq!(
        t.health.dropped_asserts, 1,
        "the dropped assert is counted, not silent"
    );
}

#[test]
fn a_conflict_across_processes_is_also_a_failure() {
    let out = [
        catalog("p1", "x", AssertKind::Always),
        catalog("p2", "x", AssertKind::Reachable),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.verdict()[0].reason, FailureReason::CatalogConflict);
    assert!(t.is_degraded());
}

#[test]
fn a_conflict_makes_the_run_stat_a_violation_not_unmet() {
    let (summary, stats) = run_of(&[(5, in_process_conflict())]);
    assert_eq!(stats[0].status, PropertyStatus::Violation);
    assert_eq!(stats[0].trials_conflicted, 1);
    assert_eq!(stats[0].first_failure.as_ref().unwrap().seed, 5);
    assert_eq!(summary.conflict_trials, 1);
}

// ---- H1: thread lines --------------------------------------------------------------------------------------------

fn many_threads(procs: &str, n: usize, declared: u64) -> String {
    let mut out = hello(procs);
    for i in 0..n {
        out += &thread_line(procs, &format!("worker-{i}"), 1000 + i as u64, 64, true);
    }
    out += &ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"{procs}","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":{declared}}}"#
    ));
    out
}

#[test]
fn a_trial_with_more_threads_than_fit_in_one_line_parses() {
    let t = fold(&many_threads("p1", 100, 100));
    assert_eq!(t.thread_reports().len(), 100);
    assert!(!t.is_degraded(), "{:?}", t.health);
}

#[test]
fn the_maximum_thread_count_is_accepted_and_one_more_is_not() {
    let ok = fold(&many_threads(
        "p1",
        MAX_THREADS_PER_PROC,
        MAX_THREADS_PER_PROC as u64,
    ));
    assert_eq!(ok.thread_reports().len(), MAX_THREADS_PER_PROC);
    assert!(!ok.is_degraded());

    let over = fold(&many_threads(
        "p1",
        MAX_THREADS_PER_PROC + 1,
        MAX_THREADS_PER_PROC as u64,
    ));
    assert_eq!(over.thread_reports().len(), MAX_THREADS_PER_PROC);
    assert!(over.health.over_limit >= 1);
    assert!(over.is_degraded());

    let json = format!(
        r#"{{"v":1,"kind":"end","proc":"p","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":{}}}"#,
        MAX_THREADS_PER_PROC + 1
    );
    assert_eq!(rejected(&json), RejectReason::OverLimit("threads"));
}

#[test]
fn a_thread_count_that_disagrees_with_the_lines_degrades_the_trial() {
    let t = fold(&many_threads("p1", 3, 5));
    assert_eq!(t.health.thread_count_mismatch, 1);
    assert!(t.is_degraded());
    let t = fold(&many_threads("p1", 5, 3));
    assert_eq!(t.health.thread_count_mismatch, 1);
}

#[test]
fn thread_lines_may_follow_end_and_duplicates_are_counted() {
    let out = [
        hello("p1"),
        end_n("p1", 1),
        thread_line("p1", "w", 1, 2, true),
        thread_line("p1", "w", 9, 9, true),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.thread_reports().len(), 1);
    assert_eq!(t.thread_reports()[0].1.evals, 1);
    assert_eq!(t.health.duplicates, 1);
    assert_eq!(t.health.thread_count_mismatch, 0);
}

// ---- H2: duplicate asserts are summed ----------------------------------------------------------------------------

#[test]
fn a_second_report_of_the_same_id_adds_its_failures_instead_of_hiding_them() {
    let out = [
        hello("p1"),
        catalog("p1", "x", AssertKind::Always),
        assert_line("p1", "x", 5, 0, None, Some(("t", 1))),
        assert_line("p1", "x", 0, 2, Some(("t", 9)), None),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    let p = &t.properties[0];
    assert_eq!((p.hits, p.trues, p.falses), (7, 5, 2));
    assert_eq!(t.verdict()[0].reason, FailureReason::AlwaysFalse);
    assert_eq!(t.health.duplicates, 1);
    assert!(t.is_degraded());
}

#[test]
fn forked_children_with_unique_tags_are_independent() {
    let out = [
        hello("p1"),
        catalog("p1", "x", AssertKind::Always),
        assert_line("p1", "x", 1, 0, None, Some(("t", 1))),
        end("p1"),
        hello("p1.f1"),
        catalog("p1.f1", "x", AssertKind::Always),
        assert_line("p1.f1", "x", 2, 0, None, Some(("t", 1))),
        end("p1.f1"),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.procs.len(), 2);
    assert_eq!(t.properties[0].hits, 3);
    assert_eq!(t.health.duplicates, 0);
    assert!(!t.is_degraded());
}

#[test]
fn the_reserved_eval_tag_cannot_be_used_by_a_real_process() {
    let json = format!(
        r#"{{"v":1,"kind":"hello","sdk":"c","sdk_version":"1","proc":"{DEFAULT_EVAL_PROC}","catalog_hash":"h","signal_handlers":false}}"#
    );
    assert_eq!(rejected(&json), RejectReason::InvalidField("proc"));
    assert_eq!(DEFAULT_EVAL_PROC, "_eval");
}

// ---- M1: order independence --------------------------------------------------------------------------------------

#[test]
fn an_assert_that_precedes_its_catalog_line_is_still_applied() {
    let out = [
        hello("p1"),
        assert_line("p1", "x", 0, 2, Some(("w", 3)), None),
        catalog("p1", "x", AssertKind::Always),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.health.uncatalogued, 0);
    assert_eq!(t.properties[0].falses, 2);
    assert_eq!(t.verdict().len(), 1);
}

// ---- M2: streaming -----------------------------------------------------------------------------------------------

#[test]
fn folding_a_stream_of_lines_matches_folding_the_buffer() {
    let out = single(AssertKind::Always, 3, 1);
    let streamed = TrialEvents::fold_stream(out.lines().map(str::as_bytes));
    assert_eq!(streamed, fold(&out));
}

// ---- M3 / M6: run-level denominators and first failure -----------------------------------------------------------

fn crashed() -> String {
    [hello("p1"), catalog("p1", "x.id", AssertKind::Always)].concat()
}

#[test]
fn crashed_trials_are_excluded_from_the_rate_and_reported_separately() {
    let (summary, stats) = run_of(&[
        (1, single(AssertKind::Always, 4, 0)),
        (2, crashed()),
        (3, crashed()),
        (4, single(AssertKind::Always, 4, 1)),
    ]);
    let s = &stats[0];
    assert_eq!(s.trials, 4);
    assert_eq!(s.trials_rated, 2);
    assert_eq!(s.trials_unreported, 2);
    let rate = s.rate.unwrap();
    assert_eq!((rate.k, rate.n), (1, 2));
    assert_eq!(summary.report_missing_trials, 2);
}

#[test]
fn a_failure_in_an_unrated_trial_is_still_a_violation() {
    let bad_but_failing = format!(
        "HAIRSPRING_EVENT {{oops\n{}",
        single(AssertKind::Always, 0, 1)
    );
    let (summary, stats) = run_of(&[(1, single(AssertKind::Always, 4, 0)), (2, bad_but_failing)]);
    let s = &stats[0];
    assert_eq!(s.status, PropertyStatus::Violation);
    assert_eq!(s.trials_false, 1);
    assert_eq!(s.trials_degraded, 1);
    assert_eq!(s.trials_rated, 1);
    assert_eq!(s.rate.unwrap().k, 0, "the rate counts rated trials only");
    assert_eq!(summary.degraded_trials, 1);
}

#[test]
fn first_failure_keeps_the_lowest_seed_whatever_the_add_order() {
    let (_, stats) = run_of(&[
        (9, single(AssertKind::Always, 0, 1)),
        (3, single(AssertKind::Always, 0, 1)),
        (7, single(AssertKind::Always, 0, 1)),
    ]);
    assert_eq!(stats[0].first_failure.as_ref().unwrap().seed, 3);
}

#[test]
fn perturbing_trials_are_counted_per_property() {
    let eval = ev(r#"{"v":1,"kind":"eval","id":"x.id","type":"always","ok":true}"#);
    let (_, stats) = run_of(&[(1, eval), (2, single(AssertKind::Always, 1, 0))]);
    assert_eq!(stats[0].trials_perturbing, 1);
}

#[test]
fn a_kind_change_between_trials_keeps_that_trials_failure_visible() {
    let (summary, _) = run_of(&[
        (1, single(AssertKind::Always, 1, 0)),
        (2, single(AssertKind::Unreachable, 3, 0)),
    ]);
    assert_eq!(summary.kind_mismatches, 1);
    let d = &summary.kind_mismatch_details[0];
    assert_eq!(
        (d.seed, d.kept, d.seen),
        (2, AssertKind::Always, AssertKind::Unreachable)
    );
    assert_eq!(
        d.failure.as_ref().unwrap().reason,
        FailureReason::UnreachableHit
    );
}

// ---- M4: bytes, not code points ----------------------------------------------------------------------------------

#[test]
fn text_limits_count_bytes_not_characters() {
    let ok = "\u{20ac}".repeat(MAX_MSG_BYTES / 3); // 170 chars, 510 bytes
    assert!(matches!(
        parse(&catalog_msg(&ok)),
        LineOutcome::Event { .. }
    ));
    let too_long_in_bytes = "\u{20ac}".repeat(MAX_MSG_BYTES); // 512 chars, 1536 bytes
    assert_eq!(
        rejected(&catalog_msg(&too_long_in_bytes)),
        RejectReason::OverLimit("msg")
    );
}

// ---- M5: unreported ids and sites --------------------------------------------------------------------------------

#[test]
fn a_catalogued_id_without_an_assert_after_end_is_counted_as_unreported() {
    let out = [
        hello("p1"),
        catalog("p1", "a", AssertKind::Always),
        catalog("p1", "b", AssertKind::Always),
        assert_line("p1", "a", 1, 0, None, Some(("t", 1))),
        end_n("p1", 0),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.health.unreported_ids, 1);
    assert!(t.is_degraded());
}

#[test]
fn end_sites_must_match_the_catalogued_ids() {
    let out = [
        hello("p1"),
        catalog("p1", "a", AssertKind::Always),
        assert_line("p1", "a", 1, 0, None, Some(("t", 1))),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":4,"evals":1,"dropped":0,"threads":0}"#),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.health.sites_mismatch, 1);
    assert!(t.is_degraded());
}

// ---- LOW ---------------------------------------------------------------------------------------------------------

#[test]
fn ordinal_zero_is_rejected() {
    let json = r#"{"v":1,"kind":"assert","proc":"p","id":"a","hits":1,"trues":0,"falses":1,"first_false":{"thread":"t","ord":0,"vals":[0,0,0,0]}}"#;
    assert_eq!(rejected(json), RejectReason::InvalidField("ord"));
}

#[test]
fn invisible_and_separator_characters_are_rejected_in_text() {
    for bad in [
        "a\\u202eb",
        "a\\u200bb",
        "a\\u2028b",
        "a\\u2029b",
        "a\\ufeffb",
        "a\\u00adb",
        "a\\u2066b",
    ] {
        assert_eq!(
            rejected(&catalog_msg(bad)),
            RejectReason::InvalidField("msg"),
            "{bad}"
        );
    }
    assert!(matches!(
        parse(&catalog_msg("na\u{ef}ve \u{4e2d}\u{6587}")),
        LineOutcome::Event { .. }
    ));
}

#[test]
fn non_event_lines_never_count_against_the_budget() {
    let out = "HAIRSPRING_EVENTS_FILE=/some/path\n".repeat(60_000);
    let t = fold(&out);
    assert_eq!(t.health.event_bytes, 0);
    assert_eq!(t.health.over_limit, 0);
    assert!(!t.is_degraded());
}

#[test]
fn nesting_beyond_the_json_libraries_own_limit_is_still_too_deep() {
    let huge = format!(
        r#"{{"v":1,"kind":"work","proc":"p","ops":1,"x":{}{}}}"#,
        "[".repeat(2000),
        "]".repeat(2000)
    );
    assert_eq!(rejected(&huge), RejectReason::TooDeep);
}

#[test]
fn documented_lenient_and_strict_number_inputs() {
    assert!(matches!(
        parse_line(b"HAIRSPRING_EVENT   {\"v\":1,\"kind\":\"work\",\"proc\":\"p\",\"ops\":1}"),
        LineOutcome::Event { .. }
    ));
    for ops in ["-0", "1e2", "1.0", "01", "+1"] {
        let json = format!(r#"{{"v":1,"kind":"work","proc":"p","ops":{ops}}}"#);
        assert!(matches!(parse(&json), LineOutcome::Rejected(_)), "{ops}");
    }
}

#[test]
fn eval_ignores_ok_for_reachable_and_unreachable() {
    let e = |kind: &str| {
        ev(&format!(
            r#"{{"v":1,"kind":"eval","id":"{kind}","type":"{kind}","ok":false}}"#
        ))
    };
    let t = fold(&[e("reachable"), e("unreachable")].concat());
    let reach = t.properties.iter().find(|p| p.id == "reachable").unwrap();
    assert_eq!((reach.hits, reach.trues, reach.falses), (1, 1, 0));
    assert!(t
        .verdict()
        .iter()
        .any(|f| f.reason == FailureReason::UnreachableHit));
}

#[test]
fn reachable_and_unreachable_use_only_hits() {
    // A malformed SDK that reports falses for a reachable site still gets hit-based semantics.
    let t = fold(&single(AssertKind::Reachable, 0, 3));
    assert!(t.verdict().is_empty());
    let (_, stats) = run_of(&[(1, single(AssertKind::Reachable, 0, 3))]);
    assert_eq!(stats[0].status, PropertyStatus::Pass);
}

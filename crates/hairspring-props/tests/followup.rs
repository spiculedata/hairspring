//! Tests for the re-review follow-ups: eval tags, unrated stats, deterministic conflicts, dropped counts, merged sites.

mod common;

use common::*;
use hairspring_props::*;

fn parse(json: &str) -> LineOutcome {
    parse_line(format!("{}{json}", limits::EVENT_PREFIX).as_bytes())
}

fn run_of(trials: &[(u64, String)]) -> Vec<PropertyStat> {
    let mut run = RunProps::new();
    for (seed, out) in trials {
        run = run.add(*seed, &fold(out));
    }
    run.finish()
}

// ---- eval lines cannot name a real process -----------------------------------------------------------------------

#[test]
fn an_eval_line_naming_a_real_process_is_rejected() {
    let json = r#"{"v":1,"kind":"eval","proc":"p1","id":"x.id","type":"always","ok":false}"#;
    assert_eq!(
        parse(json),
        LineOutcome::Rejected(RejectReason::InvalidField("proc"))
    );
}

#[test]
fn an_eval_line_may_name_the_reserved_tag_explicitly() {
    let json = r#"{"v":1,"kind":"eval","proc":"_eval","id":"x.id","type":"always","ok":true}"#;
    assert!(matches!(parse(json), LineOutcome::Event { .. }));
}

#[test]
fn an_eval_line_cannot_inflate_an_sdk_process() {
    let out = [
        single(AssertKind::Always, 1, 0),
        ev(r#"{"v":1,"kind":"eval","proc":"p1","id":"x.id","type":"always","ok":true}"#),
    ]
    .concat();
    let t = fold(&out);
    assert_eq!(t.properties[0].hits, 1);
    assert_eq!(t.health.malformed, 1);
    assert!(t.is_degraded());
}

// ---- unrated is not 0% -------------------------------------------------------------------------------------------

fn crashed() -> String {
    [hello("p1"), catalog("p1", "x.id", AssertKind::Always)].concat()
}

#[test]
fn a_stat_with_no_rated_trials_says_so_instead_of_reporting_zero_percent() {
    let stats = run_of(&[(1, crashed()), (2, crashed())]);
    let s = &stats[0];
    assert_eq!(s.trials_rated, 0);
    assert!(s.rate.is_none());
    assert_eq!(s.unrated_reason, Some(UnratedReason::NoRatedTrials));
    assert_eq!(s.rate_text(), "unrated (0 rated trials)");
}

#[test]
fn a_rated_stat_has_no_unrated_reason_and_a_numeric_rate_text() {
    let stats = run_of(&[(1, single(AssertKind::Always, 2, 0))]);
    let s = &stats[0];
    assert_eq!(s.unrated_reason, None);
    let text = s.rate_text();
    assert!(text.starts_with("0/1"), "{text}");
    assert!(!text.contains("unrated"));
}

// ---- cross-process conflicts resolve by the lowest tag -----------------------------------------------------------

fn conflict_trial(p2_first: bool) -> String {
    let p1 = [
        hello("p1"),
        catalog("p1", "x", AssertKind::Always),
        assert_line("p1", "x", 1, 1, Some(("w", 4)), Some(("w", 1))),
        end("p1"),
    ]
    .concat();
    let p2 = [
        hello("p2"),
        catalog("p2", "x", AssertKind::Sometimes),
        assert_line("p2", "x", 3, 2, None, Some(("v", 1))),
        end("p2"),
    ]
    .concat();
    if p2_first {
        [p2, p1].concat()
    } else {
        [p1, p2].concat()
    }
}

#[test]
fn the_kept_kind_and_verdict_do_not_depend_on_which_process_spoke_first() {
    let a = fold(&conflict_trial(false));
    let b = fold(&conflict_trial(true));
    assert_eq!(a.properties[0].kind, AssertKind::Always);
    assert_eq!(b.properties[0].kind, AssertKind::Always);
    assert_eq!(a.verdict(), b.verdict());
    assert_eq!(a.conflicts, b.conflicts);
    assert_eq!(a.properties, b.properties);
    assert_eq!(a.conflicts[0].proc, "p2");
}

#[test]
fn within_one_process_the_first_declaration_in_stream_order_wins() {
    let out = [
        catalog("p1", "x", AssertKind::Sometimes),
        catalog("p1", "x", AssertKind::Always),
    ]
    .concat();
    assert_eq!(fold(&out).properties[0].kind, AssertKind::Sometimes);
}

// ---- dropped counts are attached to the conflict -----------------------------------------------------------------

#[test]
fn the_conflict_failure_carries_the_counts_of_the_dropped_assert() {
    let out = [
        hello("p1"),
        catalog("p1", "x", AssertKind::Always),
        catalog("p1", "x", AssertKind::Sometimes),
        assert_line("p1", "x", 4, 1, Some(("w", 4)), Some(("w", 1))),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    let failure = &t.verdict()[0];
    assert_eq!(failure.reason, FailureReason::CatalogConflict);
    assert_eq!(
        failure.dropped,
        Some(DroppedCounts {
            hits: 5,
            trues: 4,
            falses: 1
        })
    );
    assert!(failure.describe().contains("falses 1"));
}

#[test]
fn ordinary_failures_have_no_dropped_counts() {
    let t = fold(&single(AssertKind::Always, 0, 1));
    assert_eq!(t.verdict()[0].dropped, None);
}

// ---- merged call sites -------------------------------------------------------------------------------------------

#[test]
fn one_id_at_two_call_sites_is_one_site_with_one_assert() {
    let site = |line: u32| {
        ev(&format!(
            r#"{{"v":1,"kind":"catalog","proc":"p1","id":"x.id","type":"always","must_hit":true,"loc":{{"file":"f.c","line":{line}}}}}"#
        ))
    };
    let out = [
        hello("p1"),
        site(10),
        site(99),
        assert_line("p1", "x.id", 7, 0, None, Some(("t", 1))),
        end("p1"),
    ]
    .concat();
    let t = fold(&out);
    assert!(!t.is_degraded(), "{:?}", t.health);
    assert_eq!(t.properties.len(), 1);
    assert_eq!(t.properties[0].loc.as_ref().unwrap().line, 10);
    assert_eq!(t.health.sites_mismatch, 0);
}

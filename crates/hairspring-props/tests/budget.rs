//! The per-trial event byte budget: 4 MiB by default, a parameter, and the old 1 MiB kept as a named constant.
use hairspring_props::limits::{
    DEFAULT_MAX_EVENT_BYTES_PER_TRIAL, LEGACY_MAX_EVENT_BYTES_PER_TRIAL, MAX_EVENT_BYTES_PER_TRIAL,
};
use hairspring_props::TrialEvents;

const MIB: usize = 1024 * 1024;

fn ev(json: &str) -> String {
    format!("HAIRSPRING_EVENT {json}\n")
}

/// One process declaring `ids` always-properties, each with a padded message and a witness whose thread key is as
/// long as allowed, so catalog plus assert come to roughly 850 bytes per id (the SQLite-sized shape).
fn big_report(ids: usize) -> String {
    let pad = "p".repeat(197);
    let thread = "t".repeat(64);
    let mut out = ev(
        r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"abc","signal_handlers":false}"#,
    );
    for i in 0..ids {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"catalog","proc":"p1","id":"sqlite.property.number.{i:05}","type":"always","must_hit":true,"msg":"{pad}{pad}","loc":{{"file":"sqlite3.c","line":{},"func":"walCheckpoint"}}}}"#,
            i + 1
        ));
    }
    for i in 0..ids {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"assert","proc":"p1","id":"sqlite.property.number.{i:05}","hits":3,"trues":2,"falses":1,"first_false":{{"thread":"{thread}","ord":{},"vals":[1,2,3,4]}},"first_true":{{"thread":"{thread}","ord":1,"vals":[1,2,3,4]}}}}"#,
            i + 1
        ));
    }
    out += &ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{ids},"evals":9,"dropped":0,"threads":0}}"#
    ));
    out
}

#[test]
fn the_default_budget_is_four_mebibytes_and_the_old_one_is_still_named() {
    assert_eq!(DEFAULT_MAX_EVENT_BYTES_PER_TRIAL, 4 * MIB);
    assert_eq!(MAX_EVENT_BYTES_PER_TRIAL, DEFAULT_MAX_EVENT_BYTES_PER_TRIAL);
    assert_eq!(LEGACY_MAX_EVENT_BYTES_PER_TRIAL, MIB);
}

#[test]
fn a_thirteen_hundred_id_report_folds_without_over_limit_by_default() {
    let text = big_report(1300);
    assert!(
        text.len() > MIB,
        "the fixture must not fit the old budget: {}",
        text.len()
    );
    let t = TrialEvents::from_output(text.as_bytes()).fold();
    assert_eq!(t.health.over_limit, 0, "{:?}", t.health);
    assert_eq!(t.properties.len(), 1300);
    assert!(!t.is_degraded(), "{:?}", t.health);
}

#[test]
fn the_same_report_overflows_the_old_budget_when_it_is_asked_for() {
    let text = big_report(1300);
    let t = TrialEvents::from_output(text.as_bytes())
        .fold_with_budget(LEGACY_MAX_EVENT_BYTES_PER_TRIAL);
    assert!(t.health.over_limit > 0);
    assert!(t.health.event_bytes as usize <= LEGACY_MAX_EVENT_BYTES_PER_TRIAL);
    assert!(t.is_degraded());
}

#[test]
fn at_the_default_budget_lines_past_four_mebibytes_are_counted_not_stored() {
    let text = big_report(6000_usize.min(4096));
    let doubled = format!("{text}{}", big_report(1300));
    let t = TrialEvents::from_output(doubled.as_bytes()).fold();
    assert!(t.health.over_limit > 0, "{:?}", t.health);
    assert!(t.health.event_bytes as usize <= DEFAULT_MAX_EVENT_BYTES_PER_TRIAL);
}

#[test]
fn a_zero_budget_stores_nothing() {
    let t = TrialEvents::from_output(big_report(3).as_bytes()).fold_with_budget(0);
    assert_eq!(t.health.accepted, 0);
    assert!(t.health.over_limit > 0);
}

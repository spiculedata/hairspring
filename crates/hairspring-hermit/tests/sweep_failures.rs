//! A sweep stores a failure row for each property's first failing seed, not only for the lowest failing seed, so the
//! claim gate (which needs every first-failure seed confirmed) can see them all.
mod common;
use common::events::rig_with;
use common::props::{by_seed, configure, ev};
use hairspring_core::bundle::Bundle;
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::mode::RunMode;
use hairspring_engine::report::{FailureReport, RunOutcome, RunStatus};
use hairspring_engine::sweep::MAX_FAILURE_ROWS;

/// One process's report of the always-properties `ids`; those in `failing` were false once.
fn report_of(ids: &[&str], failing: &[&str]) -> String {
    let mut out = ev(
        r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"abc","signal_handlers":false}"#,
    );
    for id in ids {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"always","must_hit":true,"msg":"msg of {id}"}}"#
        ));
    }
    for id in ids {
        let falses = u64::from(failing.contains(id));
        let witness = if falses > 0 {
            r#"{"thread":"writer-0","ord":675,"vals":[1,2,3,4]}"#
        } else {
            "null"
        };
        out += &ev(&format!(
            r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":4,"falses":{falses},"first_false":{witness},"first_true":{{"thread":"writer-0","ord":675,"vals":[1,2,3,4]}}}}"#,
            4 + falses
        ));
    }
    out += &ev(r#"{"v":1,"kind":"work","proc":"p1","ops":10}"#);
    let evals: usize = ids.len() * 4 + failing.len();
    out += &ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{},"evals":{evals},"dropped":0,"threads":0}}"#,
        ids.len()
    ));
    out
}

/// A sweep of `trials` seeds from 1 where seed `n` fails the properties listed against it.
fn sweep(
    name: &str,
    ids: &[&str],
    failing: &[(u64, &[&str])],
    trials: u64,
    width: u32,
) -> RunOutcome {
    let arms: Vec<(u64, String)> = failing
        .iter()
        .map(|(seed, props)| (*seed, report_of(ids, props)))
        .collect();
    let script = by_seed(&arms, &report_of(ids, &[]));
    let mut r = rig_with(name, &script, false, vec![], |_| {});
    configure(&mut r, trials, width);
    let (engine, _dir): (Engine, _) = r.engine();
    engine
        .run(
            &RunRequest::new("svc", 1).with_mode(RunMode::Sweep),
            &mut |_| true,
        )
        .unwrap()
}

fn seed_of(f: &FailureReport) -> u64 {
    Bundle::decode(&f.bundle).unwrap().seed
}

fn failing_seeds(out: &RunOutcome) -> Vec<u64> {
    out.failure
        .iter()
        .chain(out.more_failures.iter())
        .map(seed_of)
        .collect()
}

#[test]
fn properties_that_first_fail_at_different_seeds_each_get_a_failure_row() {
    let out = sweep("two-seeds", &["a", "b"], &[(3, &["a"]), (7, &["b"])], 10, 1);
    assert_eq!(out.status, RunStatus::Failed, "{out:?}");
    assert_eq!(failing_seeds(&out), vec![3, 7]);
    assert!(out
        .failure
        .as_ref()
        .unwrap()
        .message
        .contains("sched-seed 3 "));
    assert!(out.more_failures[0].message.contains("sched-seed 7 "));
}

#[test]
fn the_extra_row_is_confirmed_by_rerun_like_the_first() {
    let out = sweep("confirmed", &["a", "b"], &[(3, &["a"]), (7, &["b"])], 10, 1);
    assert!(
        out.more_failures[0].message.contains("reproduced 3/3"),
        "{}",
        out.more_failures[0].message
    );
}

#[test]
fn two_properties_first_failing_at_the_same_seed_share_one_row() {
    let out = sweep("same-seed", &["a", "b"], &[(3, &["a", "b"])], 10, 1);
    assert_eq!(failing_seeds(&out), vec![3]);
    assert!(out.more_failures.is_empty());
}

#[test]
fn a_property_that_fails_again_later_does_not_get_a_second_row() {
    let out = sweep(
        "repeat",
        &["a", "b"],
        &[(3, &["a"]), (5, &["a"]), (8, &["a"])],
        10,
        1,
    );
    assert_eq!(failing_seeds(&out), vec![3]);
}

#[test]
fn a_later_seed_that_fails_an_already_seen_property_and_a_new_one_gets_a_row() {
    let out = sweep(
        "mixed",
        &["a", "b"],
        &[(3, &["a"]), (6, &["a", "b"])],
        10,
        1,
    );
    assert_eq!(failing_seeds(&out), vec![3, 6]);
}

#[test]
fn rows_follow_seed_order_when_trials_run_in_parallel() {
    let out = sweep("parallel", &["a", "b"], &[(4, &["b"]), (2, &["a"])], 8, 4);
    assert_eq!(failing_seeds(&out), vec![2, 4]);
}

#[test]
fn a_single_failing_property_leaves_the_outcome_as_it_was() {
    let out = sweep("single", &["a", "b"], &[(3, &["a"]), (8, &["a"])], 10, 1);
    assert!(out.more_failures.is_empty());
    let json = serde_json::to_value(&out).unwrap();
    assert!(json.get("more_failures").is_none(), "{json}");
}

#[test]
fn the_number_of_failure_rows_is_bounded() {
    let count = MAX_FAILURE_ROWS + 4;
    let ids: Vec<String> = (0..count).map(|i| format!("p{i:02}")).collect();
    let names: Vec<&str> = ids.iter().map(String::as_str).collect();
    let failing: Vec<(u64, Vec<&str>)> = names
        .iter()
        .enumerate()
        .map(|(i, id)| (i as u64 + 1, vec![*id]))
        .collect();
    let arms: Vec<(u64, &[&str])> = failing.iter().map(|(s, v)| (*s, v.as_slice())).collect();
    let out = sweep("bounded", &names, &arms, count as u64 + 2, 1);
    assert_eq!(failing_seeds(&out).len(), MAX_FAILURE_ROWS);
}

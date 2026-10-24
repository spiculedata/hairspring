//! Stored confirmation checks (schema version 5): every `Store` must keep them the same way.
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_checks(make: &dyn Fn() -> Arc<dyn Store>) {
    a_confirmation_is_stored_and_listed_in_order(&*make());
    an_unknown_failure_is_not_found_and_nothing_is_written(&*make());
    storing_a_confirmation_again_changes_nothing(&*make());
    a_confirmation_that_does_not_fit_is_stored_not_at_all(&*make());
    the_number_of_checks_in_one_confirmation_is_capped(&*make());
    a_full_failure_drops_its_oldest_confirmations_for_a_new_one(&*make());
    confirmations_written_in_the_same_millisecond_list_in_the_order_written(&*make());
    a_confirmations_id_follows_its_content(&*make());
    later_confirmations_list_after_earlier_ones(&*make());
    hostile_text_is_stored_verbatim(&*make());
    concurrent_identical_confirmations_store_once(make());
}

fn failure(s: &dyn Store) -> FailureRecord {
    let run = s
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 7,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = s.put_blob(b"bundle").unwrap();
    s.add_failure(
        &NewFailure {
            run_id: run.id,
            target: "t".into(),
            message: "m".into(),
            original_len: 1,
            shrunk_len: 1,
            bundle_hash: bundle,
            steps_json: "[]".into(),
        },
        2,
    )
    .unwrap()
}

fn check(kind: &str, status: &str) -> NewCheck {
    NewCheck {
        kind: kind.into(),
        status: status.into(),
        passed: 3,
        failed: 0,
        inconclusive: 1,
        command: "hermit run --sched-seed=3 -- ./svc".into(),
        detail_json: r#"{"note":"x"}"#.into(),
        artefacts_json: r#"["ab12"]"#.into(),
        environment_json: r#"{"host.container":"none"}"#.into(),
        started_at: 100,
        finished_at: 250,
    }
}

fn confirmation(id: &str, failure_id: &str, checks: Vec<NewCheck>) -> NewConfirmation {
    NewConfirmation {
        id: id.into(),
        failure_id: failure_id.into(),
        runner: "local-1".into(),
        runner_trusted: true,
        checks,
    }
}

fn pair() -> Vec<NewCheck> {
    vec![
        check("same_seed_rerun", "pass"),
        check("bundle_replay", "inconclusive"),
    ]
}

pub fn a_confirmation_is_stored_and_listed_in_order(s: &dyn Store) {
    let f = failure(s);
    assert!(s.list_failure_checks(&f.id).unwrap().is_empty());
    let c = confirmation("conf_a", &f.id, pair());
    assert_eq!(
        s.add_confirmation(&c, 500).unwrap(),
        ConfirmationOutcome::Stored
    );
    let rows = s.list_failure_checks(&f.id).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|r| (r.ordinal, r.kind.as_str()))
            .collect::<Vec<_>>(),
        [(0, "same_seed_rerun"), (1, "bundle_replay")]
    );
    let first = &rows[0];
    assert_eq!(first.failure_id, f.id);
    assert_eq!(first.confirmation_id, "conf_a");
    assert_eq!(first.status, "pass");
    assert_eq!((first.passed, first.failed, first.inconclusive), (3, 0, 1));
    assert_eq!(first.command, c.checks[0].command);
    assert_eq!(first.detail_json, r#"{"note":"x"}"#);
    assert_eq!(first.artefacts_json, r#"["ab12"]"#);
    assert_eq!(first.environment_json, r#"{"host.container":"none"}"#);
    assert_eq!(
        (first.started_at, first.finished_at, first.created_at),
        (100, 250, 500)
    );
    assert_eq!(
        (first.runner.as_str(), first.runner_trusted),
        ("local-1", true)
    );
    assert_ne!(rows[0].id, rows[1].id);
}

pub fn an_unknown_failure_is_not_found_and_nothing_is_written(s: &dyn Store) {
    let err = s
        .add_confirmation(&confirmation("conf_x", "fail_nope", pair()), 5)
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound(_)), "{err}");
    assert!(s.list_failure_checks("fail_nope").unwrap().is_empty());
}

pub fn storing_a_confirmation_again_changes_nothing(s: &dyn Store) {
    let f = failure(s);
    let c = confirmation("conf_a", &f.id, pair());
    s.add_confirmation(&c, 500).unwrap();
    assert_eq!(
        s.add_confirmation(&c, 900).unwrap(),
        ConfirmationOutcome::Duplicate
    );
    let rows = s.list_failure_checks(&f.id).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.created_at == 500),
        "the first write stands"
    );
    let mut other = pair();
    other[0].status = "fail".into();
    let clash = confirmation("conf_a", &f.id, other);
    assert!(
        s.add_confirmation(&clash, 950).is_err(),
        "same id, other content"
    );
    assert_eq!(s.list_failure_checks(&f.id).unwrap(), rows);
}

pub fn a_confirmation_that_does_not_fit_is_stored_not_at_all(s: &dyn Store) {
    let f = failure(s);
    let mut checks = pair();
    checks[1].detail_json = "x".repeat(MAX_CHECK_TEXT_BYTES + 1);
    let err = s
        .add_confirmation(&confirmation("conf_big", &f.id, checks), 5)
        .unwrap_err();
    assert!(matches!(err, StoreError::LimitExceeded { .. }), "{err}");
    assert!(
        s.list_failure_checks(&f.id).unwrap().is_empty(),
        "all or nothing"
    );
    for bad in [
        confirmation("", &f.id, pair()),
        confirmation("conf_e", &f.id, vec![]),
        confirmation(&"c".repeat(MAX_CHECK_TAG_BYTES + 1), &f.id, pair()),
    ] {
        assert!(s.add_confirmation(&bad, 5).is_err());
    }
    assert!(s.list_failure_checks(&f.id).unwrap().is_empty());
}

pub fn the_number_of_checks_in_one_confirmation_is_capped(s: &dyn Store) {
    let f = failure(s);
    let too_many = vec![check("same_seed_rerun", "pass"); MAX_CHECKS_PER_CONFIRMATION + 1];
    let err = s
        .add_confirmation(&confirmation("conf_many", &f.id, too_many), 5)
        .unwrap_err();
    assert!(matches!(err, StoreError::LimitExceeded { .. }), "{err}");
}

/// A failure that has reached the cap does not refuse every later confirmation: the oldest whole confirmations are
/// dropped to make room, so the newest checks are always the ones kept.
pub fn a_full_failure_drops_its_oldest_confirmations_for_a_new_one(s: &dyn Store) {
    let f = failure(s);
    let per_confirmation = vec![check("same_seed_rerun", "pass"); MAX_CHECKS_PER_CONFIRMATION];
    let rounds = MAX_CHECKS_PER_FAILURE as usize / MAX_CHECKS_PER_CONFIRMATION;
    for i in 0..rounds {
        let c = confirmation(&format!("conf_{i}"), &f.id, per_confirmation.clone());
        s.add_confirmation(&c, 10 + i as i64).unwrap();
    }
    assert_eq!(
        s.list_failure_checks(&f.id).unwrap().len() as u64,
        MAX_CHECKS_PER_FAILURE
    );
    let outcome = s
        .add_confirmation(&confirmation("conf_over", &f.id, pair()), 999)
        .unwrap();
    assert_eq!(outcome, ConfirmationOutcome::Stored);
    let rows = s.list_failure_checks(&f.id).unwrap();
    assert!(rows.len() as u64 <= MAX_CHECKS_PER_FAILURE);
    assert!(rows.iter().all(|r| r.confirmation_id != "conf_0"), "oldest");
    assert!(rows.iter().any(|r| r.confirmation_id == "conf_1"));
    assert_eq!(rows.last().unwrap().confirmation_id, "conf_over");
    assert_eq!(
        rows.iter()
            .filter(|r| r.confirmation_id == "conf_over")
            .count(),
        2
    );
}

/// Two confirmations that carry the same time list in the order they were written, whatever their ids.
pub fn confirmations_written_in_the_same_millisecond_list_in_the_order_written(s: &dyn Store) {
    let f = failure(s);
    s.add_confirmation(&confirmation("conf_z", &f.id, pair()), 500)
        .unwrap();
    s.add_confirmation(&confirmation("conf_a", &f.id, pair()), 500)
        .unwrap();
    s.add_confirmation(&confirmation("conf_m", &f.id, pair()), 500)
        .unwrap();
    let mut order: Vec<String> = Vec::new();
    for r in s.list_failure_checks(&f.id).unwrap() {
        if order.last() != Some(&r.confirmation_id) {
            order.push(r.confirmation_id);
        }
    }
    assert_eq!(order, ["conf_z", "conf_a", "conf_m"]);
}

/// The id a confirmation is stored under comes from what it is, so a retry of the same report is the same confirmation.
pub fn a_confirmations_id_follows_its_content(s: &dyn Store) {
    let f = failure(s);
    let id = |runner: &str, trusted: bool, checks: &[NewCheck]| {
        NewConfirmation::content_id(&f.id, runner, trusted, checks)
    };
    let base = id("local-1", true, &pair());
    assert_eq!(base, id("local-1", true, &pair()));
    assert!(base.starts_with("conf_") && base.len() <= MAX_CHECK_TAG_BYTES);
    assert_ne!(base, id("local-2", true, &pair()));
    assert_ne!(base, id("local-1", false, &pair()));
    assert_ne!(
        base,
        NewConfirmation::content_id("fail_other", "local-1", true, &pair())
    );
    let mut changed = pair();
    changed[1].passed += 1;
    assert_ne!(base, id("local-1", true, &changed));
    let c = confirmation(&base, &f.id, pair());
    assert_eq!(
        s.add_confirmation(&c, 1).unwrap(),
        ConfirmationOutcome::Stored
    );
    assert_eq!(
        s.add_confirmation(&c, 2).unwrap(),
        ConfirmationOutcome::Duplicate
    );
}

pub fn later_confirmations_list_after_earlier_ones(s: &dyn Store) {
    let f = failure(s);
    s.add_confirmation(
        &confirmation("conf_late", &f.id, vec![check("fixed_control", "pass")]),
        900,
    )
    .unwrap();
    s.add_confirmation(&confirmation("conf_early", &f.id, pair()), 100)
        .unwrap();
    let order: Vec<_> = s
        .list_failure_checks(&f.id)
        .unwrap()
        .into_iter()
        .map(|r| (r.confirmation_id, r.ordinal))
        .collect();
    assert_eq!(
        order,
        [
            ("conf_early".to_string(), 0),
            ("conf_early".to_string(), 1),
            ("conf_late".to_string(), 0)
        ]
    );
}

pub fn hostile_text_is_stored_verbatim(s: &dyn Store) {
    let f = failure(s);
    let nasty = "'; DROP TABLE failure_checks; --\n\u{0}\u{202E}\"";
    let mut c = check("same_seed_rerun", "pass");
    c.command = nasty.replace('\u{0}', "");
    c.detail_json = serde_json_like(nasty);
    s.add_confirmation(&confirmation("conf_h", &f.id, vec![c.clone()]), 5)
        .unwrap();
    let rows = s.list_failure_checks(&f.id).unwrap();
    assert_eq!(rows[0].command, c.command);
    assert_eq!(rows[0].detail_json, c.detail_json);
}

/// A JSON string literal holding `text`, without a JSON library in the test.
fn serde_json_like(text: &str) -> String {
    let escaped: String = text
        .chars()
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\n' => "\\n".to_string(),
            '\u{0}' => "\\u0000".to_string(),
            other => other.to_string(),
        })
        .collect();
    format!("\"{escaped}\"")
}

pub fn concurrent_identical_confirmations_store_once(s: Arc<dyn Store>) {
    let f = failure(&*s);
    let c = confirmation("conf_race", &f.id, pair());
    let outcomes: Vec<_> = (0..4)
        .map(|_| {
            let (s, c) = (s.clone(), c.clone());
            std::thread::spawn(move || s.add_confirmation(&c, 50).unwrap())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let stored = outcomes
        .iter()
        .filter(|o| **o == ConfirmationOutcome::Stored)
        .count();
    assert_eq!(stored, 1, "{outcomes:?}");
    assert_eq!(s.list_failure_checks(&f.id).unwrap().len(), 2);
}

//! Run inputs (schema version 8): the hashes of what a run executed with, recorded once per run. Every `Store` must
//! accept, refuse, repeat and read them the same way, and `hairspring gc` must delete them with their run.
use hairspring_store::*;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn run_all_run_inputs(make: &dyn Fn() -> Arc<dyn Store>) {
    inputs_are_stored_once_and_read_back(&*make());
    the_same_inputs_again_change_nothing(&*make());
    different_inputs_for_a_run_that_has_some_are_refused(&*make());
    an_unknown_run_is_refused(&*make());
    a_run_without_inputs_reads_as_none(&*make());
    a_record_the_store_cannot_hold_is_refused_and_nothing_is_written(&*make());
    the_trust_of_the_first_reporter_stays(&*make());
    inputs_go_with_their_run_when_it_is_deleted(&*make());
    hostile_text_is_never_spliced(&*make());
    a_newer_attempt_replaces_the_inputs_and_an_older_one_is_stale(&*make());
}

/// A job leased again is a new attempt, and its trials replace the earlier attempt's. Its inputs do too: the newer
/// attempt's row replaces the older one's whole (fields, reporter and trust), the same attempt keeps the repeat and
/// conflict rules, and an older attempt is refused as stale.
fn a_newer_attempt_replaces_the_inputs_and_an_older_one_is_stale(s: &dyn Store) {
    let run = new_run(s, 10);
    let first = NewRunInputs {
        reporter_trusted: false,
        attempt: 1,
        ..inputs(&run.id)
    };
    assert_eq!(
        s.put_run_inputs(&first, 100).unwrap(),
        RunInputsOutcome::Stored
    );
    let mut second = NewRunInputs {
        reporter_id: "wrk_0000000000000002".into(),
        reporter_trusted: true,
        attempt: 2,
        ..first.clone()
    };
    second.fields.insert("hermit_sha".into(), digest('d'));
    second.missing.clear();
    assert_eq!(
        s.put_run_inputs(&second, 200).unwrap(),
        RunInputsOutcome::Replaced
    );
    let stored = s.get_run_inputs(&run.id).unwrap().unwrap();
    assert_eq!(stored.attempt, 2);
    assert_eq!(stored.fields, second.fields);
    assert!(stored.missing.is_empty());
    assert_eq!(
        (
            stored.reporter_id.as_str(),
            stored.reporter_trusted,
            stored.created_at
        ),
        ("wrk_0000000000000002", true, 200)
    );
    // The same attempt again: a repeat, then a conflict, and the stored row stays.
    assert_eq!(
        s.put_run_inputs(&second, 300).unwrap(),
        RunInputsOutcome::Duplicate
    );
    let mut conflicting = second.clone();
    conflicting.fields.insert("hermit_sha".into(), digest('e'));
    assert_eq!(
        rejected(s.put_run_inputs(&conflicting, 300).unwrap()),
        RunInputsRejection::Conflict
    );
    // An older attempt is stale, even with the same content as the one it lost to.
    for old in [
        first.clone(),
        NewRunInputs {
            attempt: 1,
            ..second.clone()
        },
    ] {
        assert_eq!(
            rejected(s.put_run_inputs(&old, 400).unwrap()),
            RunInputsRejection::Stale
        );
    }
    assert_eq!(s.get_run_inputs(&run.id).unwrap().unwrap().attempt, 2);
    // A later attempt replaces again, with its own trust.
    let third = NewRunInputs {
        attempt: 5,
        ..first
    };
    assert_eq!(
        s.put_run_inputs(&third, 500).unwrap(),
        RunInputsOutcome::Replaced
    );
    assert!(!s.get_run_inputs(&run.id).unwrap().unwrap().reporter_trusted);
}

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn new_run(s: &dyn Store, created: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: "t".into(),
            backend: "sim".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        created,
    )
    .unwrap()
}

fn inputs(run: &str) -> NewRunInputs {
    NewRunInputs {
        run_id: run.to_string(),
        scheme: "hermit/1".into(),
        fields: BTreeMap::from([
            ("hermit_sha".to_string(), digest('a')),
            ("sched_skid_margin".to_string(), digest('b')),
        ]),
        missing: vec!["loader_libc_sha".into()],
        reporter_id: "wrk_0000000000000001".into(),
        reporter_trusted: true,
        attempt: 1,
    }
}

fn rejected(outcome: RunInputsOutcome) -> RunInputsRejection {
    match outcome {
        RunInputsOutcome::Rejected(why) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn inputs_are_stored_once_and_read_back(s: &dyn Store) {
    let run = new_run(s, 10);
    let new = inputs(&run.id);
    assert_eq!(
        s.put_run_inputs(&new, 1234).unwrap(),
        RunInputsOutcome::Stored
    );
    let stored = s.get_run_inputs(&run.id).unwrap().expect("stored");
    assert_eq!(stored.run_id, run.id);
    assert_eq!(stored.scheme, "hermit/1");
    assert_eq!(stored.fields, new.fields);
    assert_eq!(stored.missing, vec!["loader_libc_sha".to_string()]);
    assert_eq!(stored.reporter_id, "wrk_0000000000000001");
    assert!(stored.reporter_trusted);
    assert_eq!(stored.created_at, 1234);
}

fn the_same_inputs_again_change_nothing(s: &dyn Store) {
    let run = new_run(s, 10);
    let new = inputs(&run.id);
    assert_eq!(
        s.put_run_inputs(&new, 100).unwrap(),
        RunInputsOutcome::Stored
    );
    // Another reporter and time, same content: a repeat, and the first row stays as it was.
    let again = NewRunInputs {
        reporter_id: "wrk_0000000000000002".into(),
        reporter_trusted: false,
        // Missing names are a set: order and repeats do not make a different record.
        missing: vec!["loader_libc_sha".into(), "loader_libc_sha".into()],
        ..new.clone()
    };
    assert_eq!(
        s.put_run_inputs(&again, 200).unwrap(),
        RunInputsOutcome::Duplicate
    );
    let stored = s.get_run_inputs(&run.id).unwrap().unwrap();
    assert_eq!(
        (
            stored.reporter_id.as_str(),
            stored.reporter_trusted,
            stored.created_at
        ),
        ("wrk_0000000000000001", true, 100)
    );
}

fn different_inputs_for_a_run_that_has_some_are_refused(s: &dyn Store) {
    let run = new_run(s, 10);
    let first = inputs(&run.id);
    s.put_run_inputs(&first, 100).unwrap();
    let mut changed_field = first.clone();
    changed_field
        .fields
        .insert("sched_skid_margin".into(), digest('c'));
    let mut changed_missing = first.clone();
    changed_missing.missing.clear();
    let mut changed_scheme = first.clone();
    changed_scheme.scheme = "wasm/1".into();
    for other in [changed_field, changed_missing, changed_scheme] {
        assert_eq!(
            rejected(s.put_run_inputs(&other, 300).unwrap()),
            RunInputsRejection::Conflict
        );
    }
    let stored = s.get_run_inputs(&run.id).unwrap().unwrap();
    assert_eq!(stored.fields, first.fields, "stored rows never change");
    assert_eq!(stored.scheme, "hermit/1");
}

fn an_unknown_run_is_refused(s: &dyn Store) {
    assert_eq!(
        rejected(
            s.put_run_inputs(&inputs("run_0000000000000000"), 1)
                .unwrap()
        ),
        RunInputsRejection::RunNotFound
    );
    assert!(s.get_run_inputs("run_0000000000000000").unwrap().is_none());
}

fn a_run_without_inputs_reads_as_none(s: &dyn Store) {
    let run = new_run(s, 10);
    assert!(s.get_run_inputs(&run.id).unwrap().is_none());
}

fn a_record_the_store_cannot_hold_is_refused_and_nothing_is_written(s: &dyn Store) {
    let run = new_run(s, 10);
    let good = inputs(&run.id);
    let many: BTreeMap<String, String> = (0..=MAX_RUN_INPUT_FIELDS)
        .map(|i| (format!("field_{i}"), digest('a')))
        .collect();
    let long_name = "a".repeat(MAX_RUN_INPUT_NAME_BYTES + 1);
    let bad = [
        NewRunInputs {
            scheme: "hermit/2".into(),
            ..good.clone()
        },
        NewRunInputs {
            scheme: String::new(),
            ..good.clone()
        },
        NewRunInputs {
            fields: BTreeMap::new(),
            ..good.clone()
        },
        NewRunInputs {
            fields: many,
            missing: Vec::new(),
            ..good.clone()
        },
        NewRunInputs {
            fields: BTreeMap::from([(long_name.clone(), digest('a'))]),
            ..good.clone()
        },
        NewRunInputs {
            missing: vec![long_name],
            ..good.clone()
        },
        // A value that is not a lower-case SHA-256 digest: a plain value must never be stored.
        NewRunInputs {
            fields: BTreeMap::from([("cwd".to_string(), "/home/someone".to_string())]),
            ..good.clone()
        },
        NewRunInputs {
            fields: BTreeMap::from([("cwd".to_string(), digest('A'))]),
            ..good.clone()
        },
        NewRunInputs {
            fields: BTreeMap::from([("cwd".to_string(), "a".repeat(63))]),
            ..good.clone()
        },
        // Names: lower-case letters, digits and underscore, starting with a letter.
        NewRunInputs {
            fields: BTreeMap::from([("Cwd".to_string(), digest('a'))]),
            ..good.clone()
        },
        NewRunInputs {
            fields: BTreeMap::from([("c w d".to_string(), digest('a'))]),
            ..good.clone()
        },
        NewRunInputs {
            fields: BTreeMap::from([(String::new(), digest('a'))]),
            ..good.clone()
        },
        NewRunInputs {
            missing: vec!["cwd\n".into()],
            ..good.clone()
        },
        // A name cannot be both recorded and missing.
        NewRunInputs {
            missing: vec!["hermit_sha".into()],
            ..good.clone()
        },
        NewRunInputs {
            reporter_id: String::new(),
            ..good.clone()
        },
        NewRunInputs {
            reporter_id: "x".repeat(129),
            ..good.clone()
        },
        NewRunInputs {
            reporter_id: "wrk\0".into(),
            ..good.clone()
        },
        NewRunInputs {
            run_id: String::new(),
            ..good.clone()
        },
    ];
    for (n, new) in bad.into_iter().enumerate() {
        assert!(
            matches!(
                s.put_run_inputs(&new, 1).unwrap(),
                RunInputsOutcome::Rejected(RunInputsRejection::Invalid(_))
            ),
            "case {n} was accepted"
        );
    }
    assert!(s.get_run_inputs(&run.id).unwrap().is_none());
    // The largest record the limits allow is accepted.
    let widest = NewRunInputs {
        fields: (0..MAX_RUN_INPUT_FIELDS)
            .map(|i| {
                (
                    format!("{}{i:02}", "a".repeat(MAX_RUN_INPUT_NAME_BYTES - 2)),
                    digest('f'),
                )
            })
            .collect(),
        missing: Vec::new(),
        ..good
    };
    assert_eq!(
        s.put_run_inputs(&widest, 2).unwrap(),
        RunInputsOutcome::Stored
    );
    assert_eq!(
        s.get_run_inputs(&run.id).unwrap().unwrap().fields.len(),
        MAX_RUN_INPUT_FIELDS
    );
}

fn the_trust_of_the_first_reporter_stays(s: &dyn Store) {
    let run = new_run(s, 10);
    let untrusted = NewRunInputs {
        reporter_trusted: false,
        ..inputs(&run.id)
    };
    s.put_run_inputs(&untrusted, 5).unwrap();
    let trusted_again = NewRunInputs {
        reporter_trusted: true,
        ..untrusted.clone()
    };
    assert_eq!(
        s.put_run_inputs(&trusted_again, 6).unwrap(),
        RunInputsOutcome::Duplicate
    );
    assert!(!s.get_run_inputs(&run.id).unwrap().unwrap().reporter_trusted);
}

fn inputs_go_with_their_run_when_it_is_deleted(s: &dyn Store) {
    let old = new_run(s, 999);
    let kept = new_run(s, 20_000);
    for run in [&old, &kept] {
        s.put_run_inputs(&inputs(&run.id), 5).unwrap();
    }
    assert!(s
        .finish_run(&old.id, RunState::Passed, 1, 1, None, 1_000)
        .unwrap());
    assert!(s
        .finish_run(&kept.id, RunState::Passed, 1, 1, None, 20_001)
        .unwrap());
    let report = s
        .retention_delete(std::slice::from_ref(&old.id), 10_000, false)
        .unwrap();
    assert!(report.applied);
    assert!(s.get_run(&old.id).unwrap().is_none());
    assert!(s.get_run_inputs(&old.id).unwrap().is_none());
    assert!(s.get_run_inputs(&kept.id).unwrap().is_some());
}

/// Names and ids are bound as parameters, never spliced into SQL.
fn hostile_text_is_never_spliced(s: &dyn Store) {
    let run = new_run(s, 10);
    let evil = "x'); DROP TABLE run_inputs;--";
    for id in [evil, "run_1' OR '1'='1"] {
        let new = NewRunInputs {
            run_id: id.to_string(),
            ..inputs(&run.id)
        };
        assert_eq!(
            rejected(s.put_run_inputs(&new, 1).unwrap()),
            RunInputsRejection::RunNotFound
        );
        assert!(s.get_run_inputs(id).unwrap().is_none());
    }
    let new = NewRunInputs {
        reporter_id: evil.to_string(),
        ..inputs(&run.id)
    };
    assert_eq!(s.put_run_inputs(&new, 1).unwrap(), RunInputsOutcome::Stored);
    assert_eq!(
        s.get_run_inputs(&run.id).unwrap().unwrap().reporter_id,
        evil
    );
}

//! Run inputs at the service: what a worker may submit, who may submit it, what is kept, and what a reader is told.
//! Digests are kept and never served; refusals use fixed words and never repeat what the caller sent.
mod common;

use common::*;
use hairspring_core::point::{CommonInputs, HermitInputs, IdentityDraft, Recorded, TmpPolicy};
use hairspring_service::{RunInputsSubmission, ServiceError};
use hairspring_store::RunState;
use std::collections::BTreeMap;

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn sim_submission(target: &str) -> RunInputsSubmission {
    let draft = IdentityDraft::Sim(CommonInputs {
        hairspring_sha: Recorded::Value(digest('a')),
        target: Recorded::Value(target.into()),
    });
    RunInputsSubmission::from_hashes(&draft.input_hashes())
}

fn refused(result: Result<hairspring_service::RunInputsReceipt, ServiceError>) -> String {
    match result {
        Err(ServiceError::InvalidRequest(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_local_worker_records_inputs_and_the_view_names_them_without_values() {
    let (svc, _store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let receipt = svc
        .record_run_inputs(&job.job_id, &local(), &sim_submission("lock-lease"))
        .unwrap();
    assert_eq!(receipt.outcome, "stored");
    let view = svc.run_inputs_view(&job.run_id).unwrap();
    assert!(view.recorded);
    assert_eq!(view.scheme.as_deref(), Some("sim/1"));
    assert_eq!(view.fields, ["backend", "hairspring_sha", "target"]);
    assert!(view.missing.is_empty());
    assert_eq!(view.reporter_trusted, Some(true));
    let served = serde_json::to_string(&view).unwrap();
    assert!(!served.contains(&digest('a')), "digests are not served");
}

#[test]
fn a_run_with_no_inputs_reads_as_not_recorded_never_as_a_guess() {
    let (svc, _store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let view = svc.run_inputs_view(&job.run_id).unwrap();
    assert!(!view.recorded);
    assert_eq!(
        (
            view.scheme,
            view.fields,
            view.missing,
            view.reporter_trusted
        ),
        (None, Vec::new(), Vec::new(), None)
    );
    assert!(matches!(
        svc.run_inputs_view("run_0000000000000000"),
        Err(ServiceError::NotFound(_))
    ));
}

#[test]
fn sending_the_same_inputs_again_is_accepted_and_different_ones_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let first = sim_submission("lock-lease");
    svc.record_run_inputs(&job.job_id, &local(), &first)
        .unwrap();
    let again = svc
        .record_run_inputs(&job.job_id, &local(), &first)
        .unwrap();
    assert_eq!(again.outcome, "duplicate");
    let mut other = first.clone();
    other.fields.insert("target".into(), digest('b'));
    let why = refused(svc.record_run_inputs(&job.job_id, &local(), &other));
    assert!(why.contains("already has different inputs"), "{why}");
    let kept = store.get_run_inputs(&job.run_id).unwrap().unwrap();
    assert_eq!(kept.fields["target"], first.fields["target"]);
}

#[test]
fn only_the_worker_holding_the_lease_may_record_and_a_remote_one_must_prove_itself() {
    let (svc, store, engine) = setup();
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    let (other, other_secret) = svc.register_remote_worker("other", "{}").unwrap();
    let job = lease(&svc, &engine, &remote.id);
    let submission = sim_submission("lock-lease");
    // Not the lease holder.
    assert!(matches!(
        svc.record_run_inputs_with_secret(&job.job_id, &other.id, Some(&other_secret), &submission),
        Err(ServiceError::InvalidRequest(_))
    ));
    // The right worker with a missing or wrong secret.
    for bad in [None, Some("wrong"), Some(other_secret.as_str())] {
        let why =
            refused(svc.record_run_inputs_with_secret(&job.job_id, &remote.id, bad, &submission));
        assert_eq!(why, "invalid worker credential");
    }
    assert!(store.get_run_inputs(&job.run_id).unwrap().is_none());
    svc.record_run_inputs_with_secret(&job.job_id, &remote.id, Some(&secret), &submission)
        .unwrap();
    let view = svc.run_inputs_view(&job.run_id).unwrap();
    assert_eq!(
        view.reporter_trusted,
        Some(false),
        "a remote worker is untrusted until an administrator says so"
    );
    // An unknown job is not found, and a finished run's lease is gone.
    assert!(matches!(
        svc.record_run_inputs(&job_id_like(), &local(), &submission),
        Err(ServiceError::NotFound(_))
    ));
}

fn job_id_like() -> String {
    "job_0000000000000000".to_string()
}

#[test]
fn trust_is_the_workers_trust_when_the_inputs_are_recorded() {
    let (svc, _store, engine) = setup();
    let (remote, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    svc.set_worker_trust(&remote.id, true).unwrap();
    let job = lease(&svc, &engine, &remote.id);
    svc.record_run_inputs_with_secret(
        &job.job_id,
        &remote.id,
        Some(&secret),
        &sim_submission("lock-lease"),
    )
    .unwrap();
    svc.set_worker_trust(&remote.id, false).unwrap();
    assert_eq!(
        svc.run_inputs_view(&job.run_id).unwrap().reporter_trusted,
        Some(true),
        "a snapshot at recording, as for trial rows"
    );
}

#[test]
fn the_scheme_must_belong_to_the_runs_backend() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut wasm = sim_submission("lock-lease");
    wasm.scheme = "wasm/1".into();
    wasm.fields.insert("module_sha".into(), digest('c'));
    let why = refused(svc.record_run_inputs(&job.job_id, &local(), &wasm));
    assert_eq!(why, "the inputs do not belong to this run's backend");
    assert!(store.get_run_inputs(&job.run_id).unwrap().is_none());
}

#[test]
fn hostile_submissions_are_refused_in_fixed_words_that_never_repeat_them() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let plant = "TOPSECRET<script>alert(1)</script>";
    let good = sim_submission("lock-lease");
    let cases: Vec<(&str, RunInputsSubmission)> = vec![
        (
            "unknown scheme",
            RunInputsSubmission {
                scheme: plant.into(),
                ..good.clone()
            },
        ),
        (
            "unknown field",
            RunInputsSubmission {
                fields: BTreeMap::from([(plant.to_string(), digest('a'))]),
                ..good.clone()
            },
        ),
        (
            "a field this scheme does not have",
            RunInputsSubmission {
                fields: BTreeMap::from([("hermit_sha".to_string(), digest('a'))]),
                ..good.clone()
            },
        ),
        (
            "value is not a digest",
            RunInputsSubmission {
                fields: BTreeMap::from([("target".to_string(), plant.to_string())]),
                ..good.clone()
            },
        ),
        (
            "unknown missing name",
            RunInputsSubmission {
                missing: vec![plant.into()],
                ..good.clone()
            },
        ),
        (
            "recorded and missing",
            RunInputsSubmission {
                missing: vec!["target".into()],
                ..good.clone()
            },
        ),
        (
            "nothing recorded",
            RunInputsSubmission {
                fields: BTreeMap::new(),
                ..good.clone()
            },
        ),
    ];
    for (label, bad) in cases {
        let why = refused(svc.record_run_inputs(&job.job_id, &local(), &bad));
        assert!(!why.is_empty(), "{label}");
        assert!(
            !why.contains("TOPSECRET") && !why.contains("script"),
            "{label}: {why}"
        );
        assert!(why.len() < 100, "{label}: one short line");
        assert!(
            store.get_run_inputs(&job.run_id).unwrap().is_none(),
            "{label}: nothing stored"
        );
    }
}

#[test]
fn no_environment_value_reaches_what_is_stored() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    // A hermit-shaped draft with a planted value, for a run whose backend is simulated: refused by backend, so build
    // the sim run's submission from a draft that only carries the planted text as its target name.
    let secret = "TOPSECRET-env-value-0042";
    let hermit = IdentityDraft::Hermit(
        CommonInputs {
            hairspring_sha: Recorded::Value(digest('a')),
            target: Recorded::Value("t".into()),
        },
        HermitInputs {
            guest_env: Recorded::Value(vec![("API_TOKEN".into(), secret.into())]),
            cwd: Recorded::Value(format!("/srv/{secret}")),
            tmp_policy: Recorded::Value(TmpPolicy::Path(format!("/tmp/{secret}"))),
            ..HermitInputs::default()
        },
    );
    let submission = RunInputsSubmission::from_hashes(&hermit.input_hashes());
    assert!(submission.fields.contains_key("guest_env"));
    let wire = serde_json::to_string(&submission).unwrap();
    assert!(
        !wire.contains(secret) && !wire.contains("API_TOKEN"),
        "{wire}"
    );
    // Recorded for the sim run under a sim scheme: take the shape, keep the hashes.
    let sim = sim_submission(secret);
    svc.record_run_inputs(&job.job_id, &local(), &sim).unwrap();
    let stored = format!("{:?}", store.get_run_inputs(&job.run_id).unwrap());
    assert!(!stored.contains(secret), "{stored}");
}

#[test]
fn a_run_executed_here_records_its_inputs_without_being_asked() {
    let (svc, _store, engine) = setup();
    let run = svc
        .run_sync(
            &engine,
            &hairspring_service::StartRun {
                target: "lock-lease".into(),
                seed: Some(1),
                budget: Some(hairspring_engine::config::Budget {
                    max_steps: 1000,
                    max_trials: 3,
                    wall_secs: 30,
                }),
            },
        )
        .unwrap();
    assert!(run.run.state.is_terminal());
    assert_ne!(run.run.state, RunState::Queued);
    let view = svc.run_inputs_view(&run.run.id).unwrap();
    assert!(view.recorded);
    assert_eq!(view.scheme.as_deref(), Some("sim/1"));
    assert_eq!(view.fields, ["backend", "hairspring_sha", "target"]);
    assert_eq!(view.reporter_trusted, Some(true));
}

//! Run inputs at the service: what a worker may submit, who may submit it, what is kept, and what a reader is told.
//! Digests are kept and never served; refusals use fixed words and never repeat what the caller sent.
mod common;

use common::*;
use hairspring_core::point::{CommonInputs, HermitInputs, IdentityDraft, Recorded, TmpPolicy};
use hairspring_service::Service;
use hairspring_service::{RunInputsSubmission, ServiceError};
use hairspring_store::{now_ms, RunState, SqliteStore, Store};
use std::collections::BTreeMap;
use std::sync::Arc;

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn sim_submission(target: &str) -> RunInputsSubmission {
    sim_submission_with(target, 'a')
}

/// A complete report for a simulated run of `target`, whose executable digest is `sha` repeated.
fn sim_submission_with(target: &str, sha: char) -> RunInputsSubmission {
    let draft = IdentityDraft::Sim(CommonInputs {
        hairspring_sha: Recorded::Value(digest(sha)),
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
    other.fields.insert("hairspring_sha".into(), digest('b'));
    let why = refused(svc.record_run_inputs(&job.job_id, &local(), &other));
    assert!(why.contains("already has different inputs"), "{why}");
    let kept = store.get_run_inputs(&job.run_id).unwrap().unwrap();
    assert_eq!(
        kept.fields["hairspring_sha"],
        first.fields["hairspring_sha"]
    );
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

/// Reads every byte the database wrote, whatever the column: nothing a guest or an operator put in the environment, a
/// path or a command line may be in any of it.
#[test]
fn no_value_path_or_environment_text_reaches_the_stored_bytes() {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-inputs-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(&dir.join("db.sqlite")).unwrap());
    let svc = Service::new(store.clone());
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = hairspring_engine::engine::Engine::new(
        registry.clone(),
        hairspring_engine::config::Config::from_registry(&registry),
    );
    let job = lease(&svc, &engine, &local());
    let secret = "TOPSECRET-env-value-0042";
    let hermit = IdentityDraft::Hermit(
        CommonInputs {
            hairspring_sha: Recorded::Value(digest('a')),
            target: Recorded::Value(format!("/opt/{secret}/target")),
        },
        HermitInputs {
            guest_env: Recorded::Value(vec![("API_TOKEN".into(), secret.into())]),
            cwd: Recorded::Value(format!("/srv/{secret}")),
            tmp_policy: Recorded::Value(TmpPolicy::Path(format!("/tmp/{secret}"))),
            target_argv: Recorded::Value(vec![format!("/usr/bin/{secret}"), "--flag".into()]),
            hermit_args: Recorded::Value(vec![format!("--arg={secret}")]),
            ..HermitInputs::default()
        },
    );
    let hashes = hermit.input_hashes();
    assert!(hashes.fields.contains_key("guest_env") && hashes.fields.contains_key("cwd"));
    // The store holds whatever digests it is given for the run, whatever its backend: store the hermit-shaped ones, and
    // a sim report through the service.
    let new = hairspring_store::NewRunInputs {
        run_id: job.run_id.clone(),
        scheme: hashes.scheme.clone(),
        fields: hashes.fields.clone(),
        missing: hashes.missing.clone(),
        reporter_id: local(),
        reporter_trusted: true,
        attempt: 1,
    };
    store.put_run_inputs(&new, 1).unwrap();
    drop((svc, store));
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        seen += bytes.len();
        for needle in [
            secret,
            "API_TOKEN",
            "/srv/",
            "/usr/bin/",
            "--flag",
            "--arg=",
        ] {
            assert!(
                !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle} reached a database file"
            );
        }
    }
    assert!(seen > 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_report_that_leaves_out_an_input_its_scheme_has_is_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut partial = sim_submission("lock-lease");
    partial.fields.remove("backend");
    let why = refused(svc.record_run_inputs(&job.job_id, &local(), &partial));
    assert_eq!(why, "the inputs leave out an input this scheme has");
    assert!(store.get_run_inputs(&job.run_id).unwrap().is_none());
    // Naming it as one that could not be recorded makes the report complete.
    partial.missing.push("backend".into());
    svc.record_run_inputs(&job.job_id, &local(), &partial)
        .unwrap();
    let view = svc.run_inputs_view(&job.run_id).unwrap();
    assert_eq!(view.missing, ["backend"]);
}

#[test]
fn a_report_for_another_target_or_backend_is_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let other_target = sim_submission("some-other-target");
    let mut other_backend = sim_submission("lock-lease");
    other_backend.fields.insert("backend".into(), digest('e'));
    for bad in [other_target, other_backend] {
        let why = refused(svc.record_run_inputs(&job.job_id, &local(), &bad));
        assert_eq!(why, "the inputs do not belong to this run");
    }
    assert!(store.get_run_inputs(&job.run_id).unwrap().is_none());
    svc.record_run_inputs(&job.job_id, &local(), &sim_submission("lock-lease"))
        .unwrap();
}

fn one_trial() -> Vec<hairspring_engine::trial_log::TrialReport> {
    vec![Recorder::new().pass(
        1,
        &props("a.id", hairspring_props::AssertKind::Always, 3, 0),
    )]
}

/// An untrusted worker records inputs and sends trials, then fails; a trusted worker leases the same job with another
/// executable. The second attempt's trials replace the first's, so its inputs must too: its report is accepted, and
/// what the run shows is the trusted worker's, not the first worker's.
#[test]
fn a_job_leased_again_has_its_inputs_replaced_by_the_new_attempt() {
    let (svc, store, engine) = setup();
    let (a, a_secret) = svc.register_remote_worker("a", "{}").unwrap();
    let (b, b_secret) = svc.register_remote_worker("b", "{}").unwrap();
    svc.set_worker_trust(&b.id, true).unwrap();
    let job = lease(&svc, &engine, &a.id);
    svc.record_run_inputs_with_secret(
        &job.job_id,
        &a.id,
        Some(&a_secret),
        &sim_submission_with("lock-lease", 'a'),
    )
    .unwrap();
    svc.ingest_trials_with_secret(&job.job_id, &a.id, Some(&a_secret), &one_trial())
        .unwrap();
    assert_eq!(
        svc.run_inputs_view(&job.run_id).unwrap().reporter_trusted,
        Some(false)
    );
    svc.fail(&job.job_id, &a.id, "the worker was lost").unwrap();

    let again = svc.lease(&b.id).unwrap().expect("the job is leased again");
    assert_eq!(again.job_id, job.job_id);
    let receipt = svc
        .record_run_inputs_with_secret(
            &again.job_id,
            &b.id,
            Some(&b_secret),
            &sim_submission_with("lock-lease", 'b'),
        )
        .unwrap();
    assert_eq!(receipt.outcome, "replaced");
    let kept = store.get_run_inputs(&job.run_id).unwrap().unwrap();
    assert_eq!(kept.attempt, 2);
    assert_eq!(kept.reporter_id, b.id);
    assert!(kept.reporter_trusted);
    assert_eq!(
        kept.fields["hairspring_sha"],
        sim_submission_with("lock-lease", 'b').fields["hairspring_sha"]
    );

    // The second attempt's trials start the run over; its own inputs stay.
    svc.ingest_trials_with_secret(&again.job_id, &b.id, Some(&b_secret), &one_trial())
        .unwrap();
    assert_eq!(
        store.get_run_inputs(&job.run_id).unwrap().unwrap().attempt,
        2
    );
    // Sending the same report again is still a repeat.
    let repeat = svc
        .record_run_inputs_with_secret(
            &again.job_id,
            &b.id,
            Some(&b_secret),
            &sim_submission_with("lock-lease", 'b'),
        )
        .unwrap();
    assert_eq!(repeat.outcome, "duplicate");
}

/// If the new attempt records nothing (a worker from an older release), the first attempt's inputs must not stay behind
/// to describe trials they did not produce: starting the new attempt's trials removes them.
#[test]
fn a_new_attempt_that_records_nothing_does_not_inherit_the_first_attempts_inputs() {
    let (svc, store, engine) = setup();
    let (a, a_secret) = svc.register_remote_worker("a", "{}").unwrap();
    let (b, b_secret) = svc.register_remote_worker("b", "{}").unwrap();
    let job = lease(&svc, &engine, &a.id);
    svc.record_run_inputs_with_secret(
        &job.job_id,
        &a.id,
        Some(&a_secret),
        &sim_submission("lock-lease"),
    )
    .unwrap();
    svc.fail(&job.job_id, &a.id, "the worker was lost").unwrap();
    let again = svc.lease(&b.id).unwrap().expect("leased again");
    // Not yet: until the new attempt's trials arrive the earlier record is what the store holds.
    assert!(store.get_run_inputs(&job.run_id).unwrap().is_some());
    svc.ingest_trials_with_secret(&again.job_id, &b.id, Some(&b_secret), &one_trial())
        .unwrap();
    assert!(!svc.run_inputs_view(&job.run_id).unwrap().recorded);
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

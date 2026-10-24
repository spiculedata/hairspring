//! Confirming a stored failure through the service: the pipeline runs on the engine, every check is stored, and the
//! failure's state is read back from the stored checks alone.
#![cfg(unix)]

mod common;

use common::setup;
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::confirm::{ConfirmRequest, FindingStatus};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::host_identity::HostSources;
use hairspring_hermit::{HermitBackend, HermitSettings};
use hairspring_service::{Service, ServiceError, StartRun};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const FAKE_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\nshift\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";
const FAILING_GUEST: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-svcconf-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn engine(host: &Path, guest: &str) -> Engine {
    let bin = scratch("bin").join("fake-hermit");
    std::fs::write(&bin, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), guest.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 1,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin,
        summary_dir: None,
        ..HermitSettings::default()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(
        HermitBackend::new(settings).with_host_sources(HostSources::rooted(host)),
    ))
}

/// A service with one stored, failing run, and the engine that made it.
fn failed_run() -> (Service, Engine, String, String) {
    let (svc, _, _) = setup();
    let host = scratch("host");
    put(&host, "etc/ld.so.cache", "cache-one");
    let engine = engine(&host, FAILING_GUEST);
    let detail = svc
        .run_sync(
            &engine,
            &StartRun {
                target: "svc".into(),
                seed: Some(1),
                budget: None,
            },
        )
        .unwrap();
    let failure = detail
        .failures
        .first()
        .expect("a stored failure")
        .id
        .clone();
    (svc, engine, detail.run.id, failure)
}

fn confirm(svc: &Service, engine: &Engine, failure: &str) -> hairspring_service::FailureChecks {
    svc.confirm_failure(engine, failure, &ConfirmRequest::default(), &mut |_| true)
        .unwrap()
}

#[test]
fn confirming_a_stored_failure_stores_every_check_and_reads_the_state_back() {
    let (svc, engine, _, failure) = failed_run();
    let before = svc.failure_checks(&failure).unwrap().unwrap();
    assert_eq!(before.finding, "candidate");
    assert!(before.checks.is_empty());

    let after = confirm(&svc, &engine, &failure);
    assert_eq!(after.finding, "confirmed");
    let kinds: Vec<_> = after.checks.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["same_seed_rerun", "bundle_replay"]);
    assert_eq!(after.checks[0].counts.passed, 3);
    assert!(after.checks[0].guest.command.contains("--sched-seed=1"));
    assert_eq!(
        after.checks[0].confirmation_id,
        after.checks[1].confirmation_id
    );
    assert!(after.checks[0].runner_trusted);

    let read = svc.failure_checks(&failure).unwrap().unwrap();
    assert_eq!(read.finding, "confirmed");
    assert_eq!(read.checks.len(), 2);
}

#[test]
fn a_later_confirmation_adds_to_the_earlier_one_and_the_latest_check_of_each_kind_counts() {
    let (svc, engine, _, failure) = failed_run();
    confirm(&svc, &engine, &failure);
    let req = ConfirmRequest {
        strict_verify: true,
        ..ConfirmRequest::default()
    };
    let second = svc
        .confirm_failure(&engine, &failure, &req, &mut |_| true)
        .unwrap();
    assert_eq!(second.checks.len(), 5, "two, then three more");
    assert!(second.checks[0].created_at <= second.checks[2].created_at);
    // No private summary directory in this rig, so the strict verify is inconclusive and the finding stays confirmed.
    assert_eq!(second.finding, "confirmed");
}

#[test]
fn the_failure_summary_in_a_run_carries_the_finding() {
    let (svc, engine, run, failure) = failed_run();
    let summary = |svc: &Service| {
        let detail = svc.get_run(&run).unwrap().unwrap();
        detail.failures[0]
            .finding
            .clone()
            .expect("a finding summary")
    };
    let before = summary(&svc);
    assert_eq!((before.status.as_str(), before.checks), ("candidate", 0));
    confirm(&svc, &engine, &failure);
    let after = summary(&svc);
    assert_eq!((after.status.as_str(), after.checks), ("confirmed", 2));
}

#[test]
fn an_unknown_or_malformed_failure_id_is_refused_without_running_anything() {
    let (svc, engine, _, _) = failed_run();
    let req = ConfirmRequest::default();
    let err = svc
        .confirm_failure(&engine, "fail_0123456789abcdef", &req, &mut |_| true)
        .unwrap_err();
    assert!(matches!(err, ServiceError::NotFound(_)), "{err:?}");
    for bad in ["", "nope", "fail_zz", "fail_0123456789abcdef0", "../x"] {
        let err = svc
            .confirm_failure(&engine, bad, &req, &mut |_| true)
            .unwrap_err();
        assert!(
            matches!(err, ServiceError::InvalidRequest(_)),
            "{bad}: {err:?}"
        );
    }
    assert!(svc
        .failure_checks("fail_0123456789abcdef")
        .unwrap()
        .is_none());
}

#[test]
fn a_request_over_the_limits_is_refused_and_nothing_is_stored() {
    let (svc, engine, _, failure) = failed_run();
    let req = ConfirmRequest {
        reruns: 11,
        ..ConfirmRequest::default()
    };
    let err = svc
        .confirm_failure(&engine, &failure, &req, &mut |_| true)
        .unwrap_err();
    assert!(matches!(err, ServiceError::Engine(_)), "{err:?}");
    assert!(svc
        .failure_checks(&failure)
        .unwrap()
        .unwrap()
        .checks
        .is_empty());
}

#[test]
fn a_cancelled_confirmation_is_stored_with_what_did_not_run_marked_skipped() {
    let (svc, engine, _, failure) = failed_run();
    let out = svc
        .confirm_failure(&engine, &failure, &ConfirmRequest::default(), &mut |_| {
            false
        })
        .unwrap();
    let replay = out
        .checks
        .iter()
        .find(|c| c.kind == "bundle_replay")
        .unwrap();
    assert_eq!(replay.status, "skipped");
    assert_eq!(out.finding, "candidate");
}

#[test]
fn a_bundle_can_be_confirmed_without_a_stored_failure() {
    let (svc, engine, _, failure) = failed_run();
    let bundle = svc.get_failure(&failure).unwrap().unwrap().bundle;
    let checks = svc
        .confirm_bundle(&engine, &bundle, &ConfirmRequest::default(), &mut |_| true)
        .unwrap();
    assert_eq!(checks.len(), 2);
    assert_eq!(
        hairspring_engine::confirm::finding_status(&checks),
        FindingStatus::Confirmed
    );
    assert!(svc
        .failure_checks(&failure)
        .unwrap()
        .unwrap()
        .checks
        .is_empty());
}

#[test]
fn a_hostile_bundle_is_refused_by_size_and_shape() {
    let (svc, engine, _, _) = failed_run();
    let req = ConfirmRequest::default();
    let big = "x".repeat(hairspring_service::MAX_BUNDLE_BYTES + 1);
    for bad in [big.as_str(), "not a bundle", ""] {
        let err = svc
            .confirm_bundle(&engine, bad, &req, &mut |_| true)
            .unwrap_err();
        assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err:?}");
    }
}

#[test]
fn a_changed_loader_is_stored_as_inconclusive_and_the_finding_is_not_confirmed() {
    let (svc, _, _, failure) = failed_run();
    // A second engine on a host whose loader cache differs from the one the bundle was recorded on.
    let host = scratch("host-two");
    put(&host, "etc/ld.so.cache", "cache-two");
    let other = engine(&host, FAILING_GUEST);
    let out = confirm(&svc, &other, &failure);
    assert_eq!(out.finding, "candidate");
    for check in &out.checks {
        assert_eq!(check.status, "inconclusive", "{check:?}");
        assert!(check.guest.detail["inconclusive"].contains("environment differs"));
    }
    assert!(!out.environment_pinned);
}

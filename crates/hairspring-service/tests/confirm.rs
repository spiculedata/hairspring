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
    engine_with_bin(host, guest, bin)
}

fn engine_with_bin(host: &Path, guest: &str, bin: PathBuf) -> Engine {
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
            pass_env: false,
            backend: None,
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
    let fixed = TargetSpec {
        name: "fixed".into(),
        kind: match &spec.kind {
            TargetKind::Process {
                env,
                hermit_args,
                scheduler,
                ..
            } => TargetKind::Process {
                command: vec!["/bin/sh".into(), "-c".into(), "echo fixed".into()],
                env: env.clone(),
                cwd: None,
                hermit_args: hermit_args.clone(),
                trial_timeout_secs: None,
                scheduler: scheduler.clone(),
                hang_is_violation: false,
                pass_env: false,
                backend: None,
                parallelism: 1,
            },
            _ => unreachable!("the target above is a process target"),
        },
        ..spec.clone()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec), ("fixed".to_string(), fixed)]),
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

#[test]
fn a_stored_failure_of_a_process_target_may_be_confirmed_here() {
    let (svc, engine, _, failure) = failed_run();

    svc.check_confirmable(&engine, &failure)
        .expect("a process target on this engine");
}

#[test]
fn a_failure_whose_target_this_engine_does_not_have_is_refused_in_fixed_words() {
    // Arrange: the failure was stored by a run of target `svc`; the engine now has no such target.
    let (svc, _, _, failure) = failed_run();
    let other = Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::new(),
        },
    );

    // Act
    let err = svc.check_confirmable(&other, &failure).unwrap_err();

    // Assert
    let ServiceError::InvalidRequest(text) = err else {
        panic!("expected a refusal, got {err}");
    };
    assert_eq!(
        text,
        "this server has no configuration for the failure's target, so it cannot confirm it"
    );
    assert!(!text.contains("svc"), "the target name is not repeated");
}

#[test]
fn a_failure_of_a_target_that_is_not_a_process_target_is_refused_in_fixed_words() {
    // Arrange: the same target name, now configured as a simulation.
    let (svc, _, _, failure) = failed_run();
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let mut config = Config::from_registry(&registry);
    let template = config
        .targets
        .values()
        .next()
        .expect("a demo target")
        .clone();
    config.targets.insert(
        "svc".to_string(),
        TargetSpec {
            name: "svc".into(),
            ..template
        },
    );
    let sim = Engine::new(registry, config);

    // Act
    let err = svc.check_confirmable(&sim, &failure).unwrap_err();

    // Assert
    let ServiceError::InvalidRequest(text) = err else {
        panic!("expected a refusal, got {err}");
    };
    assert_eq!(
        text,
        "only a failure of a process target can be confirmed, and this one is not"
    );
}

#[test]
fn a_malformed_or_unknown_failure_id_is_not_confirmable() {
    let (svc, engine, _, _) = failed_run();

    assert!(matches!(
        svc.check_confirmable(&engine, "nope"),
        Err(ServiceError::InvalidRequest(_))
    ));
    assert!(matches!(
        svc.check_confirmable(&engine, "fail_0000000000000000"),
        Err(ServiceError::NotFound(_))
    ));
}

#[test]
fn a_missing_hermit_gives_an_answer_and_does_not_hang() {
    // Arrange: the engine's Hermit is a path that does not exist.
    let (svc, _, _, failure) = failed_run();
    let host = scratch("host-missing");
    put(&host, "etc/ld.so.cache", "cache-one");
    let missing = engine_with_bin(&host, FAILING_GUEST, PathBuf::from("/nonexistent/hermit"));

    // Act
    let started = std::time::Instant::now();
    let result = svc.confirm_failure(&missing, &failure, &ConfirmRequest::default(), &mut |_| {
        true
    });

    // Assert: it answers at once, as `hairspring confirm` does: every check is stored as inconclusive, saying that
    // Hermit could not be started, and the failure does not move off candidate.
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    let checks = result.expect("the checks are stored, not an error");
    assert_eq!(checks.finding, "candidate");
    assert!(!checks.checks.is_empty());
    for check in &checks.checks {
        assert_eq!(check.status, "inconclusive", "{check:?}");
    }
    assert!(checks.checks[0].guest.detail["inconclusive"].contains("Hermit could not be started"));
}

#[test]
fn a_later_confirmation_whose_every_check_is_inconclusive_moves_a_confirmed_failure_back_to_candidate(
) {
    // This pins what the ladder does today, so that changing it is a decision and not an accident: the finding is read
    // from the newest confirmation that finished, and a confirmation whose checks are all inconclusive (here, Hermit
    // cannot be started) finished, so it replaces an earlier decisive one. The earlier checks stay stored and listed.
    let (svc, engine, _, failure) = failed_run();
    assert_eq!(confirm(&svc, &engine, &failure).finding, "confirmed");
    let host = scratch("host-no-hermit");
    put(&host, "etc/ld.so.cache", "cache-one");
    let no_hermit = engine_with_bin(&host, FAILING_GUEST, PathBuf::from("/nonexistent/hermit"));

    let after = svc
        .confirm_failure(
            &no_hermit,
            &failure,
            &ConfirmRequest::default(),
            &mut |_| true,
        )
        .unwrap();

    assert_eq!(after.finding, "candidate");
    assert!(
        after.checks.iter().any(|c| c.status == "pass"),
        "the earlier checks are still stored"
    );
    assert!(after.checks.iter().any(|c| c.status == "inconclusive"));
    // Every check of the newest confirmation is inconclusive: that is what replaced the earlier, decisive one.
    let newest = after.checks.last().expect("checks").confirmation_id.clone();
    let of_newest: Vec<_> = after
        .checks
        .iter()
        .filter(|c| c.confirmation_id == newest)
        .collect();
    assert!(!of_newest.is_empty());
    assert!(
        of_newest.iter().all(|c| c.status == "inconclusive"),
        "{of_newest:?}"
    );
    assert_eq!(
        svc.failure_checks(&failure).unwrap().unwrap().finding,
        "candidate"
    );
}

/// A Hermit that runs the guest directly and counts its starts in a file named `count`. Once a file named `cut_at`
/// holds a number, the start with that number, and every start after it, leaves a file named `started` and kills itself
/// with SIGTERM: what a service manager's stop does to the run in progress.
const CUTTABLE_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\nd=$(dirname \"$0\")\nn=$(cat \"$d/count\" 2>/dev/null || echo 0)\nn=$((n+1))\necho $n > \"$d/count\"\nif [ -f \"$d/cut_at\" ] && [ \"$n\" -ge \"$(cat \"$d/cut_at\")\" ]; then touch \"$d/started\"; kill -TERM $$; fi\nshift\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";

/// A stored failure on an engine whose Hermit can be cut, and its directory.
struct CutRig {
    svc: Service,
    engine: Engine,
    dir: PathBuf,
    failure: String,
}

impl CutRig {
    fn new(name: &str) -> CutRig {
        let (svc, _, _) = setup();
        let host = scratch(&format!("host-{name}"));
        put(&host, "etc/ld.so.cache", "cache-one");
        let dir = scratch(&format!("bin-{name}"));
        let bin = dir.join("fake-hermit");
        std::fs::write(&bin, CUTTABLE_HERMIT).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let engine = engine_with_bin(&host, FAILING_GUEST, bin);
        let run = svc
            .run_sync(
                &engine,
                &StartRun {
                    target: "svc".into(),
                    seed: Some(1),
                    budget: None,
                },
            )
            .unwrap();
        let failure = run.failures[0].id.clone();
        CutRig {
            svc,
            engine,
            dir,
            failure,
        }
    }

    /// The failure confirmed once, with nothing cut, so that its finding is "confirmed".
    fn confirmed(name: &str) -> CutRig {
        let rig = CutRig::new(name);
        assert_eq!(
            confirm(&rig.svc, &rig.engine, &rig.failure).finding,
            "confirmed"
        );
        rig
    }

    /// From the `n`th Hermit start from now on, every start is cut by SIGTERM.
    fn cut_from(&self, n: u32) {
        std::fs::write(self.dir.join("count"), "0").unwrap();
        std::fs::write(self.dir.join("cut_at"), n.to_string()).unwrap();
    }

    fn started(&self) -> PathBuf {
        self.dir.join("started")
    }

    fn confirm_with(
        &self,
        req: &ConfirmRequest,
        stop: &mut dyn FnMut() -> bool,
    ) -> hairspring_service::FailureChecks {
        self.svc
            .confirm_failure(&self.engine, &self.failure, req, &mut |_| !stop())
            .unwrap()
    }
}

/// The checks of the newest confirmation, as (kind, status, why).
fn newest(after: &hairspring_service::FailureChecks) -> Vec<(String, String, String)> {
    let newest = after.checks.last().expect("checks").confirmation_id.clone();
    after
        .checks
        .iter()
        .filter(|c| c.confirmation_id == newest)
        .map(|c| {
            let why = c
                .guest
                .detail
                .get("skipped")
                .or_else(|| c.guest.detail.get("inconclusive"))
                .cloned()
                .unwrap_or_default();
            (c.kind.clone(), c.status.clone(), why)
        })
        .collect()
}

#[test]
fn a_rerun_cut_short_by_the_stop_is_stored_skipped_and_an_earlier_confirmed_finding_stays() {
    // Arrange: confirmed once. The stop arrives while the first rerun runs, and the Hermit process is killed with it.
    let rig = CutRig::confirmed("cut-rerun");
    rig.cut_from(1);
    let started = rig.started();

    // Act
    let after = rig.confirm_with(&ConfirmRequest::default(), &mut || started.exists());

    // Assert: stored as the CLI's Ctrl-C stores it, the cut check and the rest skipped, so the finding does not move.
    assert_eq!(after.finding, "confirmed", "{:?}", after.checks);
    let mine = newest(&after);
    assert_eq!(mine[0].0, "same_seed_rerun");
    assert!(
        mine.iter()
            .all(|(_, status, why)| status == "skipped" && why == "cancelled"),
        "{mine:?}"
    );
}

#[test]
fn a_replay_cut_short_by_the_stop_is_stored_skipped_and_an_earlier_confirmed_finding_stays() {
    // The replay is the last check, so nothing after it is skipped for it: the cut replay itself must say
    // "unfinished", or the cut confirmation counts as the newest finished one.
    let rig = CutRig::confirmed("cut-replay");
    rig.cut_from(4); // three reruns, then the replay
    let started = rig.started();

    let after = rig.confirm_with(&ConfirmRequest::default(), &mut || started.exists());

    assert_eq!(after.finding, "confirmed", "{:?}", after.checks);
    let mine = newest(&after);
    assert_eq!(mine.len(), 2, "{mine:?}");
    assert_eq!(
        mine[0].1, "pass",
        "the reruns before the stop count: {mine:?}"
    );
    assert_eq!(
        (mine[1].0.as_str(), mine[1].1.as_str(), mine[1].2.as_str()),
        ("bundle_replay", "skipped", "cancelled")
    );
}

#[test]
fn a_control_cut_short_by_the_stop_is_stored_skipped_and_does_not_make_the_control_fail() {
    let rig = CutRig::confirmed("cut-control");
    rig.cut_from(5); // three reruns, the replay, then the control
    let started = rig.started();
    let req = ConfirmRequest {
        control: Some("fixed".into()),
        ..ConfirmRequest::default()
    };

    let after = rig.confirm_with(&req, &mut || started.exists());

    assert_eq!(after.finding, "confirmed", "{:?}", after.checks);
    let mine = newest(&after);
    assert_eq!(mine.len(), 3, "{mine:?}");
    assert_eq!(
        (mine[2].0.as_str(), mine[2].1.as_str(), mine[2].2.as_str()),
        ("fixed_control", "skipped", "cancelled")
    );
}

#[test]
fn a_stop_that_reaches_the_server_a_moment_after_the_run_died_is_still_heard() {
    // The Hermit process dies first and the stop is noticed 150 ms later, as when a service manager's signals arrive
    // a little apart. Without the short wait the dead run is judged a miss before the stop is known.
    let rig = CutRig::confirmed("cut-late");
    rig.cut_from(1);
    let started = rig.started();
    let heard_late = move || {
        std::fs::metadata(&started)
            .and_then(|m| m.modified())
            .is_ok_and(|at| {
                at.elapsed().unwrap_or_default() >= std::time::Duration::from_millis(150)
            })
    };

    let after = rig.confirm_with(&ConfirmRequest::default(), &mut { heard_late });

    assert_eq!(after.finding, "confirmed", "{:?}", after.checks);
    assert!(newest(&after)
        .iter()
        .all(|(_, status, _)| status == "skipped"));
}

#[test]
fn a_stop_noticed_long_after_the_run_died_is_not_heard_and_the_run_counts_as_a_miss() {
    // The limit, pinned: the wait is half a second. A stop that reaches the server later than that is not noticed,
    // and a run killed by SIGTERM is judged as it always was.
    let rig = CutRig::confirmed("cut-too-late");
    rig.cut_from(1);
    let started = rig.started();
    let heard_far_too_late = move || {
        std::fs::metadata(&started)
            .and_then(|m| m.modified())
            .is_ok_and(|at| {
                at.elapsed().unwrap_or_default() >= std::time::Duration::from_millis(3_000)
            })
    };

    let after = rig.confirm_with(&ConfirmRequest::default(), &mut { heard_far_too_late });

    assert_eq!(after.finding, "not_reproducible", "{:?}", after.checks);
}

#[test]
fn a_death_by_sigterm_with_no_stop_is_still_a_miss() {
    // So that a change to how cut runs are read cannot hide real misses: every run dies of SIGTERM and nobody asked to
    // stop. The reruns and the replay fail, and the finding is withdrawn.
    let rig = CutRig::confirmed("sigterm-no-stop");
    rig.cut_from(1);

    let after = rig.confirm_with(&ConfirmRequest::default(), &mut || false);

    assert_eq!(after.finding, "not_reproducible", "{:?}", after.checks);
    let mine = newest(&after);
    assert_eq!(mine[0].1, "fail", "{mine:?}");
    assert_eq!(mine[1].1, "fail", "{mine:?}");
}

#[test]
fn a_run_cut_short_by_the_stop_does_not_turn_a_never_confirmed_failure_into_not_reproducible() {
    let rig = CutRig::new("cut-first");
    rig.cut_from(1);
    let started = rig.started();

    let after = rig.confirm_with(&ConfirmRequest::default(), &mut || started.exists());

    assert_eq!(after.finding, "candidate", "{:?}", after.checks);
    assert!(after.checks.iter().all(|c| c.status != "fail"));
}

/// A stored failure of target `svc` whose bundle text is `bundle`, on a service whose engine has process target `svc`.
const MISSING_BLOB: &str = "<no blob>";

fn failure_with_bundle(bundle: &str) -> (Service, Engine, String) {
    failure_stored_under("svc", bundle)
}

/// A stored failure of target `target` whose bundle text is `bundle`, planted straight in the store (as old or forged
/// data could be), on a service whose engine has process target `svc`.
fn failure_stored_under(target: &str, bundle: &str) -> (Service, Engine, String) {
    let (svc, store, _) = setup();
    let host = scratch("host-bundle");
    put(&host, "etc/ld.so.cache", "cache-one");
    let engine = engine(&host, FAILING_GUEST);
    let run = store
        .create_run(
            &hairspring_store::NewRun {
                target: target.into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let hash = if bundle == MISSING_BLOB {
        // A hash no blob has: what is left after a blob was lost or the failure was written without one.
        "0".repeat(64)
    } else {
        store.put_blob(bundle.as_bytes()).unwrap()
    };
    let failure = store
        .add_failure(
            &hairspring_store::NewFailure {
                run_id: run.id,
                target: target.into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: hash,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap();
    (svc, engine, failure.id)
}

#[test]
fn a_stored_bundle_that_names_another_target_is_not_confirmable_and_the_target_is_not_repeated() {
    // Arrange: the failure is stored under `svc`, but its bundle belongs to another target (a worker's doing).
    let hostile = "<script>alert(1)</script>";
    for other in ["good", "", hostile] {
        let bundle = hairspring_core::bundle::Bundle::new(1, "t", vec![])
            .with_target(other)
            .encode();
        let (svc, engine, failure) = failure_with_bundle(&bundle);

        // Act
        let err = svc.check_confirmable(&engine, &failure).unwrap_err();

        // Assert
        let ServiceError::InvalidRequest(text) = err else {
            panic!("expected a refusal, got {err}");
        };
        assert_eq!(
            text,
            "the failure's bundle is for a different target, so it cannot be confirmed"
        );
        assert!(svc
            .failure_checks(&failure)
            .unwrap()
            .unwrap()
            .checks
            .is_empty());
    }
}

#[test]
fn confirming_a_stored_failure_refuses_a_bundle_for_another_target_whatever_the_caller() {
    // Arrange: planted data, as an older server could hold. The failure sits under `good`, but its bundle is for the
    // failing target `svc`, which this engine can run. `confirm_failure` is what the CLI and the server both call.
    let hostile = "<script>alert(1)</script>";
    for stored_under in ["good", hostile] {
        let bundle = hairspring_core::bundle::Bundle::new(1, "t", vec![])
            .with_target("svc")
            .with_backend("hermit")
            .with_param("sched_seed", "1")
            .encode();
        let (svc, engine, failure) = failure_stored_under(stored_under, &bundle);

        // Act
        let err = svc
            .confirm_failure(&engine, &failure, &ConfirmRequest::default(), &mut |_| true)
            .unwrap_err();

        // Assert: refused in fixed words before anything runs, and nothing is stored against the failure.
        let ServiceError::InvalidRequest(text) = err else {
            panic!("expected a refusal, got {err}");
        };
        assert_eq!(
            text,
            "the failure's bundle is for a different target, so it cannot be confirmed"
        );
        assert!(!text.contains("script"));
        let stored = svc.failure_checks(&failure).unwrap().unwrap();
        assert!(stored.checks.is_empty());
        assert_eq!(stored.finding, "candidate");
    }
}

#[test]
fn a_failure_whose_bundle_blob_is_missing_is_refused_in_fixed_words_by_both_paths() {
    let (svc, engine, failure) = failure_with_bundle(MISSING_BLOB);
    let words = "the failure's bundle cannot be read, so it cannot be confirmed";

    let guard = svc.check_confirmable(&engine, &failure).unwrap_err();
    let confirm = svc
        .confirm_failure(&engine, &failure, &ConfirmRequest::default(), &mut |_| true)
        .unwrap_err();

    for err in [guard, confirm] {
        assert!(
            matches!(&err, ServiceError::InvalidRequest(t) if t == words),
            "{err}"
        );
    }
}

#[test]
fn a_stored_bundle_that_does_not_decode_is_not_confirmable() {
    let (svc, engine, failure) = failure_with_bundle("not a bundle");

    let err = svc.check_confirmable(&engine, &failure).unwrap_err();

    assert!(
        matches!(&err, ServiceError::InvalidRequest(t)
            if t == "the failure's bundle cannot be read, so it cannot be confirmed"),
        "{err}"
    );
}

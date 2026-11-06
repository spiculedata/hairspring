//! Starting a confirmation from the API, over real HTTP: `POST /api/failures/{id}/confirm`. Off unless the server was
//! started with `max_confirmations`; needs the run role; runs on the server's own engine; one at a time per failure and
//! only as many at once as the server allows, the rest refused and never queued.
#![cfg(unix)]
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::host_identity::HostSources;
use hairspring_hermit::{HermitBackend, HermitSettings};
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::{Service, StartRun};
use hairspring_store::{NewFailure, NewRun, SqliteStore, Store};
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const ADMIN: &str = "admin-token-0123456789abcdef";
const HOSTILE: &str = "<script>alert(1)</script>";
const DISABLED_TEXT: &str = "confirmation from the browser is not enabled on this server";
const ABSENT: &str = "fail_0000000000000000";

/// A Hermit that runs the guest directly. While a file named `sleep` sits beside it, each start leaves a file named
/// `started` and waits a second first.
const FAKE_HERMIT: &str = "#!/bin/sh\n[ \"$1\" = \"--version\" ] && { echo 'fake-hermit 0.0'; exit 0; }\n[ -f \"$(dirname \"$0\")/sleep\" ] && { touch \"$(dirname \"$0\")/started\"; sleep 1; }\nshift\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n";
const FAILING_GUEST: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";

struct Resp {
    status: u16,
    retry_after: Option<String>,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

fn send(
    addr: SocketAddr,
    token: Option<&str>,
    method: &str,
    path: &str,
    origin: Option<&str>,
) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let url = format!("http://{addr}{path}");
    let mut resp = if method == "POST" {
        let mut r = agent.post(url);
        if let Some(t) = token {
            r = r.header("Authorization", format!("Bearer {t}"));
        }
        if let Some(o) = origin {
            r = r.header("Origin", o);
        }
        r.send_empty().expect("transport ok")
    } else {
        let mut r = agent.get(url);
        if let Some(t) = token {
            r = r.header("Authorization", format!("Bearer {t}"));
        }
        r.call().expect("transport ok")
    };
    Resp {
        status: resp.status().as_u16(),
        retry_after: resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: resp.body_mut().read_to_string().unwrap_or_default(),
    }
}

fn post(addr: SocketAddr, token: Option<&str>, id: &str) -> Resp {
    send(
        addr,
        token,
        "POST",
        &format!("/api/failures/{id}/confirm"),
        None,
    )
}

fn get(addr: SocketAddr, token: Option<&str>, path: &str) -> Resp {
    send(addr, token, "GET", path, None)
}

/// A directory of its own for each call, even from tests that run at the same time.
fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hairspring-srvconf-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn process_target() -> TargetSpec {
    TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), FAILING_GUEST.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            pass_env: false,
            backend: None,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: 1,
            wall_secs: 60,
        },
    }
}

/// An engine with one process target `svc`, run by a fake Hermit; the directory holds the fake, so a `sleep` file
/// there makes every Hermit start wait.
fn engine(dir: &Path) -> Engine {
    let host = dir.join("host");
    std::fs::create_dir_all(host.join("etc")).unwrap();
    std::fs::write(host.join("etc/ld.so.cache"), "cache-one").unwrap();
    let bin = dir.join("fake-hermit");
    std::fs::write(&bin, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let settings = HermitSettings {
        bin,
        summary_dir: None,
        ..HermitSettings::default()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), process_target())]),
        },
    )
    .with_backend(Arc::new(
        HermitBackend::new(settings).with_host_sources(HostSources::rooted(&host)),
    ))
}

struct Rig {
    server: Option<RunningServer>,
    svc: Service,
    dir: PathBuf,
    /// Two stored failures of the process target, from two runs.
    failures: [String; 2],
}

impl Rig {
    fn addr(&self) -> SocketAddr {
        self.server.as_ref().expect("running").addr()
    }

    /// From now on every Hermit start waits a second.
    fn make_slow(&self) {
        std::fs::write(self.dir.join("sleep"), "").unwrap();
    }

    fn checks(&self, id: &str) -> Value {
        get(
            self.addr(),
            Some(READ),
            &format!("/api/failures/{id}/checks"),
        )
        .json()
    }

    /// Waits until the server no longer says this failure is being confirmed.
    fn wait_idle(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let status = get(
                self.addr(),
                Some(READ),
                &format!("/api/failures/{id}/confirm"),
            )
            .json();
            if status["state"] != "running" {
                return status;
            }
            assert!(Instant::now() < deadline, "the confirmation did not finish");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn config(max_confirmations: usize, auth: Auth) -> ServerConfig {
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    cfg.max_confirmations = max_confirmations;
    cfg
}

fn boot_with(max_confirmations: usize, auth: Auth) -> Rig {
    let dir = scratch("rig");
    let engine = engine(&dir);
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store);
    let failure = |seed| {
        let detail = svc
            .run_sync(
                &engine,
                &StartRun {
                    target: "svc".into(),
                    seed: Some(seed),
                    budget: None,
                },
            )
            .unwrap();
        detail
            .failures
            .first()
            .expect("a stored failure")
            .id
            .clone()
    };
    let failures = [failure(1), failure(2)];
    let server =
        start(config(max_confirmations, auth), svc.clone(), engine).expect("server starts");
    Rig {
        server: Some(server),
        svc,
        dir,
        failures,
    }
}

fn tokens() -> Auth {
    Auth::from_spec(&format!("{READ}:read,{RUN}:run,{ADMIN}:admin")).unwrap()
}

fn boot(max_confirmations: usize) -> Rig {
    boot_with(max_confirmations, tokens())
}

#[test]
fn it_is_off_by_default_and_says_so_in_fixed_words_without_running_anything() {
    let rig = boot(0);
    let id = &rig.failures[0];

    let r = post(rig.addr(), Some(RUN), id);

    assert_eq!(r.status, 403, "{}", r.body);
    assert_eq!(r.json()["error"], DISABLED_TEXT);
    assert_eq!(r.json()["code"], "confirm_disabled");
    assert!(rig.checks(id)["checks"].as_array().unwrap().is_empty());
    let me = get(rig.addr(), Some(RUN), "/api/me").json();
    assert_eq!(me["can"]["confirm"], false);
    assert!(
        me.get("confirm").is_none(),
        "nothing about limits when it is off"
    );
}

#[test]
fn a_disabled_server_answers_the_same_for_every_id_real_unknown_or_malformed() {
    // The switch is checked before the id, so a disabled server does not tell callers which failures exist.
    let rig = boot(0);
    let real = rig.failures[0].clone();

    for id in [
        real.as_str(),
        ABSENT,
        "nope",
        "fail_xyz",
        "..%2f..",
        "fail_0123456789abcdef0",
    ] {
        let r = post(rig.addr(), Some(RUN), id);

        assert_eq!(r.status, 403, "{id}: {}", r.body);
        assert_eq!(r.json()["error"], DISABLED_TEXT, "{id}");
        assert_eq!(r.json()["code"], "confirm_disabled", "{id}");
    }
}

#[test]
fn it_needs_a_token_and_the_run_role() {
    let rig = boot(1);
    let id = &rig.failures[0];

    assert_eq!(post(rig.addr(), None, id).status, 401);
    assert_eq!(
        post(rig.addr(), Some("wrong-token-0123456789abcdef"), id).status,
        401
    );
    let read = post(rig.addr(), Some(READ), id);
    assert_eq!(read.status, 403);
    assert_eq!(read.json()["error"], "this endpoint requires the run role");
    assert!(rig.checks(id)["checks"].as_array().unwrap().is_empty());
    assert_eq!(
        rig.wait_idle(id)["state"],
        "idle",
        "a refused request starts nothing"
    );
}

#[test]
fn me_says_can_confirm_only_for_a_role_that_may_when_it_is_on() {
    let rig = boot(2);

    let read = get(rig.addr(), Some(READ), "/api/me").json();
    let run = get(rig.addr(), Some(RUN), "/api/me").json();
    let admin = get(rig.addr(), Some(ADMIN), "/api/me").json();

    assert_eq!(read["can"]["confirm"], false);
    assert!(read.get("confirm").is_none());
    for me in [&run, &admin] {
        assert_eq!(me["can"]["confirm"], true);
        assert_eq!(me["confirm"]["wall_secs"], 120);
        assert_eq!(me["confirm"]["max_running"], 2);
    }
}

#[test]
fn a_malformed_or_unknown_failure_id_is_a_404_and_nothing_runs() {
    let rig = boot(1);

    for bad in [
        "nope",
        "fail_xyz",
        "fail_0123456789abcdeg",
        "fail_0123456789abcdef0",
        "..%2f..",
        ABSENT,
    ] {
        let r = post(rig.addr(), Some(RUN), bad);
        assert_eq!(r.status, 404, "{bad}: {}", r.body);
    }
    assert_eq!(rig.wait_idle(&rig.failures[0])["state"], "idle");
}

/// A server whose store holds one failure of `target`, a name its engine does not have.
fn failure_of_target(target: &str) -> (Rig, String) {
    failure_with_bundle(target, b"bundle")
}

/// A server whose store holds one failure of `target` with the given bundle text.
fn failure_with_bundle(target: &str, bundle: &[u8]) -> (Rig, String) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let run = store
        .create_run(
            &NewRun {
                target: target.into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = store.put_blob(bundle).unwrap();
    let failure = store
        .add_failure(
            &NewFailure {
                run_id: run.id.clone(),
                target: target.into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap();
    let dir = scratch("stale");
    let engine = engine(&dir);
    let svc = Service::new(store);
    let server = start(config(1, tokens()), svc.clone(), engine).expect("server starts");
    let rig = Rig {
        server: Some(server),
        svc,
        dir,
        failures: [failure.id.clone(), failure.id.clone()],
    };
    (rig, failure.id)
}

#[test]
fn a_failure_of_a_target_this_server_does_not_have_is_refused_without_echoing_the_target() {
    let (rig, id) = failure_of_target(HOSTILE);

    let r = post(rig.addr(), Some(RUN), &id);

    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(
        r.json()["error"],
        "this server has no configuration for the failure's target, so it cannot confirm it"
    );
    assert!(!r.body.contains("script"), "{}", r.body);
    assert_eq!(
        rig.wait_idle(&id)["state"],
        "idle",
        "no slot is held for a refusal"
    );
}

#[test]
fn a_failure_whose_bundle_is_for_another_target_is_refused_and_nothing_is_stored_against_it() {
    // A worker can report a failure under a passing target with the bundle of a failing one. Confirming it would
    // rerun the bundle's target and record trusted checks against the other.
    let hostile = "<script>alert(1)</script>";
    for (stored_under, bundle_target) in [("good", "svc"), ("svc", hostile), (hostile, "svc")] {
        let bundle = hairspring_core::bundle::Bundle::new(1, "t", vec![])
            .with_target(bundle_target)
            .encode();
        let (rig, id) = failure_with_bundle(stored_under, bundle.as_bytes());
        let r = post(rig.addr(), Some(RUN), &id);
        assert_eq!(r.status, 400, "{}", r.body);
        assert!(!r.body.contains("script"), "{}", r.body);
        assert!(rig.checks(&id)["checks"].as_array().unwrap().is_empty());
        assert_eq!(rig.wait_idle(&id)["state"], "idle");
    }
}

#[test]
fn a_failure_whose_bundle_is_for_another_target_says_so_in_fixed_words() {
    let bundle = hairspring_core::bundle::Bundle::new(1, "t", vec![])
        .with_target("good")
        .encode();
    let (rig, id) = failure_with_bundle("svc", bundle.as_bytes());

    let r = post(rig.addr(), Some(RUN), &id);

    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(
        r.json()["error"],
        "the failure's bundle is for a different target, so it cannot be confirmed"
    );
    assert!(rig.checks(&id)["checks"].as_array().unwrap().is_empty());
}

#[test]
fn a_confirmation_runs_on_the_server_is_recorded_as_trusted_and_the_finding_moves() {
    let rig = boot(1);
    let id = &rig.failures[0];

    let r = post(rig.addr(), Some(RUN), id);

    assert_eq!(r.status, 202, "{}", r.body);
    assert_eq!(r.json(), serde_json::json!({ "accepted": true }));
    let status = rig.wait_idle(id);
    assert_eq!(status["state"], "idle", "{status}");
    let checks = rig.checks(id);
    assert_eq!(checks["finding"], "confirmed");
    let kinds: Vec<&str> = checks["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["same_seed_rerun", "bundle_replay"]);
    assert!(checks["checks"][0]["runner_trusted"].as_bool().unwrap());
    assert_eq!(
        checks["checks"][0]["counts"]["passed"], 3,
        "the CLI default of three reruns"
    );
}

#[test]
fn a_second_confirmation_of_the_same_failure_is_a_409_while_the_first_runs() {
    let rig = boot(2);
    let id = &rig.failures[0];
    rig.make_slow();
    assert_eq!(post(rig.addr(), Some(RUN), id).status, 202);

    let again = post(rig.addr(), Some(RUN), id);

    assert_eq!(again.status, 409, "{}", again.body);
    assert_eq!(
        again.json()["error"],
        "a confirmation of this failure is already running"
    );
    assert_eq!(again.json()["code"], "confirm_running");
    assert_eq!(rig.wait_idle(id)["state"], "idle");
}

#[test]
fn a_confirmation_over_the_limit_is_a_429_with_retry_after_and_is_not_queued() {
    let rig = boot(1);
    let (first, second) = (&rig.failures[0], &rig.failures[1]);
    rig.make_slow();
    assert_eq!(post(rig.addr(), Some(RUN), first).status, 202);

    let busy = post(rig.addr(), Some(RUN), second);

    assert_eq!(busy.status, 429, "{}", busy.body);
    assert!(busy.retry_after.is_some(), "Retry-After is set");
    assert_eq!(rig.wait_idle(first)["state"], "idle");
    // Nothing was queued: the refused failure still has no checks after the first one is done.
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.checks(second)["checks"].as_array().unwrap().is_empty());
    assert_eq!(rig.wait_idle(second)["state"], "idle");
    // With the slot free the same request is accepted.
    assert_eq!(post(rig.addr(), Some(RUN), second).status, 202);
    rig.wait_idle(second);
}

#[test]
fn the_status_needs_the_read_role_and_a_valid_id() {
    let rig = boot(1);
    let id = &rig.failures[0];
    let path = format!("/api/failures/{id}/confirm");

    assert_eq!(get(rig.addr(), None, &path).status, 401);
    assert_eq!(get(rig.addr(), Some(READ), &path).json()["state"], "idle");
    assert_eq!(
        get(rig.addr(), Some(READ), "/api/failures/nope/confirm").status,
        404
    );
    assert_eq!(
        get(
            rig.addr(),
            Some(READ),
            &format!("/api/failures/{ABSENT}/confirm")
        )
        .status,
        404
    );
}

#[test]
fn a_cross_site_request_to_an_open_server_does_not_start_a_confirmation() {
    // Open mode has no tokens, so it checks the Origin of a state-changing request.
    let rig = boot_with(1, Auth::open());
    let id = &rig.failures[0];
    let path = format!("/api/failures/{id}/confirm");

    let hostile = send(rig.addr(), None, "POST", &path, Some("http://evil.example"));

    assert_eq!(hostile.status, 403, "{}", hostile.body);
    assert!(rig.checks(id)["checks"].as_array().unwrap().is_empty());
    assert_eq!(rig.wait_idle(id)["state"], "idle");
}

#[test]
fn stopping_the_server_mid_confirmation_stores_what_ran_and_marks_the_rest_skipped() {
    let rig = boot(1);
    let id = rig.failures[0].clone();
    rig.make_slow();
    assert_eq!(post(rig.addr(), Some(RUN), &id).status, 202);
    // Stop it only once a check has really started, so there is something for the stop to cut short.
    let started = rig.dir.join("started");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the confirmation never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut rig = rig;
    let svc = rig.svc.clone();

    rig.server.take().expect("running").shutdown();

    let stored = svc.failure_checks(&id).unwrap().unwrap();
    assert!(
        !stored.checks.is_empty(),
        "a partial confirmation is stored"
    );
    assert!(
        stored.checks.iter().any(|c| c.status == "skipped"),
        "what did not run is skipped: {:?}",
        stored.checks.iter().map(|c| &c.status).collect::<Vec<_>>()
    );
}

//! Per-trial records over HTTP: the ingest endpoint, worker trust, and a real remote worker driving a fake Hermit.
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_hermit::{HermitBackend, HermitSettings};
use hairspring_props::{RunCatalog, TrialEvents};
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const ADMIN: &str = "admin-token-0123456789abcdef";

fn tokens() -> Auth {
    Auth::from_spec(&format!("{READ}:read,{RUN}:run,{ADMIN}:admin")).unwrap()
}

fn boot(workers: usize) -> (RunningServer, Arc<dyn Store>) {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), tokens());
    cfg.embedded_workers = workers;
    cfg.rate_limit_per_min = 0;
    (
        start(cfg, Service::new(store.clone()), engine).expect("server starts"),
        store,
    )
}

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
}

fn client(addr: SocketAddr, token: &str) -> Client {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    Client {
        agent,
        base: format!("http://{addr}"),
        token: token.to_string(),
    }
}

impl Client {
    fn finish(&self, r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Resp {
        let mut resp = r.expect("transport ok");
        Resp {
            status: resp.status().as_u16(),
            body: resp.body_mut().read_to_string().unwrap_or_default(),
        }
    }
    fn get(&self, path: &str) -> Resp {
        let r = self
            .agent
            .get(format!("{}{}", self.base, path))
            .header("Authorization", format!("Bearer {}", self.token));
        self.finish(r.call())
    }
    fn post_raw(&self, path: &str, body: &str) -> Resp {
        let r = self
            .agent
            .post(format!("{}{}", self.base, path))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.token));
        self.finish(r.send(body))
    }
    fn post(&self, path: &str, v: &Value) -> Resp {
        self.post_raw(path, &v.to_string())
    }
}

fn props_for(id: &str, trues: u64, falses: u64) -> hairspring_props::TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"always","must_hit":true}}"#)),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":null,"first_true":null}}"#, trues + falses)),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

fn reports(n: u64) -> Vec<TrialReport> {
    let mut catalog = RunCatalog::new();
    (0..n)
        .map(|i| {
            let rec = std::mem::take(&mut catalog).record(&props_for("a.id", 3, 0), false, 16384);
            catalog = rec.catalog;
            TrialReport {
                seq: i,
                seed: i + 1,
                verdict: ReportedVerdict::Pass,
                reason: String::new(),
                exit_code: Some(0),
                signal: None,
                behaviour_hash: Some("00aa".into()),
                schedule_fp: None,
                syscalls: 1,
                work_ops: 1,
                elapsed_ms: 1,
                catalog: rec.updates,
                digest: rec.digest.encode(),
                detail: None,
                output: None,
            }
        })
        .collect()
}

struct Leased {
    run_id: String,
    job_id: String,
    worker: String,
    secret: String,
}

/// Registers a worker over HTTP and returns its id and secret.
fn register(c: &Client, name: &str) -> (String, String) {
    let r = c
        .post("/api/worker/register", &json!({"name": name}))
        .json();
    (
        r["worker_id"].as_str().unwrap().to_string(),
        r["worker_secret"].as_str().expect("a secret").to_string(),
    )
}

/// A run whose budget is `trials` trials, leased by a freshly registered remote worker.
fn lease_as_remote(srv: &RunningServer, trials: u64) -> Leased {
    let c = client(srv.addr(), RUN);
    let run_id = c
        .post(
            "/api/runs",
            &json!({"target": "pubsub-atomic", "seed": 1,
                "budget": {"max_steps": 1000, "max_trials": trials, "wall_secs": 30}}),
        )
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (worker, secret) = register(&c, "remote");
    let job_id = c
        .post(
            "/api/worker/lease",
            &json!({"worker_id": worker, "worker_secret": secret}),
        )
        .json()["job_id"]
        .as_str()
        .expect("leased")
        .to_string();
    Leased {
        run_id,
        job_id,
        worker,
        secret,
    }
}

fn post_trials(c: &Client, l: &Leased, trials: &[TrialReport]) -> Resp {
    c.post(
        &format!("/api/worker/jobs/{}/trials", l.job_id),
        &json!({"worker_id": l.worker, "worker_secret": l.secret, "trials": trials}),
    )
}

#[test]
fn a_second_run_token_naming_a_trusted_workers_id_cannot_send_trials_or_clear_the_unverified_marker(
) {
    let (srv, store) = boot(0);
    let owner_client = client(srv.addr(), RUN);
    let l = lease_as_remote(&srv, 2);
    assert_eq!(
        client(srv.addr(), ADMIN)
            .post(
                &format!("/api/workers/{}/trust", l.worker),
                &json!({"trusted": true})
            )
            .status,
        200
    );
    // A different caller with its own run-role token sees the id in the worker list and tries to use it.
    let other = client(srv.addr(), RUN);
    let path = format!("/api/worker/jobs/{}/trials", l.job_id);
    let seen = other.get("/api/workers").body;
    assert!(
        seen.contains(&l.worker),
        "run-role callers can read worker ids"
    );
    for body in [
        json!({"worker_id": l.worker, "trials": reports(1)}),
        json!({"worker_id": l.worker, "worker_secret": "guess", "trials": reports(1)}),
        json!({"worker_id": l.worker, "worker_secret": "", "trials": reports(1)}),
    ] {
        let r = other.post(&path, &body);
        assert_eq!(r.status, 400, "{}", r.body);
        assert!(!r.body.contains(&l.secret));
    }
    // Replaying the owner's secret under another registered worker's id fails too.
    let (attacker, _) = register(&other, "attacker");
    let replay = other.post(
        &path,
        &json!({"worker_id": attacker, "worker_secret": l.secret, "trials": reports(1)}),
    );
    assert_eq!(replay.status, 400, "{}", replay.body);
    assert_eq!(
        store.count_trials(&l.run_id).unwrap(),
        0,
        "nothing was stored"
    );
    // The owner, with its secret, is trusted and its run is no longer marked unverified.
    assert_eq!(post_trials(&owner_client, &l, &reports(2)).status, 200);
    let done = owner_client.post(
        &format!("/api/worker/jobs/{}/complete", l.job_id),
        &json!({"worker_id": l.worker, "worker_secret": l.secret, "outcome": {"status": "passed", "steps_executed": 2, "trials": 2, "elapsed_ms": 1, "failure": null, "error": null, "trials_reported": 2}}),
    );
    assert_eq!(done.status, 200, "{}", done.body);
    let run = other.get(&format!("/api/runs/{}", l.run_id)).json();
    assert_eq!(run["properties"]["unverified_worker_data"], false, "{run}");
}

#[test]
fn text_from_the_program_never_appears_outside_the_guest_object_on_any_surface() {
    use hairspring_props::AssertKind;
    let (srv, _) = boot(0);
    let c = client(srv.addr(), RUN);
    let l = lease_as_remote(&srv, 2);
    // Two trials that disagree about an id's kind; the second carries marker text in its message and location.
    let mut catalog = RunCatalog::new();
    let first = props_for("guest.idmark", 3, 0);
    let mut second = props_for("guest.idmark", 3, 0);
    second.properties[0].kind = AssertKind::Sometimes;
    second.properties[0].msg = "GUESTMSGMARK".to_string();
    second.properties[0].loc = Some(hairspring_props::Loc {
        file: "GUESTFILEMARK.c".to_string(),
        line: 1,
        func: Some("GUESTFUNCMARK".to_string()),
    });
    let trials: Vec<TrialReport> = [first, second]
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let rec = std::mem::take(&mut catalog).record(p, false, 16384);
            catalog = rec.catalog;
            TrialReport {
                seq: i as u64,
                seed: i as u64 + 1,
                verdict: ReportedVerdict::Pass,
                reason: String::new(),
                exit_code: Some(0),
                signal: None,
                behaviour_hash: None,
                schedule_fp: None,
                syscalls: 1,
                work_ops: 1,
                elapsed_ms: 1,
                catalog: rec.updates,
                digest: rec.digest.encode(),
                detail: None,
                output: None,
            }
        })
        .collect();
    assert_eq!(post_trials(&c, &l, &trials).status, 200);
    let done = c.post(
        &format!("/api/worker/jobs/{}/complete", l.job_id),
        &json!({"worker_id": l.worker, "worker_secret": l.secret, "outcome": {"status": "passed", "steps_executed": 2, "trials": 2, "elapsed_ms": 1, "failure": null, "error": null, "trials_reported": 2}}),
    );
    assert_eq!(done.status, 200, "{}", done.body);

    let mut run = c.get(&format!("/api/runs/{}", l.run_id)).json();
    assert!(run["properties"]["guest"].is_object(), "{run}");
    run["properties"]["guest"] = Value::Null;
    let surfaces = [
        ("GET /api/runs/{id} outside guest", run.to_string()),
        ("GET /api/runs", c.get("/api/runs").body),
        (
            "SSE events",
            c.get(&format!("/api/runs/{}/events", l.run_id)).body,
        ),
        (
            "MCP get_run",
            c.post(
                "/mcp",
                &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_run", "arguments": {"run_id": l.run_id}}}),
            )
            .body,
        ),
        (
            "MCP run resource",
            c.post(
                "/mcp",
                &json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": {"uri": format!("hairspring://runs/{}", l.run_id)}}),
            )
            .body,
        ),
    ];
    for (name, body) in surfaces {
        assert!(!body.is_empty(), "{name}");
        for marker in [
            "GUESTMSGMARK",
            "GUESTFILEMARK",
            "GUESTFUNCMARK",
            "kind_mismatch",
        ] {
            assert!(!body.contains(marker), "{name} leaks {marker}: {body}");
        }
    }
}

fn state_of(c: &Client, run_id: &str) -> String {
    c.get(&format!("/api/runs/{run_id}")).json()["run"]["state"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn every_worker_endpoint_refuses_a_caller_who_only_knows_a_trusted_workers_id() {
    let (srv, store) = boot(0);
    let owner = client(srv.addr(), RUN);
    let l = lease_as_remote(&srv, 4);
    assert_eq!(
        client(srv.addr(), ADMIN)
            .post(
                &format!("/api/workers/{}/trust", l.worker),
                &json!({"trusted": true})
            )
            .status,
        200
    );
    // Some of the real worker's trials have arrived.
    assert_eq!(post_trials(&owner, &l, &reports(2)).status, 200);
    // Another run-role caller reads the id from the worker list and tries every endpoint with it.
    let other = client(srv.addr(), RUN);
    assert!(other.get("/api/workers").body.contains(&l.worker));
    let early = json!({"status": "passed", "steps_executed": 2, "trials": 2, "elapsed_ms": 1,
        "failure": null, "error": null, "trials_reported": 2});
    let job = &l.job_id;
    for secret in [None, Some("guess"), Some("")] {
        let with = |mut v: Value| {
            if let Some(s) = secret {
                v["worker_secret"] = json!(s);
            }
            v
        };
        let calls = [
            (
                format!("/api/worker/jobs/{job}/complete"),
                with(json!({"worker_id": l.worker, "outcome": early})),
            ),
            (
                format!("/api/worker/jobs/{job}/fail"),
                with(json!({"worker_id": l.worker, "error": "x"})),
            ),
            (
                format!("/api/worker/jobs/{job}/progress"),
                with(json!({"worker_id": l.worker, "steps": 1, "trials": 1})),
            ),
            (
                format!("/api/worker/jobs/{job}/heartbeat"),
                with(json!({"worker_id": l.worker})),
            ),
            (
                "/api/worker/lease".to_string(),
                with(json!({"worker_id": l.worker})),
            ),
        ];
        for (path, body) in calls {
            let r = other.post(&path, &body);
            assert_eq!(r.status, 400, "{path} with {secret:?}: {}", r.body);
        }
    }
    assert_eq!(
        state_of(&owner, &l.run_id),
        "running",
        "the run was not finished early"
    );
    // The real worker, with its secret, can still finish, and its later trials are accepted.
    assert_eq!(post_trials(&owner, &l, &reports(4)).status, 200);
    let done = owner.post(
        &format!("/api/worker/jobs/{job}/complete"),
        &json!({"worker_id": l.worker, "worker_secret": l.secret,
            "outcome": {"status": "passed", "steps_executed": 4, "trials": 4, "elapsed_ms": 1,
                "failure": null, "error": null, "trials_reported": 4}}),
    );
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(store.count_trials(&l.run_id).unwrap(), 4);
}

#[test]
fn a_passed_run_that_holds_fewer_trials_than_its_budget_is_marked_incomplete() {
    let (srv, _) = boot(0);
    let owner = client(srv.addr(), RUN);
    let run_id = owner
        .post(
            "/api/runs",
            &json!({"target": "pubsub-atomic", "seed": 1,
                "budget": {"max_steps": 1000, "max_trials": 10, "wall_secs": 30}}),
        )
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (worker, secret) = register(&owner, "remote");
    let job = owner
        .post(
            "/api/worker/lease",
            &json!({"worker_id": worker, "worker_secret": secret}),
        )
        .json()["job_id"]
        .as_str()
        .expect("leased")
        .to_string();
    let l = Leased {
        run_id: run_id.clone(),
        job_id: job,
        worker,
        secret,
    };
    assert_eq!(post_trials(&owner, &l, &reports(3)).status, 200);
    let done = owner.post(
        &format!("/api/worker/jobs/{}/complete", l.job_id),
        &json!({"worker_id": l.worker, "worker_secret": l.secret,
            "outcome": {"status": "passed", "steps_executed": 3, "trials": 3, "elapsed_ms": 1,
                "failure": null, "error": null, "trials_reported": 3}}),
    );
    assert_eq!(done.status, 200, "{}", done.body);
    let run = owner.get(&format!("/api/runs/{run_id}")).json();
    assert_eq!(
        run["run"]["ingest_incomplete"], true,
        "3 of 10 trials: {run}"
    );
    assert_eq!(run["properties"]["ingest_incomplete"], true);
}

#[test]
fn worker_secrets_are_shown_once_and_never_listed() {
    let (srv, _) = boot(1);
    let run = client(srv.addr(), RUN);
    let (id, secret) = register(&run, "remote");
    assert!(secret.len() >= 32);
    for token in [READ, RUN, ADMIN] {
        let listed = client(srv.addr(), token).get("/api/workers").body;
        assert!(!listed.contains(&secret), "{listed}");
        assert!(!listed.to_lowercase().contains("secret"), "{listed}");
        assert!(
            !listed.contains(&hairspring_store::sha256_hex(secret.as_bytes())),
            "{listed}"
        );
    }
    let (_, second) = register(&run, "another");
    assert_ne!(secret, second, "every worker gets its own");
    assert!(run.get("/api/workers").body.contains(&id));
}

#[test]
fn a_leased_worker_posts_trials_and_the_run_shows_its_properties_as_unverified() {
    let (srv, store) = boot(0);
    let c = client(srv.addr(), RUN);
    let l = lease_as_remote(&srv, 3);
    let r = post_trials(&c, &l, &reports(3));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["inserted"], 3);
    assert_eq!(
        post_trials(&c, &l, &reports(3)).json()["duplicates"],
        3,
        "a resend is harmless"
    );
    let done = c.post(
        &format!("/api/worker/jobs/{}/complete", l.job_id),
        &json!({"worker_id": l.worker, "worker_secret": l.secret, "outcome": {"status": "passed", "steps_executed": 3, "trials": 3, "elapsed_ms": 1, "failure": null, "error": null, "trials_reported": 3}}),
    );
    assert_eq!(done.status, 200, "{}", done.body);
    let run = c.get(&format!("/api/runs/{}", l.run_id)).json();
    assert_eq!(run["run"]["catalog_size"], 1);
    assert_eq!(run["properties"]["counts"]["pass"], 1);
    assert_eq!(run["properties"]["unverified_worker_data"], true);
    assert_eq!(run["properties"]["ingest_incomplete"], false);
    assert_eq!(run["properties"]["guest"]["violation_ids"], json!([]));
    let raw = c.get(&format!("/api/runs/{}", l.run_id)).body;
    assert!(
        !raw.contains("property_summary_json"),
        "the program's own text appears only under `guest`: {raw}"
    );
    assert_eq!(store.count_trials(&l.run_id).unwrap(), 3);
}

#[test]
fn the_ingest_endpoint_checks_who_is_calling_and_what_they_send() {
    let (srv, store) = boot(0);
    let run = client(srv.addr(), RUN);
    let l = lease_as_remote(&srv, 3);
    let path = format!("/api/worker/jobs/{}/trials", l.job_id);
    let body = json!({"worker_id": l.worker, "trials": reports(1)});
    assert_eq!(
        client(srv.addr(), READ).post(&path, &body).status,
        403,
        "read tokens cannot post"
    );
    let other = run
        .post("/api/worker/register", &json!({"name": "other"}))
        .json()["worker_id"]
        .as_str()
        .unwrap()
        .to_string();
    let stranger = run.post(&path, &json!({"worker_id": other, "trials": reports(1)}));
    assert_eq!(stranger.status, 400, "{}", stranger.body);
    assert_eq!(
        run.post("/api/worker/jobs/job_zzzz/trials", &body).status,
        404
    );
    assert_eq!(
        run.post(&path, &json!({"worker_id": "bad id", "trials": []}))
            .status,
        400
    );
    assert_eq!(run.post_raw(&path, "not json").status, 400);
    // The server may answer 413 or drop the connection while the oversized body is still being sent.
    let big = run
        .agent
        .post(format!("{}{}", run.base, path))
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {RUN}"))
        .send("x".repeat(3 * 1024 * 1024));
    if let Ok(resp) = big {
        assert_eq!(resp.status().as_u16(), 413);
    }
    let mut forged = reports(1);
    forged[0].digest = vec![9, 9];
    assert_eq!(post_trials(&run, &l, &forged).status, 400);
    let too_many = reports(257);
    assert_eq!(post_trials(&run, &l, &too_many).status, 400);
    assert_eq!(
        store.count_trials(&l.run_id).unwrap(),
        0,
        "nothing was stored"
    );
}

#[test]
fn only_an_admin_can_set_worker_trust_and_it_shows_in_the_worker_list() {
    let (srv, _) = boot(0);
    let run = client(srv.addr(), RUN);
    let id = run
        .post("/api/worker/register", &json!({"name": "w"}))
        .json()["worker_id"]
        .as_str()
        .unwrap()
        .to_string();
    let path = format!("/api/workers/{id}/trust");
    assert_eq!(run.post(&path, &json!({"trusted": true})).status, 403);
    assert_eq!(
        client(srv.addr(), READ)
            .post(&path, &json!({"trusted": true}))
            .status,
        403
    );
    let admin = client(srv.addr(), ADMIN);
    assert_eq!(
        admin
            .post(
                "/api/workers/wrk_0000000000000000/trust",
                &json!({"trusted": true})
            )
            .status,
        404
    );
    assert_eq!(
        admin
            .post("/api/workers/not-an-id/trust", &json!({"trusted": true}))
            .status,
        404
    );
    assert_eq!(admin.post(&path, &json!({"trusted": "yes"})).status, 400);
    assert_eq!(admin.post(&path, &json!({"trusted": true})).status, 200);
    let workers = run.get("/api/workers").json();
    assert_eq!(workers["workers"][0]["trusted"], true);
    assert_eq!(admin.post(&path, &json!({"trusted": false})).status, 200);
    assert_eq!(
        run.get("/api/workers").json()["workers"][0]["trusted"],
        false
    );
}

#[test]
fn the_embedded_worker_is_trusted_and_a_new_remote_worker_is_not() {
    let (srv, _) = boot(1);
    let run = client(srv.addr(), RUN);
    run.post("/api/worker/register", &json!({"name": "remote"}));
    let workers = run.get("/api/workers").json();
    let by_name = |name: &str| {
        workers["workers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["name"] == name)
            .unwrap()["trusted"]
            .clone()
    };
    assert_eq!(by_name("embedded-0"), true);
    assert_eq!(by_name("remote"), false);
}

// A real remote worker running a real Hermit backend against a fake `hermit`, end to end.

const FAKE_HERMIT: &str = r#"#!/bin/sh
shift
seed=0
while [ "$1" != "--" ]; do
  case "$1" in --sched-seed=*) seed=${1#--sched-seed=};; esac
  shift
done
shift
echo "hermit: virtual-time epoch=fake" >&2
FAKE_SCHED_SEED=$seed exec "$@"
"#;

fn scratch(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-ingest-{name}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fake_hermit(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("fake-hermit");
    std::fs::write(&p, FAKE_HERMIT).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn hermit_engine(dir: &Path, script: &str, trials: u64) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: Some(10),
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget {
            max_steps: 1,
            max_trials: trials,
            wall_secs: 60,
        },
    };
    let settings = HermitSettings {
        bin: fake_hermit(dir),
        trial_timeout: Duration::from_secs(10),
        ..HermitSettings::default()
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(HermitBackend::new(settings)))
}

fn server_for(engine: &Engine) -> (RunningServer, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), tokens());
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = 0;
    (
        start(cfg, Service::new(store.clone()), engine.clone()).expect("server starts"),
        store,
    )
}

#[test]
fn a_remote_worker_running_hermit_delivers_every_trial_and_the_server_folds_them() {
    let dir = scratch("e2e");
    let script = r#"
cat <<'EOF'
HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"wal.ckpt","type":"always","must_hit":true,"msg":"checkpoint stays ahead"}
HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"wal.ckpt","hits":4,"trues":4,"falses":0,"first_false":null,"first_true":null}
HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":4,"dropped":0,"threads":0}
EOF
"#;
    let engine = hermit_engine(&dir, script, 6);
    let (srv, store) = server_for(&engine);
    let c = client(srv.addr(), RUN);
    let run_id = c
        .post("/api/runs", &json!({"target": "svc", "seed": 1}))
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let (addr, stop2, engine2) = (srv.addr(), stop.clone(), engine.clone());
    let worker = std::thread::spawn(move || {
        let mut cfg =
            hairspring_worker::WorkerConfig::new(format!("http://{addr}"), "remote-hermit");
        cfg.token = Some(RUN.to_string());
        cfg.poll = Duration::from_millis(50);
        hairspring_worker::run_loop(&engine2, &cfg, &stop2)
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let run = loop {
        let r = c.get(&format!("/api/runs/{run_id}")).json();
        if r["run"]["state"] == "passed" {
            break r;
        }
        assert!(Instant::now() < deadline, "run never finished: {r}");
        std::thread::sleep(Duration::from_millis(50));
    };
    stop.store(true, Ordering::SeqCst);
    worker.join().unwrap().unwrap();
    assert_eq!(store.count_trials(&run_id).unwrap(), 6);
    assert_eq!(run["run"]["ingest_incomplete"], false, "{run}");
    assert_eq!(run["properties"]["counts"]["pass"], 1, "{run}");
    assert_eq!(
        run["properties"]["unverified_worker_data"], true,
        "an untrusted remote worker"
    );
    let stats = store.list_run_properties(&run_id).unwrap();
    assert_eq!(
        (
            stats[0].trials,
            stats[0].trials_rated,
            stats[0].trials_false
        ),
        (6, 6, 0)
    );
    let rows = store.list_trials(&run_id, None, 10).unwrap();
    assert!(rows
        .iter()
        .all(|t| t.worker_id.starts_with("wrk_") && !t.worker_trusted));
    assert!(
        rows[0].detail_ref.is_some(),
        "the first trials keep their detail"
    );
}

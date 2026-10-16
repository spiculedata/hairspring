//! The server's own embedded worker, end to end: a process target that produces per-trial records is run by the
//! default `embedded_workers = 1` worker, and the records reach the store as trusted rows.
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_hermit::{HermitBackend, HermitSettings};
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const ADMIN: &str = "admin-token-0123456789abcdef";
const TRIALS: u64 = 6;

/// Passes through to the guest after `--`, exporting the seed; ignores every other flag.
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

/// A valid guest report: one always-property, one assert line with both witness keys, `end.sites` equal to the
/// catalogued ids and `end.threads` equal to the thread lines.
const GUEST: &str = r#"
cat <<'EOF'
HAIRSPRING_EVENT {"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}
HAIRSPRING_EVENT {"v":1,"kind":"catalog","proc":"p1","id":"wal.ckpt","type":"always","must_hit":true,"msg":"checkpoint stays ahead"}
HAIRSPRING_EVENT {"v":1,"kind":"assert","proc":"p1","id":"wal.ckpt","hits":4,"trues":4,"falses":0,"first_false":null,"first_true":null}
HAIRSPRING_EVENT {"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":4,"dropped":0,"threads":0}
EOF
"#;

fn tokens() -> Auth {
    Auth::from_spec(&format!("{READ}:read,{RUN}:run,{ADMIN}:admin")).unwrap()
}

fn scratch() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "hairspring-embedded-{}-{nanos}",
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

fn hermit_engine(dir: &Path) -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/sh".into(), "-c".into(), GUEST.into()],
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
            max_trials: TRIALS,
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

fn boot(workers: usize) -> (RunningServer, Arc<dyn Store>) {
    let engine = hermit_engine(&scratch());
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
    fn get(&self, path: &str) -> Resp {
        let r = self
            .agent
            .get(format!("{}{}", self.base, path))
            .header("Authorization", format!("Bearer {}", self.token));
        Self::finish(r.call())
    }
    fn post(&self, path: &str, v: &Value) -> Resp {
        let r = self
            .agent
            .post(format!("{}{}", self.base, path))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.token));
        Self::finish(r.send(v.to_string()))
    }
    fn finish(r: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Resp {
        let mut resp = r.expect("transport ok");
        Resp {
            status: resp.status().as_u16(),
            body: resp.body_mut().read_to_string().unwrap_or_default(),
        }
    }
}

fn start_run(c: &Client) -> String {
    c.post("/api/runs", &json!({"target": "svc", "seed": 1}))
        .json()["run"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_embedded_worker_finishes_a_process_run_and_stores_trusted_trial_rows() {
    let (srv, store) = boot(1);
    let c = client(srv.addr(), RUN);
    let run_id = start_run(&c);

    let deadline = Instant::now() + Duration::from_secs(60);
    let run = loop {
        let r = c.get(&format!("/api/runs/{run_id}")).json();
        if r["run"]["state"] != "queued" && r["run"]["state"] != "running" {
            break r;
        }
        assert!(Instant::now() < deadline, "run never finished: {r}");
        std::thread::sleep(Duration::from_millis(50));
    };

    assert_eq!(run["run"]["state"], "passed", "{run}");
    assert_eq!(store.count_trials(&run_id).unwrap(), TRIALS);
    assert_eq!(run["run"]["ingest_incomplete"], false, "{run}");
    assert_eq!(
        run["properties"]["unverified_worker_data"], false,
        "an embedded worker is trusted: {run}"
    );
    let rows = store.list_trials(&run_id, None, 100).unwrap();
    assert_eq!(rows.len() as u64, TRIALS);
    assert!(
        rows.iter().all(|t| t.worker_trusted),
        "every row from the embedded worker is trusted"
    );

    let props = c.get(&format!("/api/runs/{run_id}/properties"));
    assert_eq!(props.status, 200, "{}", props.body);
    assert!(props.body.contains("wal.ckpt"), "{}", props.body);
}

#[test]
fn a_run_token_without_a_secret_is_refused_on_every_endpoint_for_an_embedded_workers_id() {
    let (srv, store) = boot(1);
    let c = client(srv.addr(), RUN);
    let worker = store
        .list_workers()
        .unwrap()
        .into_iter()
        .find(|w| w.name == "embedded-0")
        .expect("the embedded worker is registered")
        .id;
    assert!(worker.starts_with("wrk_"), "{worker}");
    let outcome = json!({"status": "passed", "steps_executed": 1, "trials": 1, "elapsed_ms": 1,
        "failure": null, "error": null, "trials_reported": 0});
    for secret in [None, Some("guess"), Some("")] {
        let with = |mut v: Value| {
            if let Some(s) = secret {
                v["worker_secret"] = json!(s);
            }
            v
        };
        let job = "/api/worker/jobs/job_0123456789abcdef";
        let calls = [
            (
                "/api/worker/lease".to_string(),
                with(json!({"worker_id": worker})),
            ),
            (
                format!("{job}/trials"),
                with(json!({"worker_id": worker, "trials": []})),
            ),
            (
                format!("{job}/complete"),
                with(json!({"worker_id": worker, "outcome": outcome})),
            ),
            (
                format!("{job}/fail"),
                with(json!({"worker_id": worker, "error": "x"})),
            ),
            (
                format!("{job}/progress"),
                with(json!({"worker_id": worker, "steps": 1, "trials": 1})),
            ),
            (
                format!("{job}/heartbeat"),
                with(json!({"worker_id": worker})),
            ),
        ];
        for (path, body) in calls {
            let r = c.post(&path, &body);
            assert_eq!(r.status, 400, "{path} with {secret:?}: {}", r.body);
            assert!(r.body.contains("invalid worker credential"), "{}", r.body);
        }
    }
}

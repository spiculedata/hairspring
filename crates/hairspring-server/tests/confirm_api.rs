//! The stored confirmation checks of a failure over real HTTP: the read role is enough, ids are validated first, the
//! runner's own id is never served, and free text sits under `guest`.
use hairspring_engine::config::Config;
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus};
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{NewFailure, NewRun, SqliteStore, Store};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const RUN: &str = "run-token-0123456789abcdef";
const RUNNER: &str = "runner-secret-label";
const HOSTILE: &str = "<script>alert(1)</script>";

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

fn get(addr: SocketAddr, token: Option<&str>, path: &str) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut r = agent.get(format!("http://{addr}{path}"));
    if let Some(t) = token {
        r = r.header("Authorization", format!("Bearer {t}"));
    }
    let mut resp = r.call().expect("transport ok");
    Resp {
        status: resp.status().as_u16(),
        body: resp.body_mut().read_to_string().unwrap_or_default(),
    }
}

fn pass(kind: CheckKind) -> CheckResult {
    CheckResult::new(kind, CheckStatus::Pass)
        .with_counts(1, 0, 0)
        .with_times(10, 20)
}

/// A server with one stored failure that has one confirmation. Returns the run id and the failure id.
fn boot() -> (RunningServer, String, String) {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store.clone());
    let run = store
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = store.put_blob(b"bundle").unwrap();
    let failure = store
        .add_failure(
            &NewFailure {
                run_id: run.id.clone(),
                target: "t".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap();
    let rerun = pass(CheckKind::SameSeedRerun)
        .with_command(format!("hermit run -- {HOSTILE}"))
        .with_detail("note", HOSTILE)
        .with_artefact("00aa11bb22cc33dd")
        .with_environment("host.container", "none");
    let replay = pass(CheckKind::BundleReplay).with_detail("environment_changed", "false");
    svc.record_confirmation(&failure.id, RUNNER, true, &[rerun, replay])
        .unwrap();
    let auth = Auth::from_spec(&format!("{READ}:read,{RUN}:run")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    (
        start(cfg, svc, engine).expect("server starts"),
        run.id,
        failure.id,
    )
}

#[test]
fn the_checks_need_a_token_and_the_read_role_is_enough() {
    let (server, _, failure) = boot();
    let addr = server.addr();
    let path = format!("/api/failures/{failure}/checks");
    assert_eq!(get(addr, None, &path).status, 401);
    assert_eq!(
        get(addr, Some("wrong-token-0123456789abcdef"), &path).status,
        401
    );
    assert_eq!(get(addr, Some(READ), &path).status, 200);
}

#[test]
fn a_malformed_or_unknown_failure_id_is_a_404() {
    let (server, _, _) = boot();
    let addr = server.addr();
    for bad in [
        "nope",
        "fail_xyz",
        "fail_0123456789abcdeg",
        "fail_0123456789abcdef0",
        "..%2f..",
        "fail_0000000000000000",
    ] {
        let r = get(addr, Some(READ), &format!("/api/failures/{bad}/checks"));
        assert_eq!(r.status, 404, "{bad}");
    }
}

#[test]
fn the_answer_has_the_finding_the_checks_and_the_guest_fence() {
    let (server, _, failure) = boot();
    let r = get(
        server.addr(),
        Some(READ),
        &format!("/api/failures/{failure}/checks"),
    );
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["failure_id"], failure.as_str());
    assert_eq!(v["finding"], "confirmed");
    assert_eq!(
        v["environment_pinned"], false,
        "the replay recorded no environment"
    );
    let checks = v["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0]["kind"], "same_seed_rerun");
    assert_eq!(checks[0]["status"], "pass");
    assert_eq!(checks[0]["counts"]["passed"], 1);
    assert_eq!(checks[0]["artefacts"][0], "00aa11bb22cc33dd");
    assert_eq!(checks[0]["environment"]["host.container"], "none");
    assert_eq!(checks[0]["runner_trusted"], true);
    assert!(checks[0]["guest"]["command"]
        .as_str()
        .unwrap()
        .contains(HOSTILE));
    assert_eq!(checks[0]["guest"]["detail"]["note"], HOSTILE);
    let mut found = Vec::new();
    strings_containing(&v, HOSTILE, "", &mut found);
    assert!(
        found.iter().all(|p| p.contains(".guest")),
        "guest text outside the fence: {found:?}"
    );
}

fn strings_containing(v: &Value, needle: &str, path: &str, out: &mut Vec<String>) {
    match v {
        Value::String(s) if s.contains(needle) => out.push(path.to_owned()),
        Value::Array(a) => a
            .iter()
            .for_each(|x| strings_containing(x, needle, &format!("{path}[]"), out)),
        Value::Object(o) => o
            .iter()
            .for_each(|(k, x)| strings_containing(x, needle, &format!("{path}.{k}"), out)),
        _ => {}
    }
}

#[test]
fn the_runner_label_is_never_served() {
    let (server, run, failure) = boot();
    let addr = server.addr();
    for path in [
        format!("/api/failures/{failure}/checks"),
        format!("/api/failures/{failure}"),
        format!("/api/runs/{run}"),
        "/api/failures".to_string(),
    ] {
        let r = get(addr, Some(READ), &path);
        assert_eq!(r.status, 200, "{path}");
        assert!(!r.body.contains(RUNNER), "{path}: {}", r.body);
    }
}

#[test]
fn the_run_page_carries_each_failures_finding() {
    let (server, run, _) = boot();
    let v = get(server.addr(), Some(READ), &format!("/api/runs/{run}")).json();
    let finding = &v["failures"][0]["finding"];
    assert_eq!(finding["status"], "confirmed");
    assert_eq!(finding["checks"], 2);
}

#[test]
fn a_failure_with_no_checks_is_a_candidate_with_an_empty_list() {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let run = store
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = store.put_blob(b"bundle").unwrap();
    let failure = store
        .add_failure(
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
        .unwrap();
    let auth = Auth::from_spec(&format!("{READ}:read")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    let server = start(cfg, Service::new(store), engine).unwrap();
    let v = get(
        server.addr(),
        Some(READ),
        &format!("/api/failures/{}/checks", failure.id),
    )
    .json();
    assert_eq!(v["finding"], "candidate");
    assert_eq!(v["checks"].as_array().unwrap().len(), 0);
}

//! `get_run` carries a short property summary, with the program's own text fenced as untrusted.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_mcp::McpServer;
use hairspring_props::{AssertKind, RunCatalog, TrialEvents, TrialProps};
use hairspring_service::{Role, Service, StartRun};
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;

fn props(id: &str, kind: AssertKind, trues: u64, falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{}","must_hit":{}}}"#, kind.as_str(), kind.must_hit())),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":null,"first_true":null}}"#, trues + falses)),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

/// Many ids that fail and many that are never met, in two trials.
fn stored_run(svc: &Service, engine: &Engine, ids_each: usize) -> String {
    let worker = Service::local_worker_id();
    let run = svc
        .start_run(
            engine,
            &StartRun {
                target: "lock-lease".into(),
                seed: Some(1),
                budget: None,
            },
        )
        .unwrap();
    let job = svc.lease(&worker).unwrap().unwrap();
    let mut catalog = RunCatalog::new();
    let mut batch = Vec::new();
    for (seq, seed) in [(0u64, 20u64), (1, 21)] {
        let mut trial = props("f.0", AssertKind::Always, 1, 0);
        trial.properties.clear();
        for i in 0..ids_each {
            let fail = props(&format!("fail.{i:03}"), AssertKind::Always, 0, 1);
            let unmet = props(&format!("unmet.{i:03}"), AssertKind::Sometimes, 0, 2);
            trial.properties.extend(fail.properties);
            trial.properties.extend(unmet.properties);
        }
        trial.properties.sort_by(|a, b| a.id.cmp(&b.id));
        let rec = std::mem::take(&mut catalog).record(&trial, false, 16384);
        catalog = rec.catalog;
        batch.push(TrialReport {
            seq,
            seed,
            verdict: ReportedVerdict::Violation,
            reason: String::new(),
            exit_code: Some(1),
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
        });
    }
    svc.ingest_trials(&job.job_id, &worker, &batch).unwrap();
    let outcome = RunOutcome {
        trials_reported: Some(2),
        ..RunOutcome::passed(2, 2, 1)
    };
    svc.complete(&job.job_id, &worker, &outcome).unwrap();
    run.id
}

fn get_run(server: &McpServer, id: &str) -> (Value, usize) {
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_run", "arguments": {"run_id": id}}});
    let text = server.handle_message(&req.to_string(), Role::Read).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    (v["result"]["structuredContent"].clone(), text.len())
}

#[test]
fn get_run_summarises_the_properties_and_fences_the_programs_ids() {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store);
    let id = stored_run(&svc, &engine, 3);
    let server = McpServer::new(svc, engine, true);
    let (run, _) = get_run(&server, &id);
    let p = &run["properties"];
    assert_eq!(p["catalog_size"], 6);
    assert_eq!(p["counts"]["violation"], 3);
    assert_eq!(p["counts"]["unmet"], 3);
    assert_eq!(p["first_failing_seed"], "20");
    assert_eq!(p["unverified_worker_data"], false);
    assert_eq!(p["ingest_incomplete"], false);
    assert!(p["guest"]["note"].as_str().unwrap().contains("untrusted"));
    assert_eq!(p["guest"]["violation_ids"].as_array().unwrap().len(), 3);
    assert!(
        p.get("violation_ids").is_none(),
        "ids appear only under `guest`"
    );
}

#[test]
fn the_summary_stays_small_however_many_properties_there_are() {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store);
    let id = stored_run(&svc, &engine, 400);
    let server = McpServer::new(svc, engine, true);
    let (run, bytes) = get_run(&server, &id);
    assert_eq!(run["properties"]["counts"]["violation"], 400);
    assert_eq!(
        run["properties"]["guest"]["violation_ids"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(
        run["properties"]["guest"]["unmet_ids"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert!(bytes < 64 * 1024, "{bytes} bytes");
}

#[test]
fn a_run_without_property_data_has_no_summary() {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let svc = Service::new(store);
    let run = svc
        .start_run(
            &engine,
            &StartRun {
                target: "lock-lease".into(),
                seed: Some(1),
                budget: None,
            },
        )
        .unwrap();
    let server = McpServer::new(svc, engine, true);
    let (got, _) = get_run(&server, &run.id);
    assert!(got.get("properties").is_none(), "{got}");
}

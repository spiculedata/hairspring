//! `hairspring show` prints a stored run's property table, and says when the data behind it cannot be relied on.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::{AssertKind, RunCatalog, TrialEvents, TrialProps};
use hairspring_service::{Service, StartRun};
use hairspring_store::{SqliteStore, Store};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-cli-props-{name}-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn show(dir: &Path, db: &Path, id: &str) -> String {
    let o: Output = Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .args(["--db", &db.to_string_lossy(), "show", id])
        .current_dir(dir)
        .output()
        .expect("binary runs");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn props(id: &str, kind: AssertKind, trues: u64, falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{}","must_hit":{},"msg":"m"}}"#, kind.as_str(), kind.must_hit())),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":null,"first_true":null}}"#, trues + falses)),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

fn reports(trials: &[(u64, TrialProps)]) -> Vec<TrialReport> {
    let mut catalog = RunCatalog::new();
    trials
        .iter()
        .enumerate()
        .map(|(i, (seed, p))| {
            let rec = std::mem::take(&mut catalog).record(p, false, 16384);
            catalog = rec.catalog;
            TrialReport {
                seq: i as u64,
                seed: *seed,
                verdict: if p.verdict().is_empty() {
                    ReportedVerdict::Pass
                } else {
                    ReportedVerdict::Violation
                },
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
        .collect()
}

/// A stored run whose trials came from the given worker (`None` for the process's own).
fn stored_run(db: &Path, remote: bool, trials: &[(u64, TrialProps)]) -> String {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(db).unwrap());
    let svc = Service::new(store);
    let (worker, secret) = if remote {
        let (w, secret) = svc.register_remote_worker("remote", "{}").unwrap();
        (w.id, Some(secret))
    } else {
        (Service::local_worker_id(), None)
    };
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
    let job = svc.lease(&worker).unwrap().unwrap();
    match secret.as_deref() {
        Some(s) => svc.ingest_trials_with_secret(&job.job_id, &worker, Some(s), &reports(trials)),
        None => svc.ingest_trials(&job.job_id, &worker, &reports(trials)),
    }
    .unwrap();
    let n = trials.len() as u64;
    let outcome = RunOutcome {
        trials_reported: Some(n),
        ..RunOutcome::passed(n, n, 1)
    };
    svc.complete(&job.job_id, &worker, &outcome).unwrap();
    run.id
}

#[test]
fn a_stored_run_prints_its_property_table_with_rates() {
    let d = scratch("table");
    let db = d.join("r.db");
    let trials: Vec<_> = (0..10)
        .map(|i| {
            (
                i,
                props("wal.ckpt", AssertKind::Always, 3, u64::from(i == 4)),
            )
        })
        .collect();
    let id = stored_run(&db, false, &trials);
    let text = show(&d, &db, &id);
    assert!(
        text.contains("VIOLATION") && text.contains("'wal.ckpt'"),
        "{text}"
    );
    assert!(text.contains("1/10"), "the rate is k/n: {text}");
    assert!(text.contains("first failure at seed 4"), "{text}");
    assert!(
        !text.contains("unverified"),
        "the process's own worker is trusted: {text}"
    );
}

#[test]
fn data_from_a_worker_that_is_not_trusted_is_labelled() {
    let d = scratch("untrusted");
    let db = d.join("r.db");
    let trials = vec![(1, props("a.id", AssertKind::Always, 3, 0))];
    let id = stored_run(&db, true, &trials);
    let text = show(&d, &db, &id);
    assert!(text.contains("unverified worker data"), "{text}");
}

#[test]
fn a_run_with_missing_trial_rows_says_so_and_claims_no_unmet() {
    let d = scratch("incomplete");
    let db = d.join("r.db");
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let svc = Service::new(Arc::new(SqliteStore::open(&db).unwrap()));
    let worker = Service::local_worker_id();
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
    let job = svc.lease(&worker).unwrap().unwrap();
    let never = props("s.id", AssertKind::Sometimes, 0, 4);
    svc.ingest_trials(&job.job_id, &worker, &reports(&[(1, never)]))
        .unwrap();
    // The worker says it ran three trials; one arrived.
    let outcome = RunOutcome {
        trials_reported: Some(3),
        ..RunOutcome::passed(3, 3, 1)
    };
    svc.complete(&job.job_id, &worker, &outcome).unwrap();
    let text = show(&d, &db, &run.id);
    assert!(text.contains("incomplete"), "{text}");
    assert!(
        text.contains("UNRATED") && !text.contains("UNMET"),
        "{text}"
    );
}

fn cli(dir: &Path, db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hairspring"))
        .arg("--db")
        .arg(db)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("binary runs")
}

/// Every stdout line parsed on its own; stdout must hold nothing but NDJSON.
fn ndjson_lines(o: &Output) -> Vec<serde_json::Value> {
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.is_empty() || text.ends_with('\n'), "{text}");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect()
}

fn failing_trials() -> Vec<(u64, TrialProps)> {
    (0..4)
        .map(|i| {
            (
                i,
                props("wal.ckpt", AssertKind::Always, 3, u64::from(i == 2)),
            )
        })
        .collect()
}

const NEXT_NULL: &str = r#"{"_next":null}"#;

#[test]
fn results_json_is_one_line_per_run_then_the_control_line() {
    let d = scratch("results-json");
    let db = d.join("r.db");
    let id = stored_run(&db, false, &failing_trials());
    let o = cli(&d, &db, &["results", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[1], NEXT_NULL);
    let run = &ndjson_lines(&o)[0];
    assert_eq!(run["id"], id.as_str());
    assert_eq!(run["state"], "passed");
    assert_eq!(run["target"], "lock-lease");
    assert!(run.get("config_json").is_some());
    // Keys are sorted, so the line is the same bytes on every run and every version of the serialiser.
    assert!(
        lines[0].starts_with(r#"{"backend":"#),
        "first key is the alphabetically first: {}",
        lines[0]
    );
    let keys: Vec<&String> = run.as_object().unwrap().keys().collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
}

#[test]
fn results_json_on_an_empty_store_is_only_the_control_line() {
    let d = scratch("results-json-empty");
    let db = d.join("r.db");
    let o = cli(&d, &db, &["results", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&o.stdout), format!("{NEXT_NULL}\n"));
}

#[test]
fn results_json_honours_limit_and_text_mode_is_unchanged() {
    let d = scratch("results-limit");
    let db = d.join("r.db");
    stored_run(&db, false, &failing_trials());
    stored_run(&db, false, &failing_trials());
    let two = ndjson_lines(&cli(&d, &db, &["results", "--json"]));
    assert_eq!(two.len(), 3);
    let one = ndjson_lines(&cli(&d, &db, &["results", "--json", "--limit", "1"]));
    assert_eq!(one.len(), 2);
    let text = cli(&d, &db, &["results"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("passed"));
}

#[test]
fn show_json_is_the_run_then_one_line_per_property_then_the_control_line() {
    let d = scratch("show-json");
    let db = d.join("r.db");
    let id = stored_run(&db, false, &failing_trials());
    let o = cli(&d, &db, &["show", &id, "--json"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let lines = ndjson_lines(&o);
    assert_eq!(lines.len(), 3, "{lines:?}");
    let run = &lines[0];
    assert_eq!(run["_kind"], "run");
    assert_eq!(run["run"]["id"], id.as_str());
    assert_eq!(run["run"]["seed"], 1);
    assert!(run["failures"].is_array());
    assert_eq!(run["properties"]["unverified_worker_data"], false);
    assert!(run["page"]["claim"].is_object() && run["page"]["total"] == 1);
    assert!(run["page"].get("properties").is_none());
    let prop = &lines[1];
    assert_eq!(prop["_kind"], "property");
    assert_eq!(prop["idx"], 0);
    assert_eq!(prop["status"], "violation");
    assert_eq!(prop["first_failing_seed"], "2");
    assert_eq!(prop["trials_false"], 1);
    assert_eq!(lines[2], serde_json::json!({ "_next": null }));
    let raw = String::from_utf8_lossy(&o.stdout).into_owned();
    assert!(raw.starts_with(r#"{"_kind":"run","failures":"#), "{raw}");
    assert!(
        raw.contains("\n{\"_kind\":\"property\",\"evals\":"),
        "{raw}"
    );
}

#[test]
fn show_json_keeps_the_guest_fence_and_the_trust_label() {
    let d = scratch("show-json-guest");
    let db = d.join("r.db");
    let id = stored_run(&db, true, &failing_trials());
    let lines = ndjson_lines(&cli(&d, &db, &["show", &id, "--json"]));
    assert_eq!(lines[0]["properties"]["unverified_worker_data"], true);
    let prop = &lines[1];
    assert!(prop["guest"].is_object(), "guest strings stay under guest");
    let top: Vec<&String> = prop.as_object().unwrap().keys().collect();
    assert!(!top.iter().any(|k| *k == "msg" || *k == "id"), "{top:?}");
}

#[test]
fn show_json_for_an_unknown_run_prints_nothing_on_stdout() {
    let d = scratch("show-json-missing");
    let db = d.join("r.db");
    let o = cli(&d, &db, &["show", "run_0000000000000000", "--json"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        o.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stdout)
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("no stored run"));
}

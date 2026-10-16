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
    svc.ingest_trials_with_secret(&job.job_id, &worker, secret.as_deref(), &reports(trials))
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

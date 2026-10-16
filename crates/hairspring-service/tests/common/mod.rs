//! Helpers for the service's trial-ingest tests.
#![allow(dead_code)]
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_engine::report::RunOutcome;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::{AssertKind, RunCatalog, TrialEvents, TrialProps};
use hairspring_service::{LeasedJob, Service, StartRun};
use hairspring_store::{new_id, PgStore, SqliteStore, Store};
use std::sync::Arc;

pub mod fixture;

/// An in-memory SQLite store, or, when `HAIRSPRING_TEST_PG_URL` is set, a PostgreSQL store in its own schema.
fn new_store() -> Arc<dyn Store> {
    match std::env::var("HAIRSPRING_TEST_PG_URL") {
        Ok(url) => {
            let schema = new_id("t").replace('-', "_");
            Arc::new(PgStore::connect_in_schema(&url, &schema, 8).expect("connect to PostgreSQL"))
        }
        Err(_) => Arc::new(SqliteStore::open_in_memory().unwrap()),
    }
}

pub fn setup() -> (Service, Arc<dyn Store>, Engine) {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store = new_store();
    (Service::new(store.clone()), store, engine)
}

/// A run with a budget of three trials, leased by `worker`.
pub fn lease(svc: &Service, engine: &Engine, worker: &str) -> LeasedJob {
    lease_with_budget(svc, engine, worker, 3)
}

/// A run with a budget of `trials` trials, leased by `worker`.
pub fn lease_with_budget(svc: &Service, engine: &Engine, worker: &str, trials: u64) -> LeasedJob {
    svc.start_run(
        engine,
        &StartRun {
            target: "lock-lease".into(),
            seed: Some(1),
            budget: Some(hairspring_engine::config::Budget {
                max_steps: 1000,
                max_trials: trials,
                wall_secs: 30,
            }),
        },
    )
    .unwrap();
    svc.lease(worker).unwrap().expect("a job")
}

pub fn props(id: &str, kind: AssertKind, trues: u64, falses: u64) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let must_hit = kind.must_hit();
    let witness = |on: bool| {
        if on {
            r#"{"thread":"w-0","ord":5,"vals":[1,2,3,4]}"#
        } else {
            "null"
        }
    };
    let text = [
        ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string()),
        ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{}","must_hit":{must_hit},"msg":"m"}}"#, kind.as_str())),
        ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":{},"first_true":{}}}"#, trues + falses, witness(falses > 0), witness(trues > 0))),
        ev(r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":1,"dropped":0,"threads":0}"#.to_string()),
    ]
    .concat();
    TrialEvents::from_output(text.as_bytes()).fold()
}

/// One id of a multi-id trial: `(id, kind, msg, trues, falses)`.
pub type IdSpec<'a> = (&'a str, AssertKind, &'a str, u64, u64);

/// A trial in which one process reports several ids, each with its own message.
pub fn props_multi(ids: &[IdSpec<'_>]) -> TrialProps {
    let ev = |j: String| format!("HAIRSPRING_EVENT {j}\n");
    let witness = |on: bool| {
        if on {
            r#"{"thread":"w-0","ord":5,"vals":[1,2,3,4]}"#
        } else {
            "null"
        }
    };
    let mut lines = vec![ev(r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"x","signal_handlers":false}"#.to_string())];
    for (id, kind, msg, _, _) in ids {
        lines.push(ev(format!(r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{}","must_hit":{},"msg":{},"loc":{{"file":"f.c","line":7,"func":"fn"}}}}"#, kind.as_str(), kind.must_hit(), serde_json::to_string(msg).unwrap())));
    }
    for (id, _, _, trues, falses) in ids {
        lines.push(ev(format!(r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":{},"first_true":{}}}"#, trues + falses, witness(*falses > 0), witness(*trues > 0))));
    }
    lines.push(ev(format!(r#"{{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":{},"evals":1,"dropped":0,"threads":0}}"#, ids.len())));
    TrialEvents::from_output(lines.concat().as_bytes()).fold()
}

/// Builds trial reports the way a worker does.
pub struct Recorder {
    catalog: RunCatalog,
    next: u64,
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder {
            catalog: RunCatalog::new(),
            next: 0,
        }
    }

    pub fn trial(
        &mut self,
        seed: u64,
        p: &TrialProps,
        verdict: ReportedVerdict,
        inconclusive: bool,
    ) -> TrialReport {
        let rec = std::mem::take(&mut self.catalog).record(p, inconclusive, 16384);
        self.catalog = rec.catalog;
        let seq = self.next;
        self.next += 1;
        TrialReport {
            seq,
            seed,
            verdict,
            reason: String::new(),
            exit_code: Some(0),
            signal: None,
            behaviour_hash: Some("00aa11bb22cc33dd".to_string()),
            schedule_fp: Some(format!("{:016x}", seed % 3)),
            syscalls: 100,
            work_ops: 4,
            elapsed_ms: 3,
            catalog: rec.updates,
            digest: rec.digest.encode(),
            detail: None,
            output: None,
        }
    }

    pub fn pass(&mut self, seed: u64, p: &TrialProps) -> TrialReport {
        self.trial(seed, p, ReportedVerdict::Pass, false)
    }
}

pub fn finish(svc: &Service, job: &LeasedJob, worker: &str, outcome: RunOutcome) {
    svc.complete(&job.job_id, worker, &outcome).unwrap();
}

pub fn passed(trials: u64, reported: Option<u64>) -> RunOutcome {
    RunOutcome {
        trials_reported: reported,
        ..RunOutcome::passed(trials, trials, 5)
    }
}

pub fn local() -> String {
    Service::local_worker_id()
}

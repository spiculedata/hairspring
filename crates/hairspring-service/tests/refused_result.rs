//! A result the server refuses, reached through the engine path (a local run), ends the run as an error and closes the
//! job, instead of failing the caller and leaving the job leased.
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::report::{FailureReport, ReplayOutcome, RunOutcome};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::EngineError;
use hairspring_service::{Service, StartRun};
use hairspring_store::{RunState, SqliteStore};
use std::collections::BTreeMap;
use std::sync::Arc;

fn failure(seed: u64) -> FailureReport {
    FailureReport {
        target: "svc".into(),
        backend: "fake".into(),
        message: "m".into(),
        original_len: 1,
        shrunk_len: 1,
        steps: vec![],
        bundle: Bundle::new(seed, "t", vec![])
            .with_target("svc")
            .with_param("sched_seed", seed.to_string())
            .encode(),
    }
}

/// A backend whose every run reports further failures, which only a sweep may do.
struct Overreaching;

impl Backend for Overreaching {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Process { .. })
    }
    fn run(
        &self,
        _spec: &TargetSpec,
        _req: &RunRequest,
        _budget: Budget,
        _progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        Ok(RunOutcome::failed(1, 1, 1, failure(1)).with_more_failures(vec![failure(2)]))
    }
    fn replay(&self, _: &TargetSpec, _: &Bundle) -> Result<ReplayOutcome, EngineError> {
        Err(EngineError::BackendUnavailable("fake".into()))
    }
}

fn engine() -> Engine {
    let spec = TargetSpec {
        name: "svc".into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["true".into()],
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
            wall_secs: 10,
        },
    };
    Engine::new(
        Arc::new(Registry::new()),
        Config {
            targets: BTreeMap::from([("svc".to_string(), spec)]),
        },
    )
    .with_backend(Arc::new(Overreaching))
}

#[test]
fn a_run_whose_engine_result_is_refused_ends_as_an_error_with_the_reason() {
    let svc = Service::new(Arc::new(SqliteStore::open_in_memory().unwrap()));
    let start = StartRun {
        target: "svc".into(),
        seed: Some(1),
        budget: None,
    };
    let detail = svc
        .run_sync(&engine(), &start)
        .expect("the run is recorded");
    assert_eq!(detail.run.state, RunState::Error, "{:?}", detail.run);
    let why = detail.run.error.unwrap_or_default();
    assert!(why.contains("refused"), "{why}");
}

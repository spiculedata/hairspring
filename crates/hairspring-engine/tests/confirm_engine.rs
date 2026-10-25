//! `Engine::confirm` resolves the target and the control, refuses a bad request before any run, and hands the checks of
//! the backend back unchanged.
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, ReplayOptions, RunRequest};
use hairspring_engine::config::{Budget, Config, TargetKind, TargetSpec};
use hairspring_engine::confirm::{CheckKind, CheckResult, CheckStatus, ConfirmRequest};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::report::{ReplayOutcome, RunOutcome};
use hairspring_engine::scheduler::SchedulerSpec;
use hairspring_engine::EngineError;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn process(name: &str) -> TargetSpec {
    TargetSpec {
        name: name.into(),
        description: String::new(),
        kind: TargetKind::Process {
            command: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
        },
        budget: Budget::default(),
    }
}

/// Answers `confirm` with one passing rerun check and remembers whom it was asked about.
#[derive(Default)]
struct Recording {
    asked: Mutex<Vec<(String, Option<String>, u32)>>,
}

struct Fake {
    recording: Arc<Recording>,
    supports_confirm: bool,
}

impl Backend for Fake {
    fn name(&self) -> &'static str {
        "hermit"
    }
    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Process { .. })
    }
    fn run(
        &self,
        _: &TargetSpec,
        _: &RunRequest,
        _: Budget,
        _: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        unreachable!("not used")
    }
    fn replay(&self, _: &TargetSpec, _: &Bundle) -> Result<ReplayOutcome, EngineError> {
        unreachable!("not used")
    }
    fn replay_with(
        &self,
        _: &TargetSpec,
        _: &Bundle,
        _: ReplayOptions,
    ) -> Result<ReplayOutcome, EngineError> {
        unreachable!("not used")
    }
    fn confirm(
        &self,
        spec: &TargetSpec,
        control: Option<&TargetSpec>,
        _: &Bundle,
        req: &ConfirmRequest,
        _: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<Vec<CheckResult>, EngineError> {
        if !self.supports_confirm {
            return Err(EngineError::BackendUnavailable("no confirm".into()));
        }
        self.recording.asked.lock().unwrap().push((
            spec.name.clone(),
            control.map(|c| c.name.clone()),
            req.reruns,
        ));
        Ok(vec![CheckResult::new(
            CheckKind::SameSeedRerun,
            CheckStatus::Pass,
        )])
    }
}

fn engine(supports_confirm: bool) -> (Engine, Arc<Recording>) {
    let recording = Arc::new(Recording::default());
    let config = Config {
        targets: ["svc", "fixed"]
            .into_iter()
            .map(|n| (n.to_string(), process(n)))
            .collect(),
    };
    let engine = Engine::new(Arc::new(Registry::new()), config).with_backend(Arc::new(Fake {
        recording: recording.clone(),
        supports_confirm,
    }));
    (engine, recording)
}

fn bundle(target: &str) -> Bundle {
    Bundle::new(7, "test", vec![])
        .with_target(target)
        .with_backend("hermit")
}

fn go(
    engine: &Engine,
    bundle: &Bundle,
    req: ConfirmRequest,
) -> Result<Vec<CheckResult>, EngineError> {
    engine.confirm(bundle, &req, &mut |_| true)
}

#[test]
fn the_checks_of_the_backend_come_back_and_it_is_told_the_target_and_control() {
    let (engine, recording) = engine(true);
    let req = ConfirmRequest {
        control: Some("fixed".into()),
        reruns: 5,
        ..ConfirmRequest::default()
    };
    let checks = go(&engine, &bundle("svc"), req).unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(
        *recording.asked.lock().unwrap(),
        [("svc".to_string(), Some("fixed".to_string()), 5)]
    );
}

#[test]
fn an_unknown_target_or_control_is_refused_before_anything_runs() {
    let (engine, recording) = engine(true);
    let err = go(&engine, &bundle("nope"), ConfirmRequest::default()).unwrap_err();
    assert!(matches!(err, EngineError::UnknownTarget(t) if t == "nope"));
    let req = ConfirmRequest {
        control: Some("missing".into()),
        ..ConfirmRequest::default()
    };
    let err = go(&engine, &bundle("svc"), req).unwrap_err();
    assert!(matches!(err, EngineError::UnknownTarget(t) if t == "missing"));
    assert!(recording.asked.lock().unwrap().is_empty());
}

#[test]
fn a_request_over_the_limits_is_refused_before_anything_runs() {
    let (engine, recording) = engine(true);
    let req = ConfirmRequest {
        reruns: 11,
        ..ConfirmRequest::default()
    };
    let err = go(&engine, &bundle("svc"), req).unwrap_err();
    assert!(matches!(err, EngineError::InvalidRequest(_)), "{err}");
    assert!(recording.asked.lock().unwrap().is_empty());
}

#[test]
fn a_bundle_from_another_backend_is_refused() {
    let (engine, _) = engine(true);
    let wrong = bundle("svc").with_backend("sim");
    let err = go(&engine, &wrong, ConfirmRequest::default()).unwrap_err();
    assert!(matches!(err, EngineError::InvalidBundle(_)), "{err}");
}

#[test]
fn a_backend_that_cannot_confirm_says_so() {
    let (engine, _) = engine(false);
    let err = go(&engine, &bundle("svc"), ConfirmRequest::default()).unwrap_err();
    assert!(matches!(err, EngineError::BackendUnavailable(_)), "{err}");
}

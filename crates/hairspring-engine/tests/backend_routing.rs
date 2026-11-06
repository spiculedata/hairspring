//! The engine picks a backend by the id the target names, never by "the first that can".
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::backend_info::{BackendLimits, BackendVersion, Capabilities};
use hairspring_engine::config::{Budget, Config, ConfigError, TargetKind, TargetSpec};
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_engine::report::{ReplayOutcome, RunOutcome};
use hairspring_engine::EngineError;
use std::sync::Arc;

const PROCESS: &str = "[target.p]\nkind = \"process\"\ncommand = [\"/bin/true\"]\n";

/// A process backend that answers every run with an error naming itself.
struct Stub {
    id: &'static str,
}

impl Backend for Stub {
    fn id(&self) -> &'static str {
        self.id
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
        Ok(RunOutcome::error(format!("ran on {}", self.id)))
    }
    fn replay(&self, _: &TargetSpec, _: &Bundle) -> Result<ReplayOutcome, EngineError> {
        unreachable!("not used")
    }
}

fn engine(config: &str, backends: &[&'static str]) -> Engine {
    let mut engine = Engine::new(
        Arc::new(Registry::new()),
        Config::parse(config).expect("config parses"),
    );
    for id in backends {
        engine = engine.with_backend(Arc::new(Stub { id }));
    }
    engine
}

fn ran_on(engine: &Engine) -> Result<String, EngineError> {
    let outcome = engine.run(&RunRequest::new("p", 1), &mut |_| true)?;
    Ok(outcome.error.unwrap_or_default())
}

fn spec(config: &str) -> TargetSpec {
    Config::parse(config).expect("config parses").targets["p"].clone()
}

#[test]
fn a_process_target_without_a_backend_key_means_hermit() {
    let s = spec(PROCESS);
    assert_eq!(s.kind.backend_id(), "hermit");
    assert!(matches!(s.kind, TargetKind::Process { backend: None, .. }));
}

#[test]
fn naming_hermit_resolves_the_same_as_naming_nothing() {
    let named = spec(&format!("{PROCESS}backend = \"hermit\"\n"));
    assert_eq!(named.kind.backend_id(), spec(PROCESS).kind.backend_id());
}

#[test]
fn sim_and_wasm_targets_resolve_to_their_own_backends() {
    let sim = Config::parse("[target.p]\nkind = \"sim\"\nsim = \"x\"\n").unwrap();
    let wasm = Config::parse("[target.p]\nkind = \"wasm\"\nmodule = \"m.wasm\"\n").unwrap();
    assert_eq!(sim.targets["p"].kind.backend_id(), "sim");
    assert_eq!(wasm.targets["p"].kind.backend_id(), "wasm");
}

#[test]
fn an_unknown_backend_is_a_configuration_error_that_does_not_repeat_the_value() {
    let marker = "zz-secret-marker";
    let err = Config::parse(&format!("{PROCESS}backend = \"{marker}\"\n")).unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, ConfigError::Invalid(_)), "{text}");
    assert!(text.contains("unknown `backend`"), "{text}");
    assert!(text.contains("hermit"), "{text}");
    assert!(!text.contains(marker), "{text}");
}

#[test]
fn a_backend_that_cannot_run_process_targets_is_refused_in_the_config() {
    for id in ["sim", "wasm", "qemu", "", "HERMIT", "hermit "] {
        let err = Config::parse(&format!("{PROCESS}backend = \"{id}\"\n")).unwrap_err();
        assert!(err.to_string().contains("unknown `backend`"), "{id}: {err}");
    }
}

#[test]
fn the_backend_key_is_refused_on_other_target_kinds() {
    for body in [
        "kind = \"sim\"\nsim = \"x\"",
        "kind = \"wasm\"\nmodule = \"m\"",
    ] {
        let err =
            Config::parse(&format!("[target.p]\n{body}\nbackend = \"hermit\"\n")).unwrap_err();
        assert!(
            err.to_string()
                .contains("only valid for kind = \"process\""),
            "{err}"
        );
    }
}

#[test]
fn a_target_runs_on_the_backend_it_names_not_the_first_that_supports_its_kind() {
    // Another process backend registered first must not shadow Hermit.
    let e = engine(PROCESS, &["other", "hermit"]);
    assert_eq!(ran_on(&e).unwrap(), "ran on hermit");
}

#[test]
fn a_target_is_never_moved_to_another_backend_when_its_own_is_missing() {
    let e = engine(PROCESS, &["other"]);
    let err = ran_on(&e).unwrap_err();
    assert!(matches!(err, EngineError::BackendChoice(_)), "{err}");
    assert_eq!(
        err.to_string(),
        "backend choice refused: backend `hermit` is not available in this build"
    );
}

#[test]
fn two_backends_with_one_id_are_an_error_not_a_silent_first_wins() {
    let e = engine(PROCESS, &["hermit", "hermit"]);
    let err = ran_on(&e).unwrap_err();
    assert!(matches!(err, EngineError::BackendChoice(_)), "{err}");
    assert!(err.to_string().contains("more than one backend"), "{err}");
}

#[test]
fn a_backend_that_does_not_support_the_kind_is_refused() {
    // A backend registered under the id `sim` that only handles process targets.
    let mut config = Config::default();
    let mut s = spec(PROCESS);
    s.kind = match s.kind {
        TargetKind::Process {
            command,
            env,
            cwd,
            hermit_args,
            trial_timeout_secs,
            scheduler,
            hang_is_violation,
            parallelism,
            pass_env,
            ..
        } => TargetKind::Process {
            command,
            env,
            cwd,
            hermit_args,
            trial_timeout_secs,
            scheduler,
            hang_is_violation,
            parallelism,
            pass_env,
            backend: Some("sim".to_string()),
        },
        other => other,
    };
    config.targets.insert("p".into(), s);
    let e = Engine::new(Arc::new(Registry::new()), config);
    let err = ran_on(&e).unwrap_err();
    assert!(
        err.to_string().contains("cannot run process targets"),
        "{err}"
    );
}

#[test]
fn a_hand_built_unknown_id_is_not_repeated_in_the_error() {
    let mut s = spec(PROCESS);
    if let TargetKind::Process { backend, .. } = &mut s.kind {
        *backend = Some("zz-secret-marker\nline two".to_string());
    }
    let mut config = Config::default();
    config.targets.insert("p".into(), s);
    let err = ran_on(&Engine::new(Arc::new(Registry::new()), config)).unwrap_err();
    assert!(!err.to_string().contains("zz-secret-marker"), "{err}");
    assert!(!err.to_string().contains('\n'), "{err}");
    assert_eq!(
        err.to_string(),
        "backend choice refused: the named backend is not available in this build"
    );
}

struct Described;

impl Backend for Described {
    fn id(&self) -> &'static str {
        "hermit"
    }
    fn display_name(&self) -> &'static str {
        "Described"
    }
    fn version(&self) -> BackendVersion {
        BackendVersion {
            crate_version: "9.9.9".into(),
            binary_version: Some("tool 1".into()),
            binary_sha256: None,
        }
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            replay: true,
            ..Capabilities::default()
        }
    }
    fn limits(&self) -> BackendLimits {
        BackendLimits {
            max_parallelism: Some(3),
            max_trial_seconds: Some(5),
        }
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
}

#[test]
fn a_backend_states_its_identity_capabilities_and_limits() {
    let e = Engine::new(Arc::new(Registry::new()), Config::parse(PROCESS).unwrap())
        .with_backend(Arc::new(Described));
    let info = e
        .backend_info(&e.config().targets["p"])
        .expect("hermit is registered");
    assert_eq!(info.id, "hermit");
    assert_eq!(info.display_name, "Described");
    assert_eq!(info.version.crate_version, "9.9.9");
    assert_eq!(info.version.binary_version.as_deref(), Some("tool 1"));
    assert!(info.capabilities.replay);
    assert!(!info.capabilities.confirm);
    assert_eq!(info.limits.max_parallelism, Some(3));
    assert_eq!(info.limits.max_trial_seconds, Some(5));
}

#[test]
fn a_backend_that_says_nothing_claims_nothing() {
    let info = Stub { id: "hermit" }.info();
    assert_eq!(info.display_name, "hermit");
    assert_eq!(info.capabilities, Capabilities::default());
    assert_eq!(info.limits, BackendLimits::default());
    assert!(!info.version.crate_version.is_empty());
}

#[test]
fn a_target_with_no_registered_backend_has_no_info() {
    let e = engine(PROCESS, &[]);
    assert!(e.backend_info(&e.config().targets["p"]).is_none());
}

#[test]
fn the_built_in_sim_backend_declares_what_it_has() {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let config = Config::from_registry(&registry);
    let e = Engine::new(registry, config);
    let spec = e.config().targets.values().next().unwrap().clone();
    let info = e.backend_info(&spec).expect("sim is always registered");
    assert_eq!(info.id, "sim");
    assert!(info.capabilities.replay && info.capabilities.shrink);
    assert!(!info.capabilities.confirm && !info.capabilities.strict_verify);
    assert!(!info.capabilities.snapshot_branch && !info.capabilities.fault_injection);
    assert!(!info.capabilities.multi_process);
}

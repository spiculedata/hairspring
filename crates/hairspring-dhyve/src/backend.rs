//! The `dhyve` backend as the engine sees it: an identity, an empty capability row and a machine check. It runs
//! nothing yet; `run` and `replay` say so.
use crate::host::{HostConfig, Timings};
use crate::logs::Bounds;
use crate::session::Session;
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Backend, Progress, RunRequest};
use hairspring_engine::backend_info::{BackendLimits, BackendVersion, Capabilities};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::probe::ProbeReport;
use hairspring_engine::report::{ReplayOutcome, RunOutcome};
use hairspring_engine::EngineError;

/// The id the config, bundles and listings name this backend by.
pub const BACKEND_ID: &str = "dhyve";
/// What this backend says about its own determinism. Runs may happen; nothing about them is a claim.
pub const DETERMINISM_CLASS: &str = "unproven";
/// One virtual machine at a time is the only shape this backend is designed for.
const MAX_PARALLELISM: u32 = 1;

/// The settings a backend runs with. `new()` reads them from the environment when a run starts.
#[derive(Debug, Clone)]
struct Settings {
    host: HostConfig,
    timings: Timings,
    bounds: Bounds,
}

#[derive(Debug, Default, Clone)]
pub struct DhyveBackend {
    settings: Option<Settings>,
}

impl DhyveBackend {
    pub fn new() -> DhyveBackend {
        DhyveBackend::default()
    }

    /// A backend with its settings given, for a host session and for tests against the fake.
    pub fn with_settings(host: HostConfig, timings: Timings, bounds: Bounds) -> DhyveBackend {
        DhyveBackend {
            settings: Some(Settings {
                host,
                timings,
                bounds,
            }),
        }
    }

    fn session(&self) -> Result<Session, EngineError> {
        let settings = match &self.settings {
            Some(s) => s.clone(),
            None => Settings {
                host: HostConfig::from_env().map_err(|e| {
                    EngineError::BackendUnavailable(format!(
                        "the {BACKEND_ID} backend cannot run: {e}"
                    ))
                })?,
                timings: Timings::standard(),
                bounds: Bounds::interim(),
            },
        };
        Ok(Session::new(
            settings.host,
            settings.timings,
            settings.bounds,
        ))
    }
}

fn not_built_yet(what: &str) -> EngineError {
    EngineError::BackendUnavailable(format!(
        "`{what}` is not available on the {BACKEND_ID} backend yet: it can check this machine and nothing more"
    ))
}

impl Backend for DhyveBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn display_name(&self) -> &'static str {
        "dhyve (experimental)"
    }

    fn version(&self) -> BackendVersion {
        BackendVersion::new(env!("CARGO_PKG_VERSION"))
    }

    /// Every flag is `false`: none is set before a stage of its own and its tests.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn limits(&self) -> BackendLimits {
        BackendLimits {
            max_parallelism: Some(MAX_PARALLELISM),
            max_trial_seconds: Some(crate::run::MAX_TRIAL_SECS),
        }
    }

    fn supports(&self, kind: &TargetKind) -> bool {
        matches!(kind, TargetKind::Process { .. }) && kind.backend_id() == BACKEND_ID
    }

    fn run(
        &self,
        spec: &TargetSpec,
        req: &RunRequest,
        budget: Budget,
        progress: &mut dyn FnMut(Progress) -> bool,
    ) -> Result<RunOutcome, EngineError> {
        if req.mode.is_sweep() {
            return Err(not_built_yet("sweep"));
        }
        let session = self.session()?;
        crate::run::run_trials(&session, spec, req, budget, progress)
    }

    fn replay(&self, _spec: &TargetSpec, _bundle: &Bundle) -> Result<ReplayOutcome, EngineError> {
        Err(not_built_yet("replay"))
    }

    fn probe(&self) -> ProbeReport {
        crate::assess(&crate::gather())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_engine::scheduler::SchedulerSpec;
    use std::collections::BTreeMap;

    fn process(backend: Option<&str>) -> TargetKind {
        TargetKind::Process {
            command: vec!["/bin/true".into()],
            env: BTreeMap::new(),
            cwd: None,
            hermit_args: vec![],
            trial_timeout_secs: None,
            scheduler: SchedulerSpec::default(),
            hang_is_violation: false,
            parallelism: 1,
            pass_env: false,
            backend: backend.map(String::from),
        }
    }

    #[test]
    fn it_names_itself_and_claims_no_capability() {
        let b = DhyveBackend::new();
        let info = b.info();
        assert_eq!(info.id, "dhyve");
        assert_eq!(info.display_name, "dhyve (experimental)");
        assert_eq!(info.capabilities, Capabilities::default());
        assert_eq!(info.limits.max_parallelism, Some(1));
        assert_eq!(info.limits.max_trial_seconds, Some(100));
        assert_eq!(info.version.binary_sha256, None);
        assert_eq!(DETERMINISM_CLASS, "unproven");
    }

    #[test]
    fn it_supports_only_process_targets_that_name_it() {
        let b = DhyveBackend::new();
        assert!(b.supports(&process(Some("dhyve"))));
        assert!(!b.supports(&process(None)));
        assert!(!b.supports(&process(Some("hermit"))));
    }

    #[test]
    fn replay_and_sweep_say_they_are_not_available_yet() {
        let b = DhyveBackend::new();
        let err = b.replay(&spec(), &Bundle::new(1, "b", vec![])).unwrap_err();
        assert!(matches!(err, EngineError::BackendUnavailable(m) if m.contains("`replay`")));
        let sweep = RunRequest::new("t", 1).with_mode(hairspring_engine::mode::RunMode::Sweep);
        let err = b
            .run(&spec(), &sweep, Budget::default(), &mut |_| true)
            .unwrap_err();
        assert!(matches!(err, EngineError::BackendUnavailable(m) if m.contains("`sweep`")));
    }

    #[test]
    fn a_run_with_no_settings_says_which_setting_is_missing() {
        if std::env::var_os(crate::host::ENV_BHYVE).is_some() {
            return;
        }
        let err = DhyveBackend::new()
            .run(
                &spec(),
                &RunRequest::new("t", 1),
                Budget::default(),
                &mut |_| true,
            )
            .unwrap_err();
        let EngineError::BackendUnavailable(m) = err else {
            panic!("not unavailable");
        };
        assert!(m.contains(crate::host::ENV_BHYVE), "{m}");
        assert!(!m.contains("--"), "{m}");
    }

    #[test]
    fn every_capability_flag_is_false_including_replay() {
        let c = DhyveBackend::new().capabilities();
        assert_eq!(c, Capabilities::default());
        assert!(!c.replay && !c.confirm && !c.shrink && !c.strict_verify);
        assert!(!c.snapshot_branch && !c.fault_injection && !c.multi_process);
    }

    #[test]
    fn the_probe_answers_without_panicking_on_this_machine() {
        let report = DhyveBackend::new().probe();
        assert_eq!(report.subject, "dhyve backend (experimental)");
        if std::env::consts::OS != "freebsd" {
            assert!(!report.is_ready());
            assert_eq!(report.rows.len(), 1);
        }
    }

    fn spec() -> TargetSpec {
        TargetSpec {
            name: "t".into(),
            description: String::new(),
            kind: process(Some("dhyve")),
            budget: Budget::default(),
        }
    }
}

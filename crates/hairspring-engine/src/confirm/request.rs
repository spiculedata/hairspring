//! What a caller may ask of the pipeline, and the limits that keep it bounded.
use crate::EngineError;
use std::collections::BTreeMap;

/// Same-seed reruns when the caller does not say.
pub const DEFAULT_RERUNS: u32 = 3;
/// Most same-seed reruns one confirmation may ask for.
pub const MAX_RERUNS: u32 = 10;
/// Wall-clock budget for a whole confirmation when the caller does not say, and the most it may ask for.
pub const DEFAULT_WALL_SECS: u64 = 120;
pub const MAX_WALL_SECS: u64 = 600;
/// Longest name of a control target (the same bound as every target name).
pub const MAX_CONTROL_NAME_CHARS: usize = 64;

/// Hermit options that turn on logging. Logging changed a guest's path, so a confirmation must not carry any.
const LOG_FLAGS: [&str; 2] = ["--log", "--verbose"];
const LOG_FLAG_PREFIX: &str = "--log=";
const LOG_ENV: &str = "HERMIT_LOG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmRequest {
    /// Same-seed reruns (check 1), `1..=MAX_RERUNS`.
    pub reruns: u32,
    /// Name of a target that runs the fixed build, for the control check (check 3).
    pub control: Option<String>,
    /// Also run Hermit's strict determinism verify (check 4). It costs two runs.
    pub strict_verify: bool,
    /// Replay even when the host's loader differs from the recording; the differences are still recorded.
    pub allow_environment_change: bool,
    /// Wall-clock budget for the whole confirmation, `1..=MAX_WALL_SECS`. Checks that do not fit are skipped.
    pub wall_secs: u64,
}

impl Default for ConfirmRequest {
    fn default() -> Self {
        ConfirmRequest {
            reruns: DEFAULT_RERUNS,
            control: None,
            strict_verify: false,
            allow_environment_change: false,
            wall_secs: DEFAULT_WALL_SECS,
        }
    }
}

impl ConfirmRequest {
    /// The request, or why it is refused. Values are refused rather than clamped: a caller who asked for 50 reruns
    /// should hear that the answer is not 50.
    pub fn validated(self) -> Result<ConfirmRequest, EngineError> {
        if !(1..=MAX_RERUNS).contains(&self.reruns) {
            return Err(invalid(format!("`reruns` must be 1-{MAX_RERUNS}")));
        }
        if !(1..=MAX_WALL_SECS).contains(&self.wall_secs) {
            return Err(invalid(format!("`wall_secs` must be 1-{MAX_WALL_SECS}")));
        }
        if let Some(name) = &self.control {
            if name.is_empty() || name.chars().count() > MAX_CONTROL_NAME_CHARS {
                return Err(invalid(format!(
                    "the control target name must be 1-{MAX_CONTROL_NAME_CHARS} characters"
                )));
            }
        }
        Ok(self)
    }
}

fn invalid(message: String) -> EngineError {
    EngineError::InvalidRequest(message)
}

/// The first logging option among Hermit's arguments or the environment it is given, described for a caller, or `None`.
pub fn forbidden_logging(args: &[String], env: &BTreeMap<String, String>) -> Option<String> {
    let flag = args.iter().find(|a| is_log_flag(a));
    if let Some(flag) = flag {
        let name = flag.split('=').next().unwrap_or(flag);
        return Some(format!(
            "`{name}` turns on Hermit logging, which changes a guest's path; a confirmation cannot carry it"
        ));
    }
    env.contains_key(LOG_ENV).then(|| {
        format!("`{LOG_ENV}` turns on Hermit logging, which changes a guest's path; a confirmation cannot carry it")
    })
}

fn is_log_flag(arg: &str) -> bool {
    LOG_FLAGS.contains(&arg)
        || arg.starts_with(LOG_FLAG_PREFIX)
        || (arg.starts_with("-v") && arg[1..].bytes().all(|b| b == b'v'))
}

//! Hairspring engine: ties the core algorithms to named targets, configuration,
//! pluggable backends and reports. Everything above this crate (CLI, server,
//! MCP) talks to [`engine::Engine`].
pub mod backend;
pub mod config;
pub mod confirm;
pub mod demos;
pub mod engine;
pub mod erased;
pub mod format;
mod format_props;
mod format_rate;
mod format_sweep;
pub mod mode;
pub mod report;
pub mod scheduler;
pub mod sim_backend;
mod sim_sweep;
pub mod sweep;
pub mod trial_log;

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    UnknownTarget(String),
    BackendUnavailable(String),
    InvalidBundle(String),
    Replay(String),
    Backend(String),
    /// Replay was refused because the host differs from the one the bundle was recorded on in a way that makes the
    /// recorded schedule unreliable. The caller can retry with `ReplayOptions::allow_environment_change`.
    EnvironmentMismatch(String),
    /// The caller asked for something the engine will not do (for example more reruns than the limit). Safe to show.
    InvalidRequest(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::UnknownTarget(t) => write!(f, "unknown target `{t}`"),
            EngineError::BackendUnavailable(k) => {
                write!(f, "no backend available for target kind `{k}`")
            }
            EngineError::InvalidBundle(m) => write!(f, "invalid bundle: {m}"),
            EngineError::Replay(m) => write!(f, "replay failed: {m}"),
            EngineError::Backend(m) => write!(f, "backend error: {m}"),
            EngineError::EnvironmentMismatch(m) => write!(f, "environment mismatch: {m}"),
            EngineError::InvalidRequest(m) => write!(f, "invalid request: {m}"),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<hairspring_core::bundle::BundleError> for EngineError {
    fn from(e: hairspring_core::bundle::BundleError) -> Self {
        EngineError::InvalidBundle(e.to_string())
    }
}

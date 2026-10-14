//! `anthill.toml` parsing and validation.
use crate::erased::Registry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

const MAX_STEPS_CAP: u64 = 50_000_000;
const MAX_TRIALS_CAP: u64 = 10_000;
const MAX_WALL_SECS_CAP: u64 = 600;
const MAX_NAME_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Budget {
    pub max_steps: u64,
    pub max_trials: u64,
    pub wall_secs: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_steps: 200_000,
            max_trials: 200,
            wall_secs: 60,
        }
    }
}

impl Budget {
    /// Cap runaway values (for example from an API caller) at sane maxima.
    pub fn clamp(self) -> Budget {
        Budget {
            max_steps: self.max_steps.min(MAX_STEPS_CAP),
            max_trials: self.max_trials.min(MAX_TRIALS_CAP),
            wall_secs: self.wall_secs.min(MAX_WALL_SECS_CAP),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TargetKind {
    Sim {
        sim: String,
    },
    Process {
        command: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
        hermit_args: Vec<String>,
    },
    Wasm {
        module: String,
    },
}

impl TargetKind {
    pub fn name(&self) -> &'static str {
        match self {
            TargetKind::Sim { .. } => "sim",
            TargetKind::Process { .. } => "process",
            TargetKind::Wasm { .. } => "wasm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TargetSpec {
    pub name: String,
    pub description: String,
    #[serde(flatten)]
    pub kind: TargetKind,
    pub budget: Budget,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Config {
    pub targets: BTreeMap<String, TargetSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Parse(String),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Parse(m) => write!(f, "config parse error: {m}"),
            ConfigError::Invalid(m) => write!(f, "invalid config: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawKind {
    Sim,
    Process,
    Wasm,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTarget {
    kind: RawKind,
    description: Option<String>,
    sim: Option<String>,
    command: Option<Vec<String>>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    cwd: Option<String>,
    #[serde(default)]
    hermit_args: Vec<String>,
    module: Option<String>,
    #[serde(default)]
    budget: Budget,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    target: BTreeMap<String, RawTarget>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn convert(name: String, raw: RawTarget) -> Result<TargetSpec, ConfigError> {
    if !valid_name(&name) {
        return Err(ConfigError::Invalid(format!(
            "target name `{name}` must be 1-{MAX_NAME_LEN} chars of [A-Za-z0-9_.-]"
        )));
    }
    let missing = |what: &str| ConfigError::Invalid(format!("target `{name}` requires `{what}`"));
    let kind = match raw.kind {
        RawKind::Sim => TargetKind::Sim {
            sim: raw
                .sim
                .filter(|s| !s.is_empty())
                .ok_or_else(|| missing("sim"))?,
        },
        RawKind::Process => {
            let command = raw
                .command
                .filter(|c| !c.is_empty() && !c[0].is_empty())
                .ok_or_else(|| missing("command"))?;
            TargetKind::Process {
                command,
                env: raw.env,
                cwd: raw.cwd,
                hermit_args: raw.hermit_args,
            }
        }
        RawKind::Wasm => TargetKind::Wasm {
            module: raw
                .module
                .filter(|s| !s.is_empty())
                .ok_or_else(|| missing("module"))?,
        },
    };
    Ok(TargetSpec {
        description: raw.description.unwrap_or_default(),
        name,
        kind,
        budget: raw.budget,
    })
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let raw: RawConfig = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let targets = raw
            .target
            .into_iter()
            .map(|(name, t)| convert(name.clone(), t).map(|spec| (name, spec)))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(Config { targets })
    }

    /// One sim target per registry entry, named after it.
    pub fn from_registry(registry: &Registry) -> Config {
        Config::default().with_registry_defaults(registry)
    }

    /// Adds sim targets for registry entries that this config does not already define.
    pub fn with_registry_defaults(self, registry: &Registry) -> Config {
        let mut targets = self.targets;
        for name in registry.names() {
            targets.entry(name.clone()).or_insert_with(|| TargetSpec {
                description: registry.description(&name).unwrap_or_default(),
                kind: TargetKind::Sim { sim: name.clone() },
                budget: Budget::default(),
                name,
            });
        }
        Config { targets }
    }
}

//! How a run treats a confirmed violation.
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Whether a run stops at its first confirmed violation or runs its whole trial budget.
///
/// `Search` is the default and the behaviour every run had before sweeps existed: it ends at the first confirmed
/// failure, so it can find a bug but cannot say how often one happens. `Sweep` runs every trial of the budget,
/// continues after a violation and stores every trial, so a rate can be stated for the trials it ran.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    #[default]
    Search,
    Sweep,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::Search => "search",
            RunMode::Sweep => "sweep",
        }
    }

    pub fn is_sweep(self) -> bool {
        self == RunMode::Sweep
    }
}

impl fmt::Display for RunMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The text of a mode that is neither `search` nor `sweep`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownMode(pub String);

impl fmt::Display for UnknownMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown run mode `{}`; use `search` or `sweep`", self.0)
    }
}

impl std::error::Error for UnknownMode {}

impl FromStr for RunMode {
    type Err = UnknownMode;

    fn from_str(s: &str) -> Result<Self, UnknownMode> {
        match s {
            "search" => Ok(RunMode::Search),
            "sweep" => Ok(RunMode::Sweep),
            other => Err(UnknownMode(other.chars().take(32).collect())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_the_default_mode() {
        assert_eq!(RunMode::default(), RunMode::Search);
        assert!(!RunMode::default().is_sweep());
    }

    #[test]
    fn a_mode_round_trips_through_its_name_and_json() {
        for mode in [RunMode::Search, RunMode::Sweep] {
            assert_eq!(mode.as_str().parse::<RunMode>(), Ok(mode));
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
            assert_eq!(serde_json::from_str::<RunMode>(&json).unwrap(), mode);
        }
    }

    #[test]
    fn an_unknown_mode_is_refused_and_its_text_is_bounded() {
        let err = "x".repeat(500).parse::<RunMode>().unwrap_err();
        assert_eq!(err.0.len(), 32);
        assert!("Sweep".parse::<RunMode>().is_err());
        assert!(serde_json::from_str::<RunMode>("\"fast\"").is_err());
    }
}

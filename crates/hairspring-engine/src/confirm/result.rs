//! One recorded check: what ran, how it ended, and the facts needed to run it again by hand.
use hairspring_core::environment::is_unsafe_display_char;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest command line kept for a check, in characters.
pub const MAX_COMMAND_CHARS: usize = 2000;
pub const MAX_DETAIL_ENTRIES: usize = 16;
pub const MAX_DETAIL_KEY_CHARS: usize = 32;
pub const MAX_DETAIL_VALUE_CHARS: usize = 300;
/// Hashes and fingerprints a check names (the original's and each rerun's).
pub const MAX_ARTEFACTS: usize = 16;
pub const MAX_ARTEFACT_CHARS: usize = 64;
/// More than the environment identity has fields, so nothing real is cut.
pub const MAX_ENVIRONMENT_ENTRIES: usize = 32;
pub const MAX_ENVIRONMENT_VALUE_CHARS: usize = 256;

macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(self) -> &'static str { match self { $($name::$variant => $text),+ } }
            pub fn parse(text: &str) -> Option<$name> { match text { $($text => Some($name::$variant)),+, _ => None } }
        }
    };
}

wire_enum!(CheckKind {
    SameSeedRerun => "same_seed_rerun",
    BundleReplay => "bundle_replay",
    FixedControl => "fixed_control",
    StrictVerify => "strict_verify",
});

wire_enum!(CheckStatus {
    Pass => "pass",
    Fail => "fail",
    Inconclusive => "inconclusive",
    Skipped => "skipped",
});

/// How the individual runs of a check ended. A rerun that could not be judged is counted apart from the other two.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCounts {
    pub passed: u32,
    pub failed: u32,
    pub inconclusive: u32,
}

impl CheckCounts {
    pub fn total(&self) -> u32 {
        self.passed
            .saturating_add(self.failed)
            .saturating_add(self.inconclusive)
    }
}

/// What one check found. Built with the `with_` methods, which return a new value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub kind: CheckKind,
    pub status: CheckStatus,
    #[serde(default)]
    pub counts: CheckCounts,
    /// The exact Hermit command, enough to run the check by hand. Empty for a check that ran no command.
    #[serde(default)]
    pub command: String,
    /// Short labelled facts (why it ended as it did, caveats). Bounded.
    #[serde(default)]
    pub detail: BTreeMap<String, String>,
    /// Hashes and fingerprints the check saw: the recorded ones first, then those of the runs it made.
    #[serde(default)]
    pub artefacts: Vec<String>,
    /// The execution environment the check ran in (the named fields of the environment identity).
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub started_at_ms: i64,
    #[serde(default)]
    pub finished_at_ms: i64,
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One line, no control or bidirectional characters, at most `max` characters.
fn clean(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if is_unsafe_display_char(c) { ' ' } else { c })
        .take(max)
        .collect()
}

impl CheckResult {
    pub fn new(kind: CheckKind, status: CheckStatus) -> CheckResult {
        let now = now_ms();
        CheckResult {
            kind,
            status,
            counts: CheckCounts::default(),
            command: String::new(),
            detail: BTreeMap::new(),
            artefacts: Vec::new(),
            environment: BTreeMap::new(),
            started_at_ms: now,
            finished_at_ms: now,
        }
    }

    #[must_use]
    pub fn with_status(self, status: CheckStatus) -> CheckResult {
        CheckResult { status, ..self }
    }

    #[must_use]
    pub fn with_counts(self, passed: u32, failed: u32, inconclusive: u32) -> CheckResult {
        CheckResult {
            counts: CheckCounts {
                passed,
                failed,
                inconclusive,
            },
            ..self
        }
    }

    #[must_use]
    pub fn with_command(self, command: impl Into<String>) -> CheckResult {
        CheckResult {
            command: command.into(),
            ..self
        }
    }

    #[must_use]
    pub fn with_detail(self, key: impl Into<String>, value: impl Into<String>) -> CheckResult {
        let mut detail = self.detail;
        detail.insert(key.into(), value.into());
        CheckResult { detail, ..self }
    }

    /// The detail, replacing any recorded so far.
    #[must_use]
    pub fn with_detail_fields(
        self,
        fields: impl IntoIterator<Item = (String, String)>,
    ) -> CheckResult {
        CheckResult {
            detail: fields.into_iter().collect(),
            ..self
        }
    }

    #[must_use]
    pub fn with_artefact(self, hash: impl Into<String>) -> CheckResult {
        let mut artefacts = self.artefacts;
        artefacts.push(hash.into());
        CheckResult { artefacts, ..self }
    }

    #[must_use]
    pub fn with_environment(
        self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> CheckResult {
        let mut environment = self.environment;
        environment.insert(name.into(), value.into());
        CheckResult {
            environment,
            ..self
        }
    }

    /// The environment the check ran in, replacing any recorded so far.
    #[must_use]
    pub fn with_environment_fields(
        self,
        fields: impl IntoIterator<Item = (String, String)>,
    ) -> CheckResult {
        CheckResult {
            environment: fields.into_iter().collect(),
            ..self
        }
    }

    #[must_use]
    pub fn with_times(self, started_at_ms: i64, finished_at_ms: i64) -> CheckResult {
        CheckResult {
            started_at_ms,
            finished_at_ms,
            ..self
        }
    }

    /// The same result cut to the limits: text on one line without control characters, at most the allowed number of
    /// entries, each no longer than allowed. Nothing is rejected: whatever a check holds, what is stored fits.
    #[must_use]
    pub fn bounded(self) -> CheckResult {
        let detail = self
            .detail
            .iter()
            .take(MAX_DETAIL_ENTRIES)
            .map(|(k, v)| {
                (
                    clean(k, MAX_DETAIL_KEY_CHARS),
                    clean(v, MAX_DETAIL_VALUE_CHARS),
                )
            })
            .collect();
        let environment = self
            .environment
            .iter()
            .take(MAX_ENVIRONMENT_ENTRIES)
            .map(|(k, v)| {
                (
                    clean(k, MAX_DETAIL_KEY_CHARS),
                    clean(v, MAX_ENVIRONMENT_VALUE_CHARS),
                )
            })
            .collect();
        CheckResult {
            command: clean(&self.command, MAX_COMMAND_CHARS),
            detail,
            artefacts: self
                .artefacts
                .iter()
                .take(MAX_ARTEFACTS)
                .map(|a| clean(a, MAX_ARTEFACT_CHARS))
                .collect(),
            environment,
            ..self
        }
    }
}

//! The execution environment a bundle was recorded in, and how a replay compares against it.
//!
//! A recorded schedule depends on more than the seed: the kernel, the loader cache, the Hermit binary and the guest
//! environment all change what a run does. A bundle therefore carries a small set of named fields (stored as bundle
//! params) and replay reports which of them differ now, so a failure to reproduce is not mistaken for nondeterminism.
//!
//! Fields hold hashes and short identifiers only. They never hold host names, user names or absolute paths.
use crate::bundle::Bundle;
use std::collections::BTreeMap;

/// The param namespaces that make up the recorded environment.
pub const NAMESPACES: [&str; 4] = ["host.", "hermit.", "env.", "target."];
pub const MAX_FIELD_NAME_CHARS: usize = 64;
pub const MAX_FIELD_VALUE_CHARS: usize = 256;
/// Characters of a hash shown in a warning; enough to tell two hashes apart without flooding the line.
const SHORT_HASH_CHARS: usize = 12;
const HASH_FIELD_SUFFIX: &str = ".sha256";

#[derive(Debug, PartialEq, Eq)]
pub enum EnvError {
    UnknownNamespace(String),
    BadName(String),
    BadValue(String),
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::UnknownNamespace(n) => {
                write!(f, "environment field `{n}` has an unknown namespace")
            }
            EnvError::BadName(n) => write!(f, "environment field name `{n}` is not valid"),
            EnvError::BadValue(n) => write!(f, "environment field `{n}` has an invalid value"),
        }
    }
}

impl std::error::Error for EnvError {}

/// An immutable set of named environment facts. Adding a field returns a new identity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentIdentity {
    fields: BTreeMap<String, String>,
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
}

fn validate(name: &str, value: &str) -> Result<(), EnvError> {
    if !NAMESPACES.iter().any(|ns| name.starts_with(ns)) {
        return Err(EnvError::UnknownNamespace(name.to_string()));
    }
    if name.chars().count() > MAX_FIELD_NAME_CHARS || !name.chars().all(is_name_char) {
        return Err(EnvError::BadName(name.to_string()));
    }
    if value.chars().count() > MAX_FIELD_VALUE_CHARS || value.chars().any(char::is_control) {
        return Err(EnvError::BadValue(name.to_string()));
    }
    Ok(())
}

impl EnvironmentIdentity {
    /// A copy with `name` set to `value`, or why the field is not acceptable.
    pub fn with_field(
        self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, EnvError> {
        let (name, value) = (name.into(), value.into());
        validate(&name, &value)?;
        let mut fields = self.fields;
        fields.insert(name, value);
        Ok(EnvironmentIdentity { fields })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The bundle with these fields added as params.
    pub fn apply_to(&self, bundle: Bundle) -> Bundle {
        self.fields
            .iter()
            .fold(bundle, |b, (k, v)| b.with_param(k.clone(), v.clone()))
    }

    /// The environment a bundle recorded: `None` when it recorded none, an error when a recorded field is malformed.
    pub fn from_bundle(bundle: &Bundle) -> Result<Option<Self>, EnvError> {
        let identity = bundle
            .params
            .iter()
            .filter(|(k, _)| NAMESPACES.iter().any(|ns| k.starts_with(ns)))
            .try_fold(EnvironmentIdentity::default(), |acc, (k, v)| {
                acc.with_field(k.clone(), v.clone())
            })?;
        Ok((!identity.is_empty()).then_some(identity))
    }
}

/// One recorded field whose value is not the same now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldChange {
    pub name: String,
    pub recorded: String,
    /// `None` when the current host could not provide the field.
    pub current: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvironmentComparison {
    /// The bundle recorded no environment (it predates the feature), so nothing can be compared.
    NotRecorded,
    Same,
    Differs(Vec<FieldChange>),
}

/// Compares what a bundle recorded with the current host. Fields only the current host knows are not differences.
pub fn compare(
    recorded: Option<&EnvironmentIdentity>,
    current: &EnvironmentIdentity,
) -> EnvironmentComparison {
    let Some(recorded) = recorded else {
        return EnvironmentComparison::NotRecorded;
    };
    let changes: Vec<FieldChange> = recorded
        .fields()
        .filter(|(name, value)| current.get(name) != Some(value))
        .map(|(name, value)| FieldChange {
            name: name.to_string(),
            recorded: value.to_string(),
            current: current.get(name).map(str::to_string),
        })
        .collect();
    if changes.is_empty() {
        EnvironmentComparison::Same
    } else {
        EnvironmentComparison::Differs(changes)
    }
}

fn shown(name: &str, value: &str) -> String {
    if name.ends_with(HASH_FIELD_SUFFIX) {
        value.chars().take(SHORT_HASH_CHARS).collect()
    } else {
        value.to_string()
    }
}

impl FieldChange {
    fn describe(&self) -> String {
        let now = self
            .current
            .as_deref()
            .map_or("unavailable".to_string(), |v| shown(&self.name, v));
        format!(
            "{} (recorded {}, now {now})",
            self.name,
            shown(&self.name, &self.recorded)
        )
    }
}

impl EnvironmentComparison {
    /// The line replay shows, or `None` when the environment is the same.
    pub fn warning(&self) -> Option<String> {
        match self {
            EnvironmentComparison::Same => None,
            EnvironmentComparison::NotRecorded => Some(
                "no environment recorded in this bundle; replay cannot tell whether the host changed"
                    .to_string(),
            ),
            EnvironmentComparison::Differs(changes) => {
                let list: Vec<String> = changes.iter().map(FieldChange::describe).collect();
                Some(format!("environment differs: {}", list.join(", ")))
            }
        }
    }

    pub fn differs(&self) -> bool {
        matches!(self, EnvironmentComparison::Differs(_))
    }
}

/// SHA-256 of a file's contents as lowercase hex, read in blocks so a large binary is not held in memory.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut block = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        hasher.update(&block[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

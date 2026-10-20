//! The execution environment a bundle was recorded in, and how a replay compares against it.
//!
//! A recorded schedule depends on more than the seed: the kernel, the loader cache, the Hermit binary and the guest
//! environment all change what a run does. A bundle therefore carries a small set of named fields (stored as bundle
//! params) and replay reports which of them differ now, so a failure to reproduce is not mistaken for nondeterminism.
//!
//! Only the fields in [`KNOWN_FIELDS`] exist. Fields hold hashes and short identifiers, never host names, user names
//! or absolute paths.
use crate::bundle::Bundle;
use std::collections::BTreeMap;

/// Every field a bundle may record. A param with any other name is not part of the environment.
pub const KNOWN_FIELDS: [&str; 15] = [
    "host.kernel_release",
    "host.cpu_model",
    "host.cpu_flags.sha256",
    "host.container",
    "host.loader.ld_so_cache.sha256",
    "host.loader.libc.sha256",
    "host.loader.ld_so.sha256",
    "host.loader.ld_so_preload.sha256",
    "hermit.path.sha256",
    "hermit.version",
    "env.base_env",
    "env.guest.names",
    "env.guest.names.sha256",
    "env.guest.allowlisted.sha256",
    "target.exe.sha256",
];
pub const MAX_FIELD_VALUE_CHARS: usize = 256;
/// Changes named in one warning; the rest are counted. A bundle is untrusted input, so the line must stay short.
pub const MAX_LISTED_CHANGES: usize = 8;
/// Characters of a hash shown in a warning; enough to tell two hashes apart without flooding the line.
const SHORT_HASH_CHARS: usize = 12;
const HASH_FIELD_SUFFIX: &str = ".sha256";

pub fn is_known_field(name: &str) -> bool {
    KNOWN_FIELDS.contains(&name)
}

/// Characters that can reorder or hide text when shown (bidirectional controls, zero-width and byte-order marks),
/// besides ordinary control characters.
pub fn is_unsafe_display_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
        )
}

#[derive(Debug, PartialEq, Eq)]
pub enum EnvError {
    UnknownField(String),
    BadValue(String),
}

impl std::fmt::Display for EnvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvError::UnknownField(n) => write!(f, "`{n}` is not an environment field"),
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

fn validate(name: &str, value: &str) -> Result<(), EnvError> {
    if !is_known_field(name) {
        return Err(EnvError::UnknownField(name.to_string()));
    }
    if value.chars().count() > MAX_FIELD_VALUE_CHARS || value.chars().any(is_unsafe_display_char) {
        return Err(EnvError::BadValue(name.to_string()));
    }
    Ok(())
}

/// What a bundle recorded: the fields that could be read, and the names of recorded fields that could not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedEnvironment {
    pub identity: Option<EnvironmentIdentity>,
    pub malformed: Vec<String>,
}

impl ParsedEnvironment {
    pub fn warning(&self) -> Option<String> {
        (!self.malformed.is_empty()).then(|| {
            format!(
                "malformed recorded environment fields, not compared: {}",
                self.malformed.join(", ")
            )
        })
    }
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

    /// The environment a bundle recorded. Each field is read on its own: a malformed one is listed and skipped, and the
    /// others are still compared.
    pub fn from_bundle(bundle: &Bundle) -> ParsedEnvironment {
        let mut identity = EnvironmentIdentity::default();
        let mut malformed = Vec::new();
        for (name, value) in bundle.params.iter().filter(|(k, _)| is_known_field(k)) {
            match identity.clone().with_field(name.clone(), value.clone()) {
                Ok(next) => identity = next,
                Err(_) => malformed.push(name.clone()),
            }
        }
        ParsedEnvironment {
            identity: (!identity.is_empty()).then_some(identity),
            malformed,
        }
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
                let mut list: Vec<String> = changes
                    .iter()
                    .take(MAX_LISTED_CHANGES)
                    .map(FieldChange::describe)
                    .collect();
                if changes.len() > MAX_LISTED_CHANGES {
                    list.push(format!("and {} more", changes.len() - MAX_LISTED_CHANGES));
                }
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

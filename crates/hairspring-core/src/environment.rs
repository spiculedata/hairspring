//! The execution environment a bundle was recorded in, and how a replay compares against it.
//!
//! A recorded schedule depends on more than the seed: the kernel, the loader cache, the Hermit binary and the guest
//! environment all change what a run does. A bundle therefore carries a small set of named fields (stored as bundle
//! params) and replay reports which of them differ now, so a failure to reproduce is not mistaken for nondeterminism.
//!
//! Only the fields in the [`registry`] exist. Fields hold hashes and short identifiers, never host names, user names
//! or absolute paths.
use crate::bundle::Bundle;
use std::collections::BTreeMap;

mod fieldset;
pub mod registry;

pub use fieldset::{Listed, MARKER as FIELD_LIST_PARAM};
pub use registry::{
    FieldSpec, Grammar, OwnerFields, Registry, Rule, ENGINE_FIELDS, HERMIT_FIELDS, REGISTRY,
};

pub const MAX_FIELD_VALUE_CHARS: usize = 256;
/// Changes named in one warning; the rest are counted. A bundle is untrusted input, so the line must stay short.
pub const MAX_LISTED_CHANGES: usize = 8;
/// Characters of a hash shown in a warning; enough to tell two hashes apart without flooding the line.
const SHORT_HASH_CHARS: usize = 12;
const HASH_FIELD_SUFFIX: &str = ".sha256";
/// Shown in place of a field name that cannot be shown safely.
const LISTED_NOT_RECORDED: &str = "listed, not recorded";
const UNREADABLE_NAME: &str = "(unreadable field name)";

pub fn is_known_field(name: &str) -> bool {
    REGISTRY.is_known(name)
}

/// Every field name the product's registry knows, whichever owner declares it.
pub fn known_field_names() -> impl Iterator<Item = &'static str> {
    REGISTRY.fields().map(|f| f.name)
}

/// Whether a bundle param belongs to the recorded environment: a field, or the list of recorded fields.
pub fn is_environment_param(name: &str) -> bool {
    is_known_field(name) || name == FIELD_LIST_PARAM
}

/// Characters that can reorder or hide text when shown (bidirectional controls, zero-width and byte-order marks) or
/// break a line (the line and paragraph separators U+2028 and U+2029), besides ordinary control characters. The web
/// UI keeps the same list in `web/src/shell/organisation.ts`.
pub fn is_unsafe_display_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
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

fn validate(registry: &Registry, name: &str, value: &str) -> Result<(), EnvError> {
    let spec = registry
        .spec(name)
        .ok_or_else(|| EnvError::UnknownField(name.to_string()))?;
    if value.len() > MAX_FIELD_VALUE_CHARS || !spec.grammar.accepts(value) {
        return Err(EnvError::BadValue(name.to_string()));
    }
    Ok(())
}

/// What a bundle recorded: the fields that could be read, the names of recorded fields that could not, and the
/// fields it lists that this build does not know.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedEnvironment {
    pub identity: Option<EnvironmentIdentity>,
    pub malformed: Vec<String>,
    /// Fields the bundle records (or lists) that no owner in the registry declares: possibly written by a newer build. They are
    /// never compared, since this build cannot read them; see [`ParsedEnvironment::unknown_must_match`].
    pub unknown: Vec<Listed>,
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

    /// Names of unknown fields the writer reads a difference in as a change (everything not marked informational).
    /// Replay cannot tell whether these changed, so it must not report a miss as a fix.
    pub fn unknown_must_match(&self) -> Vec<&str> {
        self.unknown
            .iter()
            .filter(|f| f.rule == Rule::MustMatch)
            .map(|f| f.name.as_str())
            .collect()
    }

    /// The line replay shows when the bundle records fields this build cannot compare.
    pub fn unknown_warning(&self) -> Option<String> {
        let names = self.unknown_must_match();
        if names.is_empty() {
            return None;
        }
        let mut list: Vec<String> = names
            .iter()
            .take(MAX_LISTED_CHANGES)
            .map(|n| n.to_string())
            .collect();
        if names.len() > MAX_LISTED_CHANGES {
            list.push(format!("and {} more", names.len() - MAX_LISTED_CHANGES));
        }
        Some(format!(
            "environment differs: this bundle records environment fields this build does not know ({}); it may have been written by a newer build, so upgrade Hairspring to compare them",
            list.join(", ")
        ))
    }

    /// Whether some recorded field could not be compared, so a miss cannot be told apart from a changed environment.
    pub fn is_partly_unreadable(&self) -> bool {
        !self.malformed.is_empty() || !self.unknown_must_match().is_empty()
    }
}

impl EnvironmentIdentity {
    /// A copy with `name` set to `value`, or why the field is not acceptable.
    pub fn with_field(
        self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, EnvError> {
        self.with_field_in(&REGISTRY, name, value)
    }

    /// [`Self::with_field`] against another registry.
    pub fn with_field_in(
        self,
        registry: &Registry,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, EnvError> {
        let (name, value) = (name.into(), value.into());
        validate(registry, &name, &value)?;
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

    /// The bundle with these fields added as params, and the list of their names (see [`FIELD_LIST_PARAM`]) so that a
    /// build which does not know one of them can tell.
    pub fn apply_to(&self, bundle: Bundle) -> Bundle {
        self.apply_to_in(&REGISTRY, bundle)
    }

    /// [`Self::apply_to`] against another registry.
    pub fn apply_to_in(&self, registry: &Registry, bundle: Bundle) -> Bundle {
        if self.is_empty() {
            return bundle;
        }
        let listed = fieldset::render(self.fields.keys().map(|name| {
            let rule = registry.spec(name).map_or(Rule::MustMatch, |s| s.rule);
            (name.as_str(), rule)
        }));
        self.fields
            .iter()
            .fold(bundle, |b, (k, v)| b.with_param(k.clone(), v.clone()))
            .with_param(FIELD_LIST_PARAM, listed)
    }

    /// The environment a bundle recorded. Each field is read on its own: a malformed one is listed and skipped, and the
    /// others are still compared.
    pub fn from_bundle(bundle: &Bundle) -> ParsedEnvironment {
        Self::from_bundle_in(&REGISTRY, bundle)
    }

    /// [`Self::from_bundle`] against another registry.
    pub fn from_bundle_in(registry: &Registry, bundle: &Bundle) -> ParsedEnvironment {
        let mut identity = EnvironmentIdentity::default();
        let mut malformed = Vec::new();
        for (name, value) in bundle.params.iter().filter(|(k, _)| registry.is_known(k)) {
            match identity
                .clone()
                .with_field_in(registry, name.clone(), value.clone())
            {
                Ok(next) => identity = next,
                Err(_) => malformed.push(name.clone()),
            }
        }
        let unknown = unknown_fields(registry, bundle, &mut malformed);
        ParsedEnvironment {
            identity: (!identity.is_empty()).then_some(identity),
            malformed,
            unknown,
        }
    }
}

/// The fields a bundle records that `registry` does not declare. A name counts if the bundle's own list names it, or if
/// it sits in a namespace the registry owns (a bundle with no list, from a build that forgot one, is still caught).
/// The list says whether the writer reads a difference as information only; a name found only by its namespace is
/// read as a change. Anything that cannot be read safely is reported as malformed, never guessed at.
fn unknown_fields(
    registry: &Registry,
    bundle: &Bundle,
    malformed: &mut Vec<String>,
) -> Vec<Listed> {
    let mut found: BTreeMap<String, Rule> = BTreeMap::new();
    if let Some(value) = bundle.params.get(FIELD_LIST_PARAM) {
        match fieldset::parse(value) {
            Some(listed) => {
                for entry in listed {
                    if !registry.is_known(&entry.name) {
                        found.insert(entry.name, entry.rule);
                    } else if !bundle.params.contains_key(&entry.name) {
                        malformed.push(format!("{} ({LISTED_NOT_RECORDED})", entry.name));
                    }
                }
            }
            None => malformed.push(FIELD_LIST_PARAM.to_string()),
        }
    }
    let by_namespace = bundle.params.keys().filter(|name| {
        name.as_str() != FIELD_LIST_PARAM
            && !registry.is_known(name)
            && registry.in_known_namespace(name)
    });
    let mut unreadable = false;
    for name in by_namespace {
        if fieldset::is_listable_name(name) {
            found.entry(name.clone()).or_insert(Rule::MustMatch);
        } else {
            unreadable = true;
        }
    }
    if unreadable {
        malformed.push(UNREADABLE_NAME.to_string());
    }
    found
        .into_iter()
        .map(|(name, rule)| Listed { name, rule })
        .collect()
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

/// Compares what a bundle recorded with the current host. Fields only the current host knows are not differences, and
/// neither are fields the registry declares informational.
pub fn compare(
    recorded: Option<&EnvironmentIdentity>,
    current: &EnvironmentIdentity,
) -> EnvironmentComparison {
    compare_in(&REGISTRY, recorded, current)
}

/// [`compare`] against another registry.
pub fn compare_in(
    registry: &Registry,
    recorded: Option<&EnvironmentIdentity>,
    current: &EnvironmentIdentity,
) -> EnvironmentComparison {
    let Some(recorded) = recorded else {
        return EnvironmentComparison::NotRecorded;
    };
    let defaults = registry.fields().filter_map(|f| {
        let default = f.default_when_absent?;
        (recorded.get(f.name).is_none() && current.get(f.name).is_some())
            .then_some((f.name, default))
    });
    let mut read: Vec<(&str, &str)> = recorded.fields().chain(defaults).collect();
    read.sort_unstable_by_key(|(name, _)| *name);
    let changes: Vec<FieldChange> = read
        .into_iter()
        .filter(|(name, _)| {
            registry
                .spec(name)
                .is_none_or(|spec| spec.rule == Rule::MustMatch)
        })
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

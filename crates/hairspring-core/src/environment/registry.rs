//! The environment fields a bundle may record, declared per owner.
//!
//! Each owner (the engine for facts every backend shares, then one entry per backend) lists the fields it records:
//! the name, what a value looks like, and how a difference is read. The table lives in core because bundles are
//! decoded here, below every backend crate, and must know every name an in-tree backend writes; a new backend adds
//! one table and its fields' namespace, and no comparison code names a field.
//!
//! Names are spelled `<namespace>.<field>`. A namespace belongs to whoever registers a field in it.

/// How a difference in a recorded field is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// A difference means the environment changed: a replay that misses is not evidence the bug is fixed.
    MustMatch,
    /// A difference is not a change. The value is recorded for the reader's information only.
    Informational,
}

/// What a value of a field looks like. Every field is also bounded in length by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grammar {
    /// Printable ASCII.
    Text,
    /// Lowercase hexadecimal.
    Sha256,
    /// Lowercase hexadecimal, or the word `absent` (the file does not exist).
    Sha256OrAbsent,
    /// `none`, or `hairspring-<n>` for version `n` of the list of variables the engine names.
    GuestPassed,
}

const ABSENT: &str = "absent";
const PASSED_NONE: &str = "none";
const PASSED_PREFIX: &str = "hairspring-";
const MAX_PASSED_VERSION_DIGITS: usize = 6;

impl Grammar {
    pub fn accepts(self, value: &str) -> bool {
        match self {
            Grammar::Text => value.bytes().all(|b| (0x20..=0x7E).contains(&b)),
            Grammar::Sha256 => is_hex(value),
            Grammar::Sha256OrAbsent => value == ABSENT || is_hex(value),
            Grammar::GuestPassed => {
                value == PASSED_NONE
                    || value.strip_prefix(PASSED_PREFIX).is_some_and(|n| {
                        !n.is_empty()
                            && n.len() <= MAX_PASSED_VERSION_DIGITS
                            && n.bytes().all(|b| b.is_ascii_digit())
                    })
            }
        }
    }

    /// Whether a value is shown shortened in a warning (a hash is; it is long and only its start tells two apart).
    pub fn is_hash(self) -> bool {
        matches!(self, Grammar::Sha256 | Grammar::Sha256OrAbsent)
    }
}

fn is_hex(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// One field an owner records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    pub name: &'static str,
    pub grammar: Grammar,
    pub rule: Rule,
    /// What a recording that lacks the field is read as, when the current side has it: for a field added after bundles
    /// were already being written. Every reader applies the same rule because it lives here.
    pub default_when_absent: Option<&'static str>,
}

impl FieldSpec {
    /// A field a difference in which means the environment changed.
    pub const fn must_match(name: &'static str, grammar: Grammar) -> Self {
        FieldSpec {
            name,
            grammar,
            rule: Rule::MustMatch,
            default_when_absent: None,
        }
    }

    pub const fn informational(name: &'static str, grammar: Grammar) -> Self {
        FieldSpec {
            name,
            grammar,
            rule: Rule::Informational,
            default_when_absent: None,
        }
    }

    pub const fn defaulting_to(self, value: &'static str) -> Self {
        FieldSpec {
            default_when_absent: Some(value),
            ..self
        }
    }
}

/// The fields one owner records.
#[derive(Clone, Copy, Debug)]
pub struct OwnerFields {
    pub owner: &'static str,
    pub fields: &'static [FieldSpec],
}

/// Facts about the host, the guest's inputs and the program, recorded whichever backend ran it.
pub const ENGINE_FIELDS: OwnerFields = OwnerFields {
    owner: "engine",
    fields: &[
        FieldSpec::must_match("host.kernel_release", Grammar::Text),
        FieldSpec::must_match("host.cpu_model", Grammar::Text),
        FieldSpec::must_match("host.cpu_flags.sha256", Grammar::Sha256),
        FieldSpec::must_match("host.container", Grammar::Text),
        FieldSpec::must_match("env.base_env", Grammar::Text),
        FieldSpec::must_match("env.guest.names", Grammar::Text),
        FieldSpec::must_match("env.guest.names.sha256", Grammar::Sha256),
        FieldSpec::must_match("env.guest.allowlisted.sha256", Grammar::Sha256),
        FieldSpec::must_match("env.guest.passed", Grammar::GuestPassed).defaulting_to("none"),
        FieldSpec::must_match("target.exe.sha256", Grammar::Sha256),
        FieldSpec::must_match("target.command.sha256", Grammar::Sha256),
        FieldSpec::must_match("target.cwd.sha256", Grammar::Sha256),
    ],
};

/// What the Hermit backend records about itself and the dynamic loader its guest runs under.
pub const HERMIT_FIELDS: OwnerFields = OwnerFields {
    owner: "hermit",
    fields: &[
        FieldSpec::must_match("host.loader.ld_so_cache.sha256", Grammar::Sha256),
        FieldSpec::must_match("host.loader.libc.sha256", Grammar::Sha256),
        FieldSpec::must_match("host.loader.ld_so.sha256", Grammar::Sha256),
        FieldSpec::must_match("host.loader.ld_so_preload.sha256", Grammar::Sha256OrAbsent),
        FieldSpec::must_match("hermit.path.sha256", Grammar::Sha256),
        FieldSpec::must_match("hermit.version", Grammar::Text),
    ],
};

/// A set of owners' tables. [`REGISTRY`] is the one the product uses; tests build their own.
#[derive(Clone, Copy, Debug)]
pub struct Registry {
    owners: &'static [OwnerFields],
}

pub const REGISTRY: Registry = Registry::new(&[ENGINE_FIELDS, HERMIT_FIELDS]);

impl Registry {
    pub const fn new(owners: &'static [OwnerFields]) -> Self {
        Registry { owners }
    }

    pub fn owners(&self) -> &'static [OwnerFields] {
        self.owners
    }

    pub fn fields(&self) -> impl Iterator<Item = &'static FieldSpec> {
        self.owners.iter().flat_map(|o| o.fields.iter())
    }

    pub fn spec(&self, name: &str) -> Option<&'static FieldSpec> {
        self.fields().find(|f| f.name == name)
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.spec(name).is_some()
    }

    /// Whether `name` is spelled in a namespace some owner registers fields in. A name in such a namespace that is not
    /// registered is a field this build does not know, not an ordinary replay parameter.
    pub fn in_known_namespace(&self, name: &str) -> bool {
        name.split_once('.')
            .is_some_and(|(ns, _)| self.fields().any(|f| namespace_of(f.name) == ns))
    }
}

fn namespace_of(name: &str) -> &str {
    name.split_once('.').map_or(name, |(ns, _)| ns)
}

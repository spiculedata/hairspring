//! The list of fields a bundle says it recorded.
//!
//! A reader that does not know a field ignores it, so a bundle written by a newer build could replay on an older one
//! as if its environment were unchanged. Every bundle that records an environment therefore also lists the names it
//! recorded, in the param [`MARKER`]: a build that finds a name in that list it does not know can say so. A name is
//! prefixed with `~` when the writer reads a difference in it as information only. Bundles written before this param
//! existed have no list, and builds before it cannot read one.
use super::registry::Rule;

/// The param holding the list. It is metadata about the environment, not a compared field.
pub const MARKER: &str = "env.fields";
/// Room for the names of every field an owner is likely to add; a bundle is untrusted input, so it is still bounded.
pub const MAX_MARKER_CHARS: usize = 4096;
const MAX_NAME_CHARS: usize = 64;
const SEPARATOR: char = ',';
const INFORMATIONAL_PREFIX: char = '~';

/// One name from the list, with how the writer read a difference in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub rule: Rule,
}

/// Whether `name` is a name this format can carry: short, lowercase, and free of anything that could be misread as a
/// separator or shown as something else.
pub fn is_listable_name(name: &str) -> bool {
    (1..=MAX_NAME_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

/// The value of the list for these fields, in name order.
pub fn render<'a>(fields: impl Iterator<Item = (&'a str, Rule)>) -> String {
    let mut entries: Vec<String> = fields
        .map(|(name, rule)| match rule {
            Rule::MustMatch => name.to_string(),
            Rule::Informational => format!("{INFORMATIONAL_PREFIX}{name}"),
        })
        .collect();
    entries.sort_unstable();
    entries.join(&SEPARATOR.to_string())
}

/// The names in a list, or `None` when it is not a well-formed list (too long, an empty entry, a name that cannot be
/// carried, a name listed twice). A list that cannot be read is not guessed at; in particular a name listed both with
/// and without the informational mark has no single reading, so the order of the entries never decides it.
pub fn parse(value: &str) -> Option<Vec<Listed>> {
    if value.len() > MAX_MARKER_CHARS {
        return None;
    }
    let listed: Vec<Listed> = value
        .split(SEPARATOR)
        .map(|entry| {
            let (name, rule) = match entry.strip_prefix(INFORMATIONAL_PREFIX) {
                Some(name) => (name, Rule::Informational),
                None => (entry, Rule::MustMatch),
            };
            is_listable_name(name).then(|| Listed {
                name: name.to_string(),
                rule,
            })
        })
        .collect::<Option<_>>()?;
    let mut seen = std::collections::BTreeSet::new();
    listed
        .iter()
        .all(|l| seen.insert(l.name.as_str()))
        .then_some(listed)
}

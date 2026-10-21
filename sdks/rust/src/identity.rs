//! The process tag, and how it survives a spawn or an exec.
//!
//! The tag travels in the environment as `HAIRSPRING_PROC`, with the owning pid and two flags in
//! `HAIRSPRING_PROC_PID` (`<pid>`, `<pid>:a`, `<pid>:u`, `<pid>:au`; `a` = the tag has been announced with a hello,
//! `u` = the tag is not stable across runs):
//!
//! * root process: `HAIRSPRING_PROC` if the harness set it, else `p1`;
//! * spawned child (`std::process::Command`, `system`): the inherited pid is not its own, so the inherited tag belongs
//!   to an ancestor and the child takes `<inherited>.s<pid>`, flagged unstable (a pid differs between runs);
//! * exec without fork: the inherited pid IS its own and the tag was announced, so the image takes `<tag>.x1`.
//!
//! Unlike the C SDK there is no fork handler: a raw `fork` child shares its parent's state and reports nothing of
//! its own (see docs/rust-sdk.md).

use crate::fmt::{ident_hash, is_ident_char, TAG_MAX};

pub const ENV_TAG: &str = "HAIRSPRING_PROC";
pub const ENV_PID: &str = "HAIRSPRING_PROC_PID";
pub const DEFAULT_TAG: &str = "p1";
/// Readable part kept when a derived tag would pass 64 bytes.
const HASH_PREFIX_MAX: usize = 40;
const HASH_MIX: u64 = 0x9e37_79b9_7f4a_7c15;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub tag: String,
    pub tag_stable: bool,
    /// The harness gave an invalid tag: it was refused, not shortened, and the SDK counts it as dropped.
    pub refused: bool,
}

/// 1 to 64 bytes of `[A-Za-z0-9._:/-]`, and not `_eval`, which is reserved for `eval` lines.
pub fn tag_valid(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= TAG_MAX && tag.bytes().all(is_ident_char) && tag != "_eval"
}

/// `base.<letter><n>`, or `<first 40 bytes of base>.h<16 hex digits>` when that would not fit in a tag.
pub fn derive(base: &str, letter: char, n: u64) -> String {
    let plain = format!("{base}.{letter}{n}");
    if plain.len() <= TAG_MAX {
        return plain;
    }
    let keep = base.len().min(HASH_PREFIX_MAX);
    let mix = ident_hash(base.as_bytes(), TAG_MAX) ^ n.wrapping_mul(HASH_MIX) ^ u64::from(letter);
    format!("{}.h{mix:016x}", &base[..keep])
}

/// The value of `HAIRSPRING_PROC_PID`.
pub fn owner_field(pid: u32, announced: bool, stable: bool) -> String {
    if !announced && stable {
        return pid.to_string();
    }
    format!(
        "{pid}:{}{}",
        if announced { "a" } else { "" },
        if stable { "" } else { "u" }
    )
}

struct Owner {
    pid: u64,
    announced: bool,
    unstable: bool,
}

/// `<pid>` or `<pid>:` followed by the flags `a` and `u`; anything else is not an owner field.
fn parse_owner(text: &str) -> Option<Owner> {
    let digits_end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let pid = text[..digits_end].parse().ok()?;
    let rest = &text[digits_end..];
    if rest.is_empty() {
        return Some(Owner {
            pid,
            announced: false,
            unstable: false,
        });
    }
    let flags = rest.strip_prefix(':')?;
    if !flags.bytes().all(|c| c == b'a' || c == b'u') {
        return None;
    }
    Some(Owner {
        pid,
        announced: flags.contains('a'),
        unstable: flags.contains('u'),
    })
}

/// Decide the tag from the inherited environment values and this process's pid.
pub fn resolve(inherited_tag: Option<&str>, owner: Option<&str>, pid: u32) -> Identity {
    let root = |tag: &str, refused: bool| Identity {
        tag: tag.to_string(),
        tag_stable: true,
        refused,
    };
    let Some(inherited) = inherited_tag.filter(|t| !t.is_empty()) else {
        return root(DEFAULT_TAG, false);
    };
    if !tag_valid(inherited) {
        return root(DEFAULT_TAG, true);
    }
    let Some(owner_text) = owner else {
        return root(inherited, false);
    };
    match parse_owner(owner_text) {
        Some(o) if o.pid == u64::from(pid) => Identity {
            tag: if o.announced {
                derive(inherited, 'x', 1)
            } else {
                inherited.to_string()
            },
            tag_stable: !o.unstable,
            refused: false,
        },
        _ => Identity {
            tag: derive(inherited, 's', u64::from(pid)),
            tag_stable: false,
            refused: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(tag: &str, stable: bool, refused: bool) -> Identity {
        Identity {
            tag: tag.to_string(),
            tag_stable: stable,
            refused,
        }
    }

    #[test]
    fn without_a_harness_tag_the_root_process_is_p1() {
        assert_eq!(resolve(None, None, 10), ident("p1", true, false));
        assert_eq!(resolve(Some(""), None, 10), ident("p1", true, false));
    }

    #[test]
    fn a_harness_tag_without_an_owner_is_the_root_tag() {
        assert_eq!(
            resolve(Some("job-7"), None, 10),
            ident("job-7", true, false)
        );
    }

    #[test]
    fn an_invalid_harness_tag_is_refused_not_shortened() {
        for bad in ["_eval", "a b", &"x".repeat(65)] {
            assert_eq!(
                resolve(Some(bad), None, 10),
                ident("p1", true, true),
                "{bad}"
            );
        }
    }

    #[test]
    fn tags_are_valid_at_their_limits() {
        assert!(tag_valid("a"));
        assert!(tag_valid(&"x".repeat(TAG_MAX)));
        assert!(!tag_valid(""));
        assert!(!tag_valid("_eval"));
        assert!(!tag_valid("a#b"));
    }

    #[test]
    fn a_child_whose_inherited_pid_is_not_its_own_takes_a_spawn_tag() {
        assert_eq!(
            resolve(Some("p1"), Some("999"), 42),
            ident("p1.s42", false, false)
        );
        assert_eq!(
            resolve(Some("p1"), Some("junk"), 42),
            ident("p1.s42", false, false)
        );
    }

    #[test]
    fn an_image_that_owns_the_inherited_pid_keeps_the_tag() {
        assert_eq!(
            resolve(Some("p1"), Some("42"), 42),
            ident("p1", true, false)
        );
    }

    #[test]
    fn an_exec_without_fork_after_the_hello_takes_an_x_tag() {
        assert_eq!(
            resolve(Some("p1"), Some("42:a"), 42),
            ident("p1.x1", true, false)
        );
    }

    #[test]
    fn an_unstable_tag_stays_unstable_across_an_exec() {
        assert_eq!(
            resolve(Some("p1.s9"), Some("42:u"), 42),
            ident("p1.s9", false, false)
        );
        assert_eq!(
            resolve(Some("p1.s9"), Some("42:au"), 42),
            ident("p1.s9.x1", false, false)
        );
    }

    #[test]
    fn the_owner_field_records_the_pid_and_its_flags() {
        assert_eq!(owner_field(42, false, true), "42");
        assert_eq!(owner_field(42, true, true), "42:a");
        assert_eq!(owner_field(42, false, false), "42:u");
        assert_eq!(owner_field(42, true, false), "42:au");
    }

    #[test]
    fn derived_tags_append_a_letter_and_a_number() {
        assert_eq!(derive("p1", 'f', 2), "p1.f2");
    }

    #[test]
    fn a_derived_tag_that_would_pass_64_bytes_keeps_a_prefix_and_a_hash() {
        let base = "b".repeat(62);
        let tag = derive(&base, 's', 123_456);
        assert!(tag.len() <= TAG_MAX, "{tag}");
        assert!(tag.starts_with(&"b".repeat(HASH_PREFIX_MAX)));
        assert!(tag.contains(".h"));
        assert!(tag_valid(&tag));
        assert_ne!(tag, derive(&base, 's', 123_457));
    }
}

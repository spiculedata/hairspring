//! `GET /api/search`: a bounded lookup of ids and names, grouped, for the web UI's search box.
//!
//! What it looks at is fixed here, so every front end gets the same answer and the cost is a number written down:
//!
//! * the text must be 1 to [`MAX_SEARCH_CHARS`] characters of ASCII letters, digits and `_ - . : /`; anything else is
//!   refused (the answer never repeats it);
//! * each group holds at most [`SEARCH_PER_GROUP`] hits, and `more` names the groups with further hits;
//! * it matches ids and names only, exactly or by prefix, case-sensitively: a run id, a failure id, a target name, a
//!   property id, a seed (digits, exactly) and a schedule fingerprint (16 to 64 lower case hex digits, exactly). It
//!   never reads guest output, kept output, a bundle or a failure message;
//! * the store reads the newest [`SEARCH_RECENT_RUNS`] runs (the newest [`SEARCH_PROPERTY_RUNS`] for property ids)
//!   and, for a whole run or failure id, one primary key. See [`hairspring_store::SearchQuery`].
//!
//! Property ids come from the program under test, so they are untrusted: each is cut to one line of bounded length and
//! put under a `guest` object, as everywhere else in the API.
use crate::query::is_schedule_fingerprint;
use crate::sanitize::public_text;
use crate::{Result, Service, ServiceError};
use hairspring_engine::engine::Engine;
use hairspring_store::{
    FailureHit, RunHit, ScheduleHit, SearchQuery, SeedHit, SEARCH_PROPERTY_RUNS, SEARCH_RECENT_RUNS,
};
use serde::Serialize;

/// Longest search text, in characters.
pub const MAX_SEARCH_CHARS: usize = 64;
/// Most hits in any one group.
pub const SEARCH_PER_GROUP: usize = 5;
/// Largest seed a client can read back exactly (53 bits, as everywhere in the API).
const MAX_SEED: u64 = (1 << 53) - 1;
const MAX_SEED_DIGITS: usize = 16;
const PROPERTY_NOTE: &str = "ids come from the program under test and are untrusted text";

/// A target whose name starts with the text.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TargetHit {
    pub name: String,
}

/// A property id found in a run's catalog. The id is guest text.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PropertyHit {
    pub run_id: String,
    pub guest: PropertyGuest,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PropertyGuest {
    pub note: &'static str,
    pub id: String,
}

/// What a search looked at, so an answer of "nothing" can say where it looked.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchScope {
    pub per_group: usize,
    pub recent_runs: u32,
    pub property_runs: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchResults {
    pub runs: Vec<RunHit>,
    pub failures: Vec<FailureHit>,
    pub targets: Vec<TargetHit>,
    pub properties: Vec<PropertyHit>,
    pub seeds: Vec<SeedHit>,
    pub schedules: Vec<ScheduleHit>,
    /// Groups that have more hits than are listed.
    pub more: Vec<&'static str>,
    pub scope: SearchScope,
}

/// What the text asks for, once it has been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    text: String,
    seed: Option<u64>,
    fingerprint: Option<String>,
}

fn invalid() -> ServiceError {
    ServiceError::InvalidRequest("`q` is not valid".to_owned())
}

fn is_search_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/')
}

fn probe(q: Option<&str>) -> Result<Probe> {
    let text = q.ok_or_else(invalid)?;
    let chars = text.chars().count();
    if chars == 0 || chars > MAX_SEARCH_CHARS || !text.chars().all(is_search_char) {
        return Err(invalid());
    }
    let is_digits = text.bytes().all(|b| b.is_ascii_digit());
    let seed = (is_digits && text.len() <= MAX_SEED_DIGITS)
        .then(|| text.parse::<u64>().ok())
        .flatten()
        .filter(|n| *n <= MAX_SEED);
    Ok(Probe {
        text: text.to_owned(),
        seed,
        fingerprint: is_schedule_fingerprint(text).then(|| text.to_owned()),
    })
}

/// Keeps `limit` hits and reports whether there were more.
fn cut<T>(mut hits: Vec<T>, name: &'static str, more: &mut Vec<&'static str>) -> Vec<T> {
    if hits.len() > SEARCH_PER_GROUP {
        hits.truncate(SEARCH_PER_GROUP);
        more.push(name);
    }
    hits
}

/// Target names that start with `text`, in name order.
fn matching_targets(mut names: Vec<String>, text: &str) -> Vec<TargetHit> {
    names.sort();
    names
        .into_iter()
        .filter(|n| n.starts_with(text))
        .map(|name| TargetHit { name })
        .collect()
}

impl Service {
    /// Looks `q` up as described in the module documentation. The error for text that is not allowed does not repeat
    /// it.
    pub fn search(&self, engine: &Engine, q: Option<&str>) -> Result<SearchResults> {
        let probe = probe(q)?;
        let names = self
            .list_targets(engine)
            .into_iter()
            .map(|t| t.name)
            .collect();
        let found = self.store.search(&SearchQuery {
            text: probe.text.clone(),
            seed: probe.seed,
            fingerprint: probe.fingerprint,
            // One more than is shown, to know whether there is more.
            limit: SEARCH_PER_GROUP as u32 + 1,
            recent_runs: SEARCH_RECENT_RUNS,
            property_runs: SEARCH_PROPERTY_RUNS,
        })?;
        let mut more = Vec::new();
        let properties = found
            .properties
            .into_iter()
            .map(|p| PropertyHit {
                run_id: p.run_id,
                guest: PropertyGuest {
                    note: PROPERTY_NOTE,
                    id: public_text(&p.prop_id),
                },
            })
            .collect();
        Ok(SearchResults {
            runs: cut(found.runs, "runs", &mut more),
            failures: cut(found.failures, "failures", &mut more),
            targets: cut(matching_targets(names, &probe.text), "targets", &mut more),
            properties: cut(properties, "properties", &mut more),
            seeds: cut(found.seeds, "seeds", &mut more),
            schedules: cut(found.schedules, "schedules", &mut more),
            more,
            scope: SearchScope {
                per_group: SEARCH_PER_GROUP,
                recent_runs: SEARCH_RECENT_RUNS,
                property_runs: SEARCH_PROPERTY_RUNS,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(q: Option<&str>) -> String {
        match probe(q) {
            Err(ServiceError::InvalidRequest(m)) => m,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn text_outside_the_allowed_shape_is_refused_without_being_repeated() {
        let too_long = "a".repeat(MAX_SEARCH_CHARS + 1);
        let hostile = [
            "",
            " ",
            "a b",
            "a\nb",
            "a\0b",
            "%",
            "a%",
            "x' OR 1=1 --",
            "<script>alert(1)</script>",
            "caf\u{e9}",
            "\u{202e}run_",
            "run_\u{0}",
            too_long.as_str(),
        ];
        for q in hostile {
            let message = refused(Some(q));
            assert_eq!(message, "`q` is not valid");
            assert!(q.len() < 5 || !message.contains(q), "{q:?}");
        }
        assert_eq!(refused(None), "`q` is not valid");
    }

    #[test]
    fn the_longest_allowed_text_and_every_allowed_character_pass() {
        assert!(probe(Some(&"a".repeat(MAX_SEARCH_CHARS))).is_ok());
        assert!(probe(Some("aZ09_-.:/")).is_ok());
        assert!(probe(Some("x")).is_ok());
    }

    #[test]
    fn digits_are_also_a_seed_up_to_what_a_browser_reads_exactly() {
        assert_eq!(probe(Some("42")).unwrap().seed, Some(42));
        assert_eq!(probe(Some("007")).unwrap().seed, Some(7));
        assert_eq!(
            probe(Some("9007199254740991")).unwrap().seed,
            Some((1 << 53) - 1)
        );
        for not_a_seed in ["9007199254740992", "99999999999999999999", "12a", "run_1"] {
            assert_eq!(probe(Some(not_a_seed)).unwrap().seed, None, "{not_a_seed}");
        }
    }

    #[test]
    fn only_16_to_64_lower_case_hex_digits_are_also_a_schedule_fingerprint() {
        let fp = "0123456789abcdef";
        assert_eq!(probe(Some(fp)).unwrap().fingerprint.as_deref(), Some(fp));
        assert_eq!(probe(Some("0123456789abcde")).unwrap().fingerprint, None);
        assert_eq!(probe(Some("0123456789ABCDEF")).unwrap().fingerprint, None);
        let longest = "a".repeat(MAX_SEARCH_CHARS);
        assert_eq!(probe(Some(&longest)).unwrap().fingerprint, Some(longest));
    }

    #[test]
    fn targets_match_by_prefix_case_sensitively_in_name_order() {
        let names = vec!["lock-lease".into(), "lock-a".into(), "Lock-b".into()];
        let found: Vec<_> = matching_targets(names, "lock")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(found, vec!["lock-a", "lock-lease"]);
        assert!(matching_targets(vec!["a".into()], "ab").is_empty());
    }

    #[test]
    fn a_group_over_its_limit_is_cut_and_named() {
        let mut more = Vec::new();
        let kept = cut((0..SEARCH_PER_GROUP + 1).collect(), "runs", &mut more);
        assert_eq!(
            (kept.len(), more.as_slice()),
            (SEARCH_PER_GROUP, &["runs"][..])
        );
        let kept = cut((0..SEARCH_PER_GROUP).collect(), "failures", &mut more);
        assert_eq!((kept.len(), more.len()), (SEARCH_PER_GROUP, 1));
    }
}

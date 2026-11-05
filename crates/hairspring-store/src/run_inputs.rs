//! Run inputs (schema version 8): the SHA-256 of each input a run executed with, kept once per run so that two runs can
//! later be told apart as "same instrument" or "different instrument".
//!
//! Only digests are stored, never values: the execution inputs include the working directory and the guest
//! environment, which can hold secrets. Digests of short values can still be guessed, so the store holds them and
//! nothing serves them (the service reports names and same/different only).
//!
//! The rules both stores share live here, so SQLite and PostgreSQL cannot differ: what a record may hold, and when a
//! second record for the same run is a repeat or a conflict.
use crate::lineage::IDENTITY_SCHEMES;
use crate::lineage_fields::is_sha256_hex;
use std::collections::{BTreeMap, BTreeSet};

/// Most fields (recorded and missing together) one record may name.
pub const MAX_RUN_INPUT_FIELDS: usize = 64;
/// Longest field name, in bytes.
pub const MAX_RUN_INPUT_NAME_BYTES: usize = 40;
/// Most text either JSON column may hold.
pub const MAX_RUN_INPUTS_JSON_BYTES: usize = 16 * 1024;
const MAX_REPORTER_BYTES: usize = 128;
/// The largest attempt number a record may carry (the column is a 32-bit integer on PostgreSQL).
const MAX_ATTEMPT: u32 = i32::MAX as u32;
const MAX_RUN_ID_BYTES: usize = 128;

/// The inputs of a run, as offered by whoever ran it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRunInputs {
    pub run_id: String,
    /// The identity scheme (`sim/1`, `wasm/1` or `hermit/1`).
    pub scheme: String,
    /// Lower-case SHA-256 hex of each recorded input, by name.
    pub fields: BTreeMap<String, String>,
    /// The names of the inputs that could not be recorded.
    pub missing: Vec<String>,
    pub reporter_id: String,
    pub reporter_trusted: bool,
    /// The attempt of the job that ran (its lease count, from 1). A job leased again is a new attempt whose trials
    /// replace the earlier attempt's, so its inputs replace theirs.
    pub attempt: u32,
}

/// A stored record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunInputsRecord {
    pub run_id: String,
    pub scheme: String,
    pub fields: BTreeMap<String, String>,
    /// Sorted, without repeats.
    pub missing: Vec<String>,
    pub reporter_id: String,
    pub reporter_trusted: bool,
    pub attempt: u32,
    pub created_at: i64,
}

/// Why the store refused a record. Nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunInputsRejection {
    RunNotFound,
    /// The run already has different inputs from the same attempt, and stored rows never change within an attempt.
    Conflict,
    /// The run has inputs from a later attempt of its job.
    Stale,
    /// A fixed description of what was wrong; it never repeats the offered text.
    Invalid(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunInputsOutcome {
    Stored,
    /// Inputs from an earlier attempt were replaced, whole, by these.
    Replaced,
    /// The same inputs were already stored for this attempt (the first reporter and time stay).
    Duplicate,
    Rejected(RunInputsRejection),
}

/// A record checked and rendered for storage. Equal content renders to equal text, so a repeat is a text comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prepared {
    pub fields_json: String,
    pub missing_json: String,
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains('\0')
}

/// Lower-case letters, digits and underscores, starting with a letter.
fn name_ok(name: &str) -> bool {
    name.len() <= MAX_RUN_INPUT_NAME_BYTES
        && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Checks everything the store can know without a database, and renders the two JSON columns: keys sorted, missing
/// names sorted and without repeats.
pub(crate) fn prepare(new: &NewRunInputs) -> Result<Prepared, &'static str> {
    if !text_ok(&new.run_id, MAX_RUN_ID_BYTES) {
        return Err("run id is empty, too long or has a NUL");
    }
    if !IDENTITY_SCHEMES.contains(&new.scheme.as_str()) {
        return Err("unknown input scheme");
    }
    if new.attempt == 0 || new.attempt > MAX_ATTEMPT {
        return Err("attempt is not valid");
    }
    if !text_ok(&new.reporter_id, MAX_REPORTER_BYTES) {
        return Err("reporter id is empty, too long or has a NUL");
    }
    let missing: BTreeSet<&str> = new.missing.iter().map(String::as_str).collect();
    if new.fields.is_empty() {
        return Err("no input was recorded");
    }
    if new.fields.len() + missing.len() > MAX_RUN_INPUT_FIELDS {
        return Err("too many inputs");
    }
    if new.fields.keys().any(|k| !name_ok(k)) || missing.iter().any(|k| !name_ok(k)) {
        return Err("an input name is not valid");
    }
    if new.fields.values().any(|v| !is_sha256_hex(v)) {
        return Err("an input value is not a SHA-256 digest");
    }
    if missing.iter().any(|k| new.fields.contains_key(*k)) {
        return Err("an input is both recorded and missing");
    }
    let render = |json: Result<String, serde_json::Error>| {
        json.map_err(|_| "inputs could not be rendered")
            .and_then(|text| {
                if text.len() > MAX_RUN_INPUTS_JSON_BYTES {
                    Err("inputs are too large")
                } else {
                    Ok(text)
                }
            })
    };
    Ok(Prepared {
        fields_json: render(serde_json::to_string(&new.fields))?,
        missing_json: render(serde_json::to_string(&missing))?,
    })
}

/// Reads the two stored columns back. A column that is not what [`prepare`] writes means the row was changed by hand;
/// the message does not repeat it.
pub(crate) fn parse_columns(
    fields_json: &str,
    missing_json: &str,
) -> Result<(BTreeMap<String, String>, Vec<String>), &'static str> {
    let fields =
        serde_json::from_str(fields_json).map_err(|_| "stored run inputs are unreadable")?;
    let missing =
        serde_json::from_str(missing_json).map_err(|_| "stored run inputs are unreadable")?;
    Ok((fields, missing))
}

/// What the store does with a record, given the row it holds for the run (its scheme, columns and attempt), if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Insert,
    Replace,
    Duplicate,
    Conflict,
    Stale,
}

/// The one rule both stores follow: a later attempt replaces, the same attempt repeats or conflicts, an earlier
/// attempt is stale.
pub(crate) fn decide(
    stored: Option<(&str, &str, &str, u32)>,
    new: &NewRunInputs,
    prepared: &Prepared,
) -> Decision {
    let Some((scheme, fields_json, missing_json, attempt)) = stored else {
        return Decision::Insert;
    };
    match new.attempt.cmp(&attempt) {
        std::cmp::Ordering::Greater => Decision::Replace,
        std::cmp::Ordering::Less => Decision::Stale,
        std::cmp::Ordering::Equal => {
            let same = scheme == new.scheme
                && fields_json == prepared.fields_json
                && missing_json == prepared.missing_json;
            if same {
                Decision::Duplicate
            } else {
                Decision::Conflict
            }
        }
    }
}

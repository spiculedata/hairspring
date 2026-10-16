//! Kept trial detail: a versioned JSON record of one trial's folded events.
//!
//! Only some trials keep their detail (failing, degraded, a first few, a sample), so the record holds what a person
//! reading one trial needs: processes, threads, per-id counts with witnesses, conflicts and event health. It crosses
//! the worker boundary as untrusted bytes, so [`TrialRecord::decode`] re-checks size, version and every string.

use crate::limits::{
    MAX_HASH_BYTES, MAX_IDS_PER_RUN, MAX_ID_BYTES, MAX_LOC_FILE_BYTES, MAX_LOC_FUNC_BYTES,
    MAX_MSG_BYTES, MAX_PROCS_PER_TRIAL, MAX_PROC_BYTES, MAX_SDK_STR_BYTES, MAX_THREADS_PER_PROC,
    MAX_THREAD_KEY_BYTES,
};
use crate::trial::{TrialProperty, TrialProps, Witness};
use crate::valid::{is_ident, is_plain_text, is_thread_key};
use std::fmt;

/// The only record version this build reads and writes.
pub const RECORD_VERSION: u32 = 1;
/// Largest record, in bytes.
pub const MAX_DETAIL_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialRecord {
    pub v: u32,
    /// The property list was left out because the whole record would not fit.
    pub truncated: bool,
    pub props: TrialProps,
}

/// Why a record was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordError(pub String);

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "trial record: {}", self.0)
    }
}

impl std::error::Error for RecordError {}

fn refuse<T>(why: &str) -> Result<T, RecordError> {
    Err(RecordError(why.to_owned()))
}

impl TrialRecord {
    /// The record's bytes, or `None` if even the version without the property list is too large.
    pub fn encode(props: &TrialProps) -> Option<Vec<u8>> {
        let full = TrialRecord {
            v: RECORD_VERSION,
            truncated: false,
            props: props.clone(),
        };
        let bytes = serde_json::to_vec(&full).ok()?;
        if bytes.len() <= MAX_DETAIL_BYTES {
            return Some(bytes);
        }
        let short = TrialRecord {
            truncated: true,
            props: TrialProps {
                properties: Vec::new(),
                ..props.clone()
            },
            ..full
        };
        serde_json::to_vec(&short)
            .ok()
            .filter(|b| b.len() <= MAX_DETAIL_BYTES)
    }

    /// Reads and validates a record from untrusted bytes.
    pub fn decode(bytes: &[u8]) -> Result<TrialRecord, RecordError> {
        if bytes.len() > MAX_DETAIL_BYTES {
            return refuse("record is larger than the size cap");
        }
        let record: TrialRecord = serde_json::from_slice(bytes)
            .map_err(|e| RecordError(format!("record is not valid: {e}")))?;
        if record.v != RECORD_VERSION {
            return refuse("unsupported record version");
        }
        record.props.check()?;
        Ok(record)
    }
}

fn plain(s: &str, max: usize) -> Result<(), RecordError> {
    if is_plain_text(s, max) {
        Ok(())
    } else {
        refuse("text is too long or has control or format characters")
    }
}

fn witness(w: &Witness) -> Result<(), RecordError> {
    if is_ident(&w.proc, MAX_PROC_BYTES) && is_thread_key(&w.thread, MAX_THREAD_KEY_BYTES) {
        Ok(())
    } else {
        refuse("a witness names an invalid process or thread")
    }
}

fn property(p: &TrialProperty) -> Result<(), RecordError> {
    if !is_ident(&p.id, MAX_ID_BYTES) {
        return refuse("an id is empty, too long or has characters outside the id set");
    }
    plain(&p.msg, MAX_MSG_BYTES)?;
    if let Some(loc) = &p.loc {
        plain(&loc.file, MAX_LOC_FILE_BYTES)?;
        if let Some(f) = &loc.func {
            plain(f, MAX_LOC_FUNC_BYTES)?;
        }
    }
    p.first_false
        .iter()
        .chain(&p.first_true)
        .try_for_each(witness)
}

impl TrialProps {
    /// Re-checks counts and strings against the protocol's own limits.
    fn check(&self) -> Result<(), RecordError> {
        if self.procs.len() > MAX_PROCS_PER_TRIAL {
            return refuse("too many processes");
        }
        if self.properties.len() > MAX_IDS_PER_RUN || self.conflicts.len() > MAX_IDS_PER_RUN {
            return refuse("too many ids");
        }
        for p in &self.procs {
            if !is_ident(&p.tag, MAX_PROC_BYTES) {
                return refuse("a process tag is invalid");
            }
            for s in [&p.sdk, &p.sdk_version].into_iter().flatten() {
                plain(s, MAX_SDK_STR_BYTES)?;
            }
            if let Some(h) = &p.catalog_hash {
                plain(h, MAX_HASH_BYTES)?;
            }
            if p.threads.len() > MAX_THREADS_PER_PROC {
                return refuse("too many threads");
            }
            if !p
                .threads
                .iter()
                .all(|t| is_thread_key(&t.thread, MAX_THREAD_KEY_BYTES))
            {
                return refuse("a thread key is invalid");
            }
        }
        self.properties.iter().try_for_each(property)?;
        for c in &self.conflicts {
            if !is_ident(&c.id, MAX_ID_BYTES) || !is_ident(&c.proc, MAX_PROC_BYTES) {
                return refuse("a conflict names an invalid id or process");
            }
        }
        Ok(())
    }
}

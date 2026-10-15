//! Hermit's `--summary-json`, reduced to a fingerprint of the schedule a trial actually ran.
//!
//! A passing trial prints the same line whatever interleaving it ran, so output cannot tell two seeds apart. The summary
//! counts syscalls, scheduler turns and timeslices, which do differ. Wall-clock fields are left out: they differ between
//! identical schedules.
use serde_json::Value;
use std::io::Read;
use std::path::Path;

/// The summary is a small JSON object; anything larger is not one and is not read.
const MAX_SUMMARY_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// FNV-1a over the deterministic fields, as 16 hex digits.
    pub fingerprint: String,
    /// Syscalls the guest completed: a measure of the work the trial did.
    pub syscalls: u64,
}

/// Reads a summary file. Missing, oversized or malformed files give `None`; the trial itself is unaffected.
pub fn read(path: &Path) -> Option<Summary> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_SUMMARY_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_SUMMARY_BYTES {
        return None;
    }
    parse(&text)
}

pub fn parse(text: &str) -> Option<Summary> {
    let v: Value = serde_json::from_str(text).ok()?;
    let syscalls = v.get("syscalls")?.as_u64()?;
    v.get("sched_turns")?.as_u64()?;
    let canonical = [
        "sched_turns",
        "syscalls",
        "virttime_elapsed",
        "timeslice_stats",
        "per_thread_timeslice",
    ]
    .iter()
    .map(|k| v.get(*k).map(Value::to_string).unwrap_or_default())
    .collect::<Vec<_>>()
    .join("|");
    Some(Summary {
        fingerprint: fnv1a_hex(canonical.as_bytes()),
        syscalls,
    })
}

fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

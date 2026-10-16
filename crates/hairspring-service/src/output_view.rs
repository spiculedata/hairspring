//! Reading a trial's kept-output record for the detail view.
//!
//! The record is JSON a worker sent, so nothing in it is trusted: each part is read on its own, and a part that is not
//! the expected shape or size is left out rather than cut or guessed at.
use crate::view::{OmittedBytes, ScheduleView};
use serde_json::Value;

/// The streams a trial's kept output can hold.
pub(crate) const STREAMS: [&str; 2] = ["stdout", "stderr"];
/// Longest text field of a schedule summary that is served.
const MAX_SCHEDULE_FIELD_BYTES: usize = 8 * 1024;

/// What the detail view reads from a kept-output record.
#[derive(Debug, Default)]
pub(crate) struct OutputMeta {
    pub streams: Vec<&'static str>,
    pub omitted: OmittedBytes,
    pub schedule: Option<ScheduleView>,
}

fn text_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)?
        .as_str()
        .filter(|s| s.len() <= MAX_SCHEDULE_FIELD_BYTES)
        .map(str::to_owned)
}

fn schedule(v: &Value) -> Option<ScheduleView> {
    Some(ScheduleView {
        sched_turns: v.get("sched_turns")?.as_u64()?,
        syscalls: v.get("syscalls")?.as_u64()?,
        virttime_elapsed: text_field(v, "virttime_elapsed"),
        timeslice_stats: text_field(v, "timeslice_stats"),
        per_thread_timeslice: text_field(v, "per_thread_timeslice"),
    })
}

fn omitted(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

impl OutputMeta {
    /// An unreadable record reads as one with nothing in it.
    pub(crate) fn read(bytes: &[u8]) -> OutputMeta {
        let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
            return OutputMeta::default();
        };
        OutputMeta {
            streams: STREAMS
                .into_iter()
                .filter(|s| v.get(*s).and_then(Value::as_str).is_some())
                .collect(),
            omitted: OmittedBytes {
                stdout: omitted(&v, "stdout_omitted_bytes"),
                stderr: omitted(&v, "stderr_omitted_bytes"),
            },
            schedule: v.get("schedule").and_then(schedule),
        }
    }
}

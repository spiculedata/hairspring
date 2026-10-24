//! Turning a judged trial into the record a worker reports: its row, its digest, and, for the trials worth it, its
//! detail and output tails.
use crate::exec::Trial;
use crate::trace::{behaviour_hash, Verdict};
use hairspring_engine::trial_log::{one_line, ReportedVerdict, TrialReport, MAX_REASON_CHARS};
use hairspring_props::record::TrialRecord;
use hairspring_props::{CatalogUpdate, DigestFlags, TrialDigest, TrialProps};

/// The first trials of a run always keep their detail, so every run has a few examples to look at.
pub const DETAIL_FIRST_TRIALS: u64 = 32;
/// One trial in this many keeps its detail whatever it did, so passing trials are represented too.
pub const DETAIL_SAMPLE_EVERY: u64 = 100;
/// Most bytes kept from the end of each output stream.
pub const OUTPUT_TAIL_BYTES: usize = 64 * 1024;
/// Largest kept output record once serialised; the tails shrink until it fits.
pub const MAX_OUTPUT_JSON_BYTES: usize = 128 * 1024;
/// Lines Hermit itself writes to stderr; they say nothing about the guest.
const HERMIT_LINE_PREFIX: &str = "hermit:";

/// The digest of a trial whose events cannot be counted: stored for the record, left out of the statistics.
pub fn excluded_digest(trial: &Trial) -> TrialDigest {
    TrialDigest {
        flags: DigestFlags {
            excluded: true,
            inconclusive: trial.timed_out,
            degraded: true,
            ..DigestFlags::default()
        },
        ..TrialDigest::default()
    }
}

fn verdict_parts(v: &Verdict) -> (ReportedVerdict, &str) {
    match v {
        Verdict::Pass => (ReportedVerdict::Pass, ""),
        Verdict::Violation(r) => (ReportedVerdict::Violation, r),
        // A deadlock is a finding; the stored verdict set has no separate kind, so the reason says what it was.
        Verdict::Deadlock(r) => (ReportedVerdict::Violation, r),
        Verdict::Hang(r) => (ReportedVerdict::Inconclusive, r),
        Verdict::Inconclusive(r) => (ReportedVerdict::Unusable, r),
        Verdict::Infra(r) => (ReportedVerdict::Infra, r),
    }
}

/// `reason` followed by `note`, within [`MAX_REASON_CHARS`]: it is the reason that is cut, never the note, so a long
/// reason cannot hide what was said about the trial.
pub fn with_note(reason: &str, note: &str) -> String {
    const SEPARATOR: &str = " | ";
    if reason.is_empty() {
        return one_line(note);
    }
    let room = MAX_REASON_CHARS.saturating_sub(note.chars().count() + SEPARATOR.len());
    let head: String = reason.chars().take(room).collect();
    format!("{head}{SEPARATOR}{note}")
}

fn keeps_detail(seq: u64, seed: u64, verdict: ReportedVerdict, degraded: bool) -> bool {
    verdict != ReportedVerdict::Pass
        || degraded
        || seq < DETAIL_FIRST_TRIALS
        || seed.is_multiple_of(DETAIL_SAMPLE_EVERY)
}

/// The last `max` bytes of `text`, starting on a character boundary, without Hermit's own lines.
fn tail(text: &str, max: usize) -> String {
    let guest: String = text
        .lines()
        .filter(|l| !l.starts_with(HERMIT_LINE_PREFIX))
        .map(|l| format!("{l}\n"))
        .collect();
    if guest.len() <= max {
        return guest;
    }
    let mut start = guest.len() - max;
    while !guest.is_char_boundary(start) {
        start += 1;
    }
    guest[start..].to_string()
}

/// The record for one trial: both tails, `<stream>_omitted_bytes` for a stream the capture had to cut, and the
/// scheduler summary's deterministic fields when Hermit wrote one.
fn output_record(trial: &Trial, limit: usize) -> serde_json::Value {
    let mut record = serde_json::json!({
        "stdout": tail(&trial.stdout, limit),
        "stderr": tail(&trial.stderr, limit),
    });
    if let Some(s) = &trial.schedule {
        record["schedule"] = serde_json::json!({
            "sched_turns": s.sched_turns,
            "syscalls": s.syscalls,
            "virttime_elapsed": s.virttime_elapsed,
            "timeslice_stats": s.timeslice_stats,
            "per_thread_timeslice": s.per_thread_timeslice,
        });
    }
    for (stream, omitted) in [
        ("stdout_omitted_bytes", trial.stdout_omitted),
        ("stderr_omitted_bytes", trial.stderr_omitted),
    ] {
        if omitted.is_truncated() {
            record[stream] = omitted.bytes.into();
        }
    }
    record
}

/// `{"stdout": ..., "stderr": ...}`, with the tails halved until the serialised record fits.
fn output_json(trial: &Trial) -> String {
    let mut limit = OUTPUT_TAIL_BYTES;
    loop {
        let text = output_record(trial, limit).to_string();
        if text.len() <= MAX_OUTPUT_JSON_BYTES || limit == 0 {
            return text;
        }
        limit /= 2;
    }
}

/// What the run counted for one judged trial.
pub struct Counted<'a> {
    pub props: &'a TrialProps,
    pub digest: TrialDigest,
    pub updates: Vec<CatalogUpdate>,
}

pub fn build(
    seq: u64,
    seed: u64,
    trial: &Trial,
    verdict: &Verdict,
    counted: Option<Counted>,
) -> TrialReport {
    let (kind, reason) = verdict_parts(verdict);
    let (digest, catalog, props) = match counted {
        Some(c) => (c.digest, c.updates, Some(c.props)),
        None => (excluded_digest(trial), Vec::new(), None),
    };
    let degraded = digest.flags.degraded;
    let keep = keeps_detail(seq, seed, kind, degraded);
    let detail = props
        .filter(|_| keep)
        .and_then(TrialRecord::encode)
        .and_then(|b| String::from_utf8(b).ok());
    TrialReport {
        seq,
        seed,
        verdict: kind,
        reason: one_line(reason),
        exit_code: trial.exit_code,
        signal: trial.signal,
        behaviour_hash: Some(behaviour_hash(trial)),
        schedule_fp: trial.schedule.as_ref().map(|s| s.fingerprint.clone()),
        syscalls: trial.schedule.as_ref().map_or(0, |s| s.syscalls),
        work_ops: props.map_or(0, TrialProps::total_work),
        elapsed_ms: trial.elapsed_ms,
        catalog,
        digest: digest.encode(),
        detail,
        output: keep.then(|| output_json(trial)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_follows_the_reason_and_stands_alone_when_there_is_none() {
        assert_eq!(
            with_note("boom", "rerun reproduced 3/3"),
            "boom | rerun reproduced 3/3"
        );
        assert_eq!(
            with_note("", "rerun reproduced 3/3"),
            "rerun reproduced 3/3"
        );
    }

    #[test]
    fn a_long_reason_is_cut_before_the_note_is() {
        let text = with_note(&"x".repeat(900), "not confirmed");
        assert_eq!(text.chars().count(), MAX_REASON_CHARS);
        assert!(text.ends_with(" | not confirmed"), "{text}");
    }
}

//! Turning a judged trial into the record a worker reports: its row, its digest, and, for the trials worth it, its
//! detail and output tails.
use crate::exec::Trial;
use crate::trace::{behaviour_hash, Verdict};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
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
/// Longest reason kept, in characters.
pub const MAX_REASON_CHARS: usize = 500;
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
        Verdict::Hang(r) => (ReportedVerdict::Inconclusive, r),
        Verdict::Inconclusive(r) => (ReportedVerdict::Unusable, r),
        Verdict::Infra(r) => (ReportedVerdict::Infra, r),
    }
}

/// One line of at most [`MAX_REASON_CHARS`] characters: control characters become spaces, runs of spaces collapse.
fn one_line(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_REASON_CHARS).collect()
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

/// `{"stdout": ..., "stderr": ...}`, with the tails halved until the serialised record fits.
fn output_json(trial: &Trial) -> String {
    let mut limit = OUTPUT_TAIL_BYTES;
    loop {
        let text = serde_json::json!({
            "stdout": tail(&trial.stdout, limit),
            "stderr": tail(&trial.stderr, limit),
        })
        .to_string();
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

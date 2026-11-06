//! The readable trace for a trial, its behaviour hash, and Hermit's side of its verdict.
use crate::exec::Trial;
use crate::verdict_facts::{facts_of, is_skid_overshoot};
use hairspring_engine::report::Step;
use hairspring_engine::verdict::judge;

pub use hairspring_engine::verdict::Verdict;

const MAX_EXCERPT_LINES: usize = 40;
const MAX_LINE_CHARS: usize = 300;

/// Names how `behaviour_hash` treats a stream over the capture cap (head, tail and a hash of the omitted middle).
/// Bundles record it; one without it was hashed from the first 1 MiB alone, and cannot be compared on cut output.
pub const HASH_SCHEME: &str = "2";

/// FNV-1a over the guest's observable behaviour (exit status and stdout).
pub fn behaviour_hash(t: &Trial) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(&t.exit_code.unwrap_or(-1).to_le_bytes());
    feed(&t.signal.unwrap_or(-1).to_le_bytes());
    feed(t.stdout.as_bytes());
    if t.stdout_omitted.is_truncated() {
        // The kept text shows only how many bytes were left out; the hash of the lost bytes tells runs apart that
        // differ only there. A stream kept whole hashes exactly as it always did.
        feed(b"omitted");
        feed(&t.stdout_omitted.bytes.to_le_bytes());
        feed(&t.stdout_omitted.hash.to_le_bytes());
    }
    if let Some(digest) = t.events_digest {
        // Only trials that reported events get this, so every other trial hashes exactly as it always did.
        feed(b"events");
        feed(&digest.to_le_bytes());
    }
    format!("{h:016x}")
}

/// Whether Hermit panicked on a PMU skid overshoot in this trial. Such a trial is `Infra`, and the one failure of
/// Hermit's own that a confirmation retries.
pub(crate) fn skid_overshoot(t: &Trial) -> bool {
    is_skid_overshoot(&t.stderr_findings())
}

/// Judges a trial: Hermit declares the facts, the engine's ladder decides.
pub fn classify(t: &Trial, timeout_secs: u64) -> Verdict {
    judge(&facts_of(t, timeout_secs))
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// A short readable trace: how it was run, how it ended, and the tail of its output.
pub fn steps(
    command: &[String],
    seed: u64,
    epoch: &str,
    heuristic: &str,
    t: &Trial,
    verdict_note: Option<String>,
) -> Vec<Step> {
    let mut out = vec![Step {
        index: 0,
        action_id: format!("{seed:016x}"),
        label: clip(
            &format!(
                "run `{}` under hermit with sched-seed {seed}",
                command.join(" ")
            ),
            MAX_LINE_CHARS,
        ),
        state: format!("epoch={epoch} heuristic={heuristic}"),
    }];
    let ending = if t.timed_out {
        "timed out (killed)".to_string()
    } else {
        match (t.exit_code, t.signal) {
            (Some(c), _) => format!("exit code {c}"),
            (None, Some(s)) => format!("terminated by signal {s}"),
            _ => "terminated abnormally".to_string(),
        }
    };
    let mut push = |label: String| {
        let index = out.len();
        out.push(Step {
            index,
            action_id: format!("{index:016x}"),
            label: clip(&label, MAX_LINE_CHARS + 16),
            state: String::new(),
        });
    };
    push(ending);
    if let Some(note) = verdict_note {
        push(note);
    }
    for (stream, text) in [("stdout", &t.stdout), ("stderr", &t.stderr)] {
        let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with("hermit:")).collect();
        let start = lines.len().saturating_sub(MAX_EXCERPT_LINES / 2);
        for line in &lines[start..] {
            push(format!("{stream}: {}", clip(line, MAX_LINE_CHARS)));
        }
    }
    out
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    fn trial(code: i32, stdout: &str) -> Trial {
        Trial {
            exit_code: Some(code),
            stdout: stdout.to_string(),
            ..Trial::default()
        }
    }

    /// Hashes recorded in bundles before guest events existed. They must never change: old bundles stay replayable.
    #[test]
    fn trials_without_events_keep_their_historic_hash() {
        assert_eq!(
            behaviour_hash(&trial(1, "HAIRSPRING_VIOLATION: boom\n")),
            "ae348452964ed6ea"
        );
        assert_eq!(behaviour_hash(&trial(0, "")), "3cd02a862a574321");
        let killed = Trial {
            signal: Some(9),
            ..Trial::default()
        };
        assert_eq!(behaviour_hash(&killed), "ae9561c0a73f1ad8");
    }

    #[test]
    fn events_change_the_hash_and_equal_events_do_not() {
        let plain = trial(1, "x\n");
        let with = |d: u64| Trial {
            events_digest: Some(d),
            ..plain.clone()
        };
        assert_ne!(behaviour_hash(&plain), behaviour_hash(&with(1)));
        assert_ne!(behaviour_hash(&with(1)), behaviour_hash(&with(2)));
        assert_eq!(behaviour_hash(&with(7)), behaviour_hash(&with(7)));
    }
}

//! Verdicts and the readable trace for a trial.
use crate::exec::Trial;
use hairspring_engine::report::Step;

const MARKER: &str = "HAIRSPRING_VIOLATION";
/// Programs written under the project's former name (Anthill) still report violations this way.
const LEGACY_MARKER: &str = "ANTHILL_VIOLATION";
const INTERNAL_FAILURE: &str = "HERMIT_INTERNAL_FAILURE";
const MAX_EXCERPT_LINES: usize = 40;
const MAX_LINE_CHARS: usize = 300;

fn is_marker_line(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with(MARKER) || l.starts_with(LEGACY_MARKER)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Violation(String),
    /// Hermit itself failed: not a finding about the target.
    Infra(String),
}

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
    format!("{h:016x}")
}

fn first_line(text: &str, containing: &str) -> Option<String> {
    text.lines()
        .find(|l| l.contains(containing))
        .map(|l| l.trim().to_string())
}

pub fn classify(t: &Trial, timeout_secs: u64) -> Verdict {
    if t.stderr.contains(INTERNAL_FAILURE) {
        let detail = first_line(&t.stderr, "Error:")
            .unwrap_or_else(|| "hermit reported an internal failure".to_string());
        return Verdict::Infra(format!("hermit failed to run the program: {detail}"));
    }
    if t.timed_out {
        return Verdict::Violation(format!(
            "timed out after {timeout_secs}s (possible hang or deadlock)"
        ));
    }
    if let Some(line) = t
        .stdout
        .lines()
        .chain(t.stderr.lines())
        .find(|l| is_marker_line(l))
    {
        let reason = line
            .trim_start()
            .trim_start_matches(MARKER)
            .trim_start_matches(LEGACY_MARKER)
            .trim_start_matches(':')
            .trim();
        return Verdict::Violation(if reason.is_empty() {
            "property violated".to_string()
        } else {
            reason.to_string()
        });
    }
    match (t.exit_code, t.signal) {
        (Some(0), _) => Verdict::Pass,
        (Some(code), _) => Verdict::Violation(format!("exit code {code}")),
        (None, Some(sig)) => Verdict::Violation(format!("terminated by signal {sig}")),
        (None, None) => Verdict::Violation("terminated abnormally".to_string()),
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// A short readable trace: how it was run, how it ended, and the tail of its output.
pub fn steps(
    command: &[String],
    seed: u64,
    epoch: &str,
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
        state: format!("epoch={epoch} heuristic=random"),
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

//! The text `hairspring confirm` prints for one confirmation: a line per check with its command and notes, the finding,
//! and what the environment says about it.
use std::collections::BTreeMap;
use std::io::{self, Write};

/// One check as it is printed.
pub(super) struct CheckLine {
    pub kind: String,
    pub status: String,
    pub passed: u32,
    pub failed: u32,
    pub inconclusive: u32,
    pub command: String,
    pub detail: BTreeMap<String, String>,
}

const ENVIRONMENT_DIFFERS: &str = "environment differs";

/// The exit code of a finding: 1 when it repeats, 0 when it did not on the environment it was recorded in, 3 for every
/// state that shows nothing about whether the bug is fixed.
pub(super) fn exit_code(finding: &str) -> i32 {
    match finding {
        "confirmed" | "verified" => crate::EXIT_VIOLATION,
        "not_reproducible" => crate::EXIT_OK,
        _ => crate::EXIT_INCONCLUSIVE,
    }
}

fn explanation(finding: &str) -> &'static str {
    match finding {
        "verified" => "the failure repeats on reruns and on a replay, and the strict verify agrees",
        "confirmed" => "the failure repeats on reruns and on a replay of its bundle",
        "reproduced" => "reruns repeat the failure, but no replay of the bundle has yet",
        "not_reproducible" => {
            "a rerun or the replay did not repeat the failure on the environment it was recorded in"
        }
        "control_failed" => "control failed: the fixed build still fails at this seed",
        _ => "no check has repeated the failure",
    }
}

pub(super) fn write_confirmation(
    out: &mut dyn Write,
    lines: &[CheckLine],
    finding: &str,
    pinned: bool,
) -> io::Result<()> {
    for line in lines {
        writeln!(
            out,
            "  {:<16} {:<13} {} passed, {} failed, {} inconclusive",
            line.kind, line.status, line.passed, line.failed, line.inconclusive
        )?;
        if !line.command.is_empty() {
            writeln!(out, "    command: {}", line.command)?;
        }
        for (key, value) in &line.detail {
            writeln!(out, "    {key}: {value}")?;
        }
    }
    let differs = lines.iter().any(|l| {
        l.detail
            .get("inconclusive")
            .is_some_and(|v| v.contains(ENVIRONMENT_DIFFERS))
    });
    if differs {
        writeln!(
            out,
            "inconclusive: environment differs from the recording; this does not show the bug is fixed"
        )?;
    }
    let pin = match finding {
        "confirmed" | "verified" if pinned => " (environment pinned)",
        "confirmed" | "verified" => " (environment not pinned)",
        _ => "",
    };
    writeln!(out, "finding: {finding}{pin}: {}", explanation(finding))
}

//! What a backend says about the machine it would run on, before any target is looked at.
//!
//! A probe is read-only and bounded. It names each prerequisite it checked, and for each one that is not met it says
//! which one and how to fix it.
use std::io::Write;

/// How one prerequisite came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    /// Met.
    Ok,
    /// Not met: the backend cannot run here until it is.
    Fail,
    /// Worth knowing, does not block the backend.
    Note,
}

impl ProbeStatus {
    fn word(self) -> &'static str {
        match self {
            ProbeStatus::Ok => "ok",
            ProbeStatus::Fail => "FAIL",
            ProbeStatus::Note => "note",
        }
    }
}

/// One prerequisite and what was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRow {
    pub label: String,
    pub status: ProbeStatus,
    pub detail: String,
}

/// The answer of one backend's `probe()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// What the report is about, in the words a person uses for it (for example `process backend`).
    pub subject: String,
    /// One row per prerequisite checked, in the order they are shown.
    pub rows: Vec<ProbeRow>,
    /// Said when every prerequisite is met, including what the probe did not show.
    pub ready_note: String,
    /// One entry per prerequisite that is not met: which one, and how to fix it. Empty when the backend can run here.
    pub missing: Vec<String>,
}

impl ProbeReport {
    /// A report for a backend that has no machine prerequisites to check: ready, with nothing claimed beyond that.
    pub fn nothing_to_check(subject: &str) -> ProbeReport {
        ProbeReport {
            subject: subject.to_string(),
            rows: Vec::new(),
            ready_note: "no prerequisites of this backend are checked on this machine".to_string(),
            missing: Vec::new(),
        }
    }

    /// A report for a probe that could not finish (it panicked). Never ready.
    pub fn failed(subject: &str) -> ProbeReport {
        ProbeReport {
            subject: subject.to_string(),
            rows: Vec::new(),
            ready_note: String::new(),
            missing: vec![
                "the check itself failed unexpectedly, so this machine cannot be said to be ready. Run it again, and \
                 report it if it keeps failing."
                    .to_string(),
            ],
        }
    }

    /// Whether the backend can run on this machine as far as the probe can tell.
    pub fn is_ready(&self) -> bool {
        self.missing.is_empty()
    }

    /// Writes the report: a title, one line per prerequisite, then the result and, when not ready, each missing
    /// prerequisite with its fix.
    pub fn write(&self, out: &mut dyn Write) -> std::io::Result<()> {
        writeln!(out, "{}: prerequisites on this machine", self.subject)?;
        for r in &self.rows {
            writeln!(
                out,
                "  {:<14} {:<4}  {}",
                r.label,
                r.status.word(),
                r.detail
            )?;
        }
        if self.is_ready() {
            return writeln!(out, "result: ok: {}", self.ready_note);
        }
        writeln!(
            out,
            "result: CANNOT RUN the {} on this machine:",
            self.subject
        )?;
        self.missing
            .iter()
            .try_for_each(|m| writeln!(out, "  - {m}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(report: &ProbeReport) -> String {
        let mut out = Vec::new();
        report.write(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn row(label: &str, status: ProbeStatus, detail: &str) -> ProbeRow {
        ProbeRow {
            label: label.into(),
            status,
            detail: detail.into(),
        }
    }

    #[test]
    fn a_report_with_nothing_missing_is_ready_and_prints_its_note() {
        let report = ProbeReport {
            subject: "demo backend".into(),
            rows: vec![row("tool", ProbeStatus::Ok, "found")],
            ready_note: "tool answered".into(),
            missing: vec![],
        };
        assert!(report.is_ready());
        assert_eq!(
            text(&report),
            "demo backend: prerequisites on this machine\n  tool           ok    found\nresult: ok: tool answered\n"
        );
    }

    #[test]
    fn a_report_with_something_missing_names_each_with_its_fix() {
        let report = ProbeReport {
            subject: "demo backend".into(),
            rows: vec![
                row("tool", ProbeStatus::Fail, "not found"),
                row("memory", ProbeStatus::Note, "low"),
            ],
            ready_note: String::new(),
            missing: vec!["tool was not found: install it".into(), "second".into()],
        };
        assert!(!report.is_ready());
        let text = text(&report);
        assert!(
            text.contains("  tool           FAIL  not found\n"),
            "{text}"
        );
        assert!(text.contains("  memory         note  low\n"), "{text}");
        assert!(
            text.ends_with(
                "result: CANNOT RUN the demo backend on this machine:\n  - tool was not found: install it\n  - second\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_backend_with_nothing_to_check_is_ready_and_claims_no_more() {
        let report = ProbeReport::nothing_to_check("Simulation");
        assert!(report.is_ready());
        assert!(report.rows.is_empty());
        assert!(text(&report).contains("no prerequisites of this backend are checked"));
    }

    #[test]
    fn a_probe_that_failed_is_never_ready_and_says_to_run_it_again() {
        let report = ProbeReport::failed("Hermit");
        assert!(!report.is_ready());
        assert!(text(&report).contains("Run it again"));
    }
}

//! How a verdict is recorded in a run's trial rows.
use super::Verdict;
use crate::trial_log::ReportedVerdict;

impl Verdict {
    /// The stored kind and the reason text of a verdict.
    pub fn reported(&self) -> (ReportedVerdict, &str) {
        match self {
            Verdict::Pass => (ReportedVerdict::Pass, ""),
            Verdict::Violation(r) => (ReportedVerdict::Violation, r),
            // A deadlock is a finding; the stored verdict set has no separate kind, so the reason says what it was.
            Verdict::Deadlock(r) => (ReportedVerdict::Violation, r),
            Verdict::Hang(r) => (ReportedVerdict::Inconclusive, r),
            Verdict::Inconclusive(r) => (ReportedVerdict::Unusable, r),
            Verdict::Infra(r) => (ReportedVerdict::Infra, r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_verdict_is_stored_as_one_kind_and_only_findings_are_violations() {
        let table = [
            (Verdict::Pass, ReportedVerdict::Pass, ""),
            (
                Verdict::Violation("v".into()),
                ReportedVerdict::Violation,
                "v",
            ),
            (
                Verdict::Deadlock("d".into()),
                ReportedVerdict::Violation,
                "d",
            ),
            (
                Verdict::Hang("h".into()),
                ReportedVerdict::Inconclusive,
                "h",
            ),
            (
                Verdict::Inconclusive("c".into()),
                ReportedVerdict::Unusable,
                "c",
            ),
            (Verdict::Infra("i".into()), ReportedVerdict::Infra, "i"),
        ];
        for (verdict, kind, reason) in table {
            assert_eq!(verdict.reported(), (kind, reason));
        }
    }
}

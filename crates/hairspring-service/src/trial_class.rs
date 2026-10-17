//! How a trial is shown in a list or on the seed strip: one class per trial, decided here and nowhere else.
//!
//! A trial that was not judged a clean, rated pass is never shown as a pass. A `pass` verdict whose trial was degraded,
//! left out of the statistics or reported nothing is `Excluded`, and the strip gives it a code of its own.
use hairspring_store::{TrialRecord, TrialStripRow, TrialVerdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrialClass {
    /// Judged a pass and counted: rated, not degraded, with a report.
    Pass,
    Violation,
    Inconclusive,
    Unusable,
    Infra,
    /// Judged a pass by the engine, but left out of every rate (degraded, unreported or otherwise not rated).
    Excluded,
}

/// Every class, in the order a legend lists them (most important first).
pub const ALL_CLASSES: [TrialClass; 6] = [
    TrialClass::Violation,
    TrialClass::Inconclusive,
    TrialClass::Unusable,
    TrialClass::Infra,
    TrialClass::Excluded,
    TrialClass::Pass,
];

impl TrialClass {
    pub fn of(t: &TrialRecord) -> TrialClass {
        TrialClass::from_parts(t.verdict, t.rated, t.degraded, t.report_missing)
    }

    /// The class of a trial read for the strip: the same rule as [`TrialClass::of`] over the same fields.
    pub fn of_strip(t: &TrialStripRow) -> TrialClass {
        TrialClass::from_parts(t.verdict, t.rated, t.degraded, t.report_missing)
    }

    fn from_parts(
        verdict: TrialVerdict,
        rated: bool,
        degraded: bool,
        report_missing: bool,
    ) -> TrialClass {
        match verdict {
            TrialVerdict::Violation => TrialClass::Violation,
            TrialVerdict::Inconclusive => TrialClass::Inconclusive,
            TrialVerdict::Unusable => TrialClass::Unusable,
            TrialVerdict::Infra => TrialClass::Infra,
            TrialVerdict::Pass if rated && !degraded && !report_missing => TrialClass::Pass,
            TrialVerdict::Pass => TrialClass::Excluded,
        }
    }

    /// The single character the strip uses for this class.
    pub fn code(self) -> char {
        match self {
            TrialClass::Pass => 'p',
            TrialClass::Violation => 'v',
            TrialClass::Inconclusive => 'i',
            TrialClass::Unusable => 'u',
            TrialClass::Infra => 'x',
            TrialClass::Excluded => 'e',
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TrialClass::Pass => "pass",
            TrialClass::Violation => "violation",
            TrialClass::Inconclusive => "inconclusive",
            TrialClass::Unusable => "unusable",
            TrialClass::Infra => "infra",
            TrialClass::Excluded => "excluded",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_store::KeptState;

    fn trial(verdict: TrialVerdict) -> TrialRecord {
        TrialRecord {
            run_id: "run_0".into(),
            seq: 0,
            seed: 1,
            verdict,
            reason: String::new(),
            exit_code: Some(0),
            signal: None,
            timed_out: false,
            degraded: false,
            perturbing: false,
            report_missing: false,
            rated: true,
            claimable: true,
            behaviour_hash: None,
            schedule_fp: None,
            syscalls: 0,
            work_ops: 0,
            elapsed_ms: 0,
            worker_id: "w".into(),
            worker_trusted: true,
            attempt: 1,
            kept: KeptState::NotKept,
            detail_ref: None,
            output_ref: None,
            digest: Vec::new(),
        }
    }

    #[test]
    fn a_strip_row_gets_the_class_its_full_row_gets() {
        let verdicts = [
            TrialVerdict::Pass,
            TrialVerdict::Violation,
            TrialVerdict::Inconclusive,
            TrialVerdict::Unusable,
            TrialVerdict::Infra,
        ];
        for verdict in verdicts {
            for bits in 0..8u8 {
                let (rated, degraded, report_missing) =
                    (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                let full = TrialRecord {
                    rated,
                    degraded,
                    report_missing,
                    ..trial(verdict)
                };
                let strip = hairspring_store::TrialStripRow {
                    seq: 0,
                    verdict,
                    rated,
                    degraded,
                    report_missing,
                };
                assert_eq!(TrialClass::of_strip(&strip), TrialClass::of(&full));
            }
        }
    }

    #[test]
    fn a_clean_rated_pass_is_a_pass() {
        assert_eq!(TrialClass::of(&trial(TrialVerdict::Pass)), TrialClass::Pass);
    }

    #[test]
    fn a_pass_that_was_degraded_unreported_or_not_rated_is_never_shown_as_a_pass() {
        let degraded = TrialRecord {
            degraded: true,
            ..trial(TrialVerdict::Pass)
        };
        let unreported = TrialRecord {
            report_missing: true,
            ..trial(TrialVerdict::Pass)
        };
        let unrated = TrialRecord {
            rated: false,
            ..trial(TrialVerdict::Pass)
        };
        for t in [degraded, unreported, unrated] {
            assert_eq!(TrialClass::of(&t), TrialClass::Excluded);
        }
    }

    #[test]
    fn every_other_verdict_keeps_its_own_class() {
        let cases = [
            (TrialVerdict::Violation, TrialClass::Violation),
            (TrialVerdict::Inconclusive, TrialClass::Inconclusive),
            (TrialVerdict::Unusable, TrialClass::Unusable),
            (TrialVerdict::Infra, TrialClass::Infra),
        ];
        for (verdict, class) in cases {
            // Even a degraded trial keeps the verdict the engine gave it.
            let t = TrialRecord {
                degraded: true,
                rated: false,
                ..trial(verdict)
            };
            assert_eq!(TrialClass::of(&t), class);
        }
    }

    #[test]
    fn codes_and_names_are_distinct() {
        let codes: std::collections::BTreeSet<char> =
            ALL_CLASSES.iter().map(|c| c.code()).collect();
        let names: std::collections::BTreeSet<&str> =
            ALL_CLASSES.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            (codes.len(), names.len()),
            (ALL_CLASSES.len(), ALL_CLASSES.len())
        );
        assert!(
            !codes.contains(&'-'),
            "'-' is the strip's mark for a trial with no stored row"
        );
    }
}

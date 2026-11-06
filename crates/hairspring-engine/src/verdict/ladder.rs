//! The one ladder every backend's trials are judged by.
use super::facts::TrialFacts;
use super::markers::marker_reason;
use super::properties::describe_failures;
use super::Verdict;

/// Judges a trial from the facts its backend declared.
///
/// Order: the backend's own failure and a misconfigured event channel (the run cannot go on), then a timeout, then a
/// host fault collecting events (both inconclusive, never a pass), then property failures (a false that was read
/// counts even when the rest of the report is damaged), then the violation marker line, then a positively detected
/// deadlock (with a non-zero exit), then the exit status. A trial that exits cleanly but whose evidence is unusable
/// is inconclusive, not a pass.
pub fn judge(facts: &TrialFacts) -> Verdict {
    if let Some(message) = &facts.backend_failure {
        return Verdict::Infra(message.clone());
    }
    if let Some(problem) = &facts.config_fault {
        return Verdict::Infra(problem.clone());
    }
    if facts.timed_out {
        return Verdict::Hang(format!(
            "timed out after {}s (possible hang or deadlock)",
            facts.timeout_secs
        ));
    }
    if let Some(problem) = &facts.collection_fault {
        return Verdict::Inconclusive(format!(
            "the guest's events could not be collected: {problem}"
        ));
    }
    if !facts.property_failures.is_empty() {
        return Verdict::Violation(describe_failures(&facts.property_failures));
    }
    if let Some(line) = &facts.marker_line {
        return Verdict::Violation(marker_reason(line));
    }
    if let (Some(report), Some(code)) = (&facts.deadlock, facts.exit_code) {
        if code != 0 {
            return Verdict::Deadlock(format!(
                "deadlock: {} reported `{}` (exit code {code})",
                report.reporter, report.line
            ));
        }
    }
    match (facts.exit_code, facts.signal) {
        (Some(0), _) => match &facts.unusable_evidence {
            Some(why) => Verdict::Inconclusive(why.clone()),
            None => Verdict::Pass,
        },
        (Some(code), _) => Verdict::Violation(format!("exit code {code}")),
        (None, Some(sig)) => Verdict::Violation(format!("terminated by signal {sig}")),
        (None, None) => Verdict::Violation("terminated abnormally".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::facts::DeadlockReport;
    use super::*;

    fn clean() -> TrialFacts {
        TrialFacts {
            timeout_secs: 5,
            exit_code: Some(0),
            ..TrialFacts::default()
        }
    }

    fn deadlock() -> Option<DeadlockReport> {
        Some(DeadlockReport {
            reporter: "the runner".into(),
            line: "all threads blocked".into(),
        })
    }

    #[test]
    fn only_a_clean_exit_with_no_other_evidence_is_a_pass() {
        assert_eq!(judge(&clean()), Verdict::Pass);
    }

    #[test]
    fn a_timeout_is_never_a_pass_whatever_else_the_trial_says() {
        let timed_out = |t: TrialFacts| TrialFacts {
            timed_out: true,
            ..t
        };
        let hang = Verdict::Hang("timed out after 5s (possible hang or deadlock)".into());
        assert_eq!(judge(&timed_out(clean())), hang);
        assert_eq!(
            judge(&timed_out(TrialFacts::default().with_timeout(5))),
            hang
        );
        let marked = TrialFacts {
            marker_line: Some("HAIRSPRING_VIOLATION: x".into()),
            ..clean()
        };
        assert_eq!(judge(&timed_out(marked)), hang);
        let held = TrialFacts {
            collection_fault: Some("disk".into()),
            ..clean()
        };
        assert_eq!(judge(&timed_out(held)), hang);
    }

    #[test]
    fn a_trial_with_no_exit_status_and_no_signal_is_a_violation_not_a_pass() {
        assert_eq!(
            judge(&TrialFacts::default()),
            Verdict::Violation("terminated abnormally".into())
        );
    }

    #[test]
    fn a_backend_failure_and_a_config_fault_come_before_a_timeout() {
        let t = TrialFacts {
            timed_out: true,
            backend_failure: Some("tool failed".into()),
            config_fault: Some("bad".into()),
            ..clean()
        };
        assert_eq!(judge(&t), Verdict::Infra("tool failed".into()));
        let t = TrialFacts {
            backend_failure: None,
            ..t
        };
        assert_eq!(judge(&t), Verdict::Infra("bad".into()));
    }

    #[test]
    fn unusable_evidence_on_a_clean_exit_is_inconclusive_and_only_there() {
        let t = TrialFacts {
            unusable_evidence: Some("events lost".into()),
            ..clean()
        };
        assert_eq!(judge(&t), Verdict::Inconclusive("events lost".into()));
        let failing = TrialFacts {
            exit_code: Some(3),
            ..t
        };
        assert_eq!(judge(&failing), Verdict::Violation("exit code 3".into()));
    }

    #[test]
    fn a_deadlock_needs_a_failing_exit_and_loses_to_a_marker() {
        let t = TrialFacts {
            deadlock: deadlock(),
            exit_code: Some(1),
            ..clean()
        };
        assert_eq!(
            judge(&t),
            Verdict::Deadlock(
                "deadlock: the runner reported `all threads blocked` (exit code 1)".into()
            )
        );
        let clean_exit = TrialFacts {
            exit_code: Some(0),
            ..t.clone()
        };
        assert_eq!(judge(&clean_exit), Verdict::Pass);
        let signalled = TrialFacts {
            exit_code: None,
            signal: Some(6),
            ..t.clone()
        };
        assert_eq!(
            judge(&signalled),
            Verdict::Violation("terminated by signal 6".into())
        );
        let marked = TrialFacts {
            marker_line: Some("HAIRSPRING_VIOLATION: m".into()),
            ..t
        };
        assert_eq!(judge(&marked), Verdict::Violation("m".into()));
    }

    #[test]
    fn the_hang_and_budget_rules_only_touch_hangs_and_violations() {
        let hang = Verdict::Hang("slow".into());
        assert_eq!(
            hang.clone().with_hang_policy(true),
            Verdict::Violation("slow".into())
        );
        assert_eq!(hang.clone().with_hang_policy(false), hang);
        assert_eq!(Verdict::Pass.with_hang_policy(true), Verdict::Pass);
        assert_eq!(
            Verdict::Violation("v".into()).cut_by_budget(),
            Verdict::Hang("v".into())
        );
        assert_eq!(hang.clone().cut_by_budget(), hang);
        for other in [
            Verdict::Pass,
            Verdict::Deadlock("d".into()),
            Verdict::Infra("i".into()),
            Verdict::Inconclusive("c".into()),
        ] {
            assert_eq!(other.clone().cut_by_budget(), other);
        }
    }

    impl TrialFacts {
        fn with_timeout(self, timeout_secs: u64) -> TrialFacts {
            TrialFacts {
                timeout_secs,
                ..self
            }
        }
    }
}

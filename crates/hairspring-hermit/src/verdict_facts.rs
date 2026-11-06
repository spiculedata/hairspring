//! What Hermit knows about a trial, declared as facts for the engine's verdict ladder. Hermit says what each fact
//! means (its own failures, its deadlock report); the engine decides the verdict.
use crate::events::EventsFault;
use crate::exec::Trial;
use crate::line_scan::Findings;
use crate::property_verdict;
use hairspring_engine::verdict::{DeadlockReport, TrialFacts};

/// How the verdict text names Hermit's deadlock report.
const REPORTER: &str = "hermit";

pub(crate) fn is_skid_overshoot(stderr: &Findings) -> bool {
    stderr.skid_overshoot || (stderr.panicked && stderr.overshoot)
}

/// Failures of Hermit itself that are not about the guest: its own internal failure, a command-line usage error, or
/// the PMU skid overshoot panic. Any of them would otherwise look like the guest exiting non-zero and be reported as
/// a finding.
fn own_failure(stderr: &Findings) -> Option<String> {
    if stderr.internal_failure {
        let detail = stderr
            .error_line
            .clone()
            .unwrap_or_else(|| "hermit reported an internal failure".to_string());
        return Some(format!("hermit failed to run the program: {detail}"));
    }
    if stderr.usage {
        let detail = stderr
            .lower_error_line
            .clone()
            .unwrap_or_else(|| "bad command line".to_string());
        return Some(format!(
            "hermit rejected its command line (usage error): {detail}"
        ));
    }
    if is_skid_overshoot(stderr) {
        return Some(
            "hermit panicked on a PMU skid overshoot: the preemption landed late, so this schedule is not reproducible; raise `skid_margin`"
                .to_string(),
        );
    }
    None
}

/// The facts of one trial. The lines are read from the streams as they arrived, not from the kept text: a long stream
/// keeps only its two ends.
pub(crate) fn facts_of(t: &Trial, timeout_secs: u64) -> TrialFacts {
    let (out, err) = (t.stdout_findings(), t.stderr_findings());
    TrialFacts {
        timeout_secs,
        timed_out: t.timed_out,
        exit_code: t.exit_code,
        signal: t.signal,
        backend_failure: own_failure(&err),
        config_fault: match &t.events_error {
            Some(EventsFault::Config(problem)) => Some(problem.clone()),
            _ => None,
        },
        collection_fault: match &t.events_error {
            Some(EventsFault::Infra(problem)) => Some(problem.clone()),
            _ => None,
        },
        property_failures: property_verdict::failures(t),
        marker_line: out.violation_line.or(err.violation_line),
        deadlock: err.deadlock_line.map(|line| DeadlockReport {
            reporter: REPORTER.to_string(),
            line,
        }),
        unusable_evidence: property_verdict::unusable_events(t),
    }
}

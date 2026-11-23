//! What one collected trial means. Anything wrong with the host or the tooling is an infrastructure error and never a
//! finding about the program. What is left is declared as facts for the engine's verdict ladder, which decides.
use crate::director::{ControlError, ResumeResult};
use crate::error::{InfraError, InfraKind};
use crate::serial::Serial;
use crate::session::Ended;
use crate::trial::TrialRun;
use hairspring_core::point::encoding::sha256_hex;
use hairspring_engine::verdict::{is_marker_line, judge, TrialFacts, Verdict};
use hairspring_props::limits::EVENT_PREFIX;
use hairspring_props::{TrialEvents, TrialProps};

/// A judged trial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    pub verdict: Verdict,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

fn marker_line(serial: &Serial) -> Option<String> {
    serial
        .lines
        .iter()
        .filter_map(|l| std::str::from_utf8(l).ok())
        .find(|l| is_marker_line(l))
        .map(str::to_string)
}

fn props_of(serial: &Serial) -> TrialProps {
    TrialEvents::from_lines(serial.lines.iter().map(Vec::as_slice)).fold()
}

fn unusable(serial: &Serial, props: &TrialProps) -> Option<String> {
    if serial.truncated {
        return Some("the serial log was longer than the size read".to_string());
    }
    if props.is_degraded() {
        return Some("guest events were lost or contradictory".to_string());
    }
    let missing = props.report_missing();
    (!missing.is_empty()).then(|| {
        format!(
            "process {} sent no end report (cut off, or it ended before reporting)",
            missing.join(", ")
        )
    })
}

/// The infrastructure failure the control channel and the VM's end show, if any.
fn infra_of(run: &TrialRun) -> Option<InfraError> {
    let c = &run.control;
    if !c.connected {
        return Some(InfraError::bare(InfraKind::NoConnect));
    }
    if c.ready == 0 {
        return Some(InfraError::bare(InfraKind::NoReady));
    }
    if c.stopped > 0 {
        return Some(InfraError::bare(InfraKind::StoppedAgain));
    }
    if let Some(ControlError::Lost(why)) = &c.error {
        return Some(InfraError::new(InfraKind::ControlLost, why.clone()));
    }
    if let Some(ControlError::Wire(e)) = &c.error {
        return Some(InfraError::new(InfraKind::ControlLost, e.to_string()));
    }
    match &c.resume {
        ResumeResult::Seen => {}
        ResumeResult::Unseen(why) => {
            return Some(InfraError::new(InfraKind::ResumeNotTaken, why.clone()))
        }
        ResumeResult::NotSent => return Some(InfraError::bare(InfraKind::ResumeNotTaken)),
    }
    if matches!(run.ended, Ended::Breach(_)) {
        return Some(InfraError::bare(InfraKind::LogBound));
    }
    // The VM ended without the guest's exit report and its log shows a control stop after the connect line: it stopped
    // again after it was resumed. Whether that was the branch limit is the likely case and is not shown.
    if let Some(stop) = run.stop_after_resume {
        return Some(InfraError::new(
            InfraKind::StoppedAgain,
            format!("stop kind {}", stop.kind),
        ));
    }
    match &run.serial {
        Some(s) if !s.lines.is_empty() => None,
        _ => Some(InfraError::bare(InfraKind::SerialMissing)),
    }
}

/// Judges `run`. `timeout_secs` is the time the trial was allowed, for the text of a hang.
pub fn judge_trial(
    run: &TrialRun,
    timeout_secs: u64,
    hang_is_violation: bool,
) -> Result<Judged, InfraError> {
    if let Some(e) = infra_of(run) {
        return Err(e);
    }
    let serial = run
        .serial
        .as_ref()
        .ok_or_else(|| InfraError::bare(InfraKind::SerialMissing))?;
    let timed_out = serial.exit.is_none() && run.ended == Ended::Cap;
    if serial.exit.is_none() && !timed_out {
        return Err(InfraError::bare(InfraKind::NoExitStatus));
    }
    let props = props_of(serial);
    let facts = TrialFacts {
        timeout_secs,
        timed_out,
        exit_code: serial.exit.and_then(|e| e.code),
        signal: serial.exit.and_then(|e| e.signal),
        property_failures: if timed_out {
            Vec::new()
        } else {
            props.verdict()
        },
        marker_line: marker_line(serial),
        unusable_evidence: unusable(serial, &props),
        ..TrialFacts::default()
    };
    Ok(Judged {
        verdict: judge(&facts).with_hang_policy(hang_is_violation),
        exit_code: facts.exit_code,
        signal: facts.signal,
    })
}

/// What a trial showed, as counts and digests, for a session summary. The digests say whether two trials' serial output
/// and event lines were equal; they say nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub resume_seen: bool,
    pub exit_reported: bool,
    pub serial_digest: Option<String>,
    pub events_digest: Option<String>,
}

pub fn observe(run: &TrialRun) -> Observed {
    let digest = |keep: &dyn Fn(&[u8]) -> bool| {
        run.serial.as_ref().map(|s| {
            let kept: Vec<u8> = s
                .lines
                .iter()
                .filter(|l| keep(l))
                .flat_map(|l| l.iter().copied().chain(std::iter::once(b'\n')))
                .collect();
            sha256_hex(&kept)
        })
    };
    Observed {
        resume_seen: run.control.resume == ResumeResult::Seen,
        exit_reported: run.serial.as_ref().is_some_and(|s| s.exit.is_some()),
        serial_digest: digest(&|_| true),
        events_digest: run
            .serial
            .as_ref()
            .is_some_and(|s| {
                s.lines
                    .iter()
                    .any(|l| l.starts_with(EVENT_PREFIX.as_bytes()))
            })
            .then(|| digest(&|l| l.starts_with(EVENT_PREFIX.as_bytes())))
            .flatten(),
    }
}

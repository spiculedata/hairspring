//! Validation and sanitising of results that come back from workers. A worker is a
//! separate (possibly remote, possibly compromised) process, and its text is later shown
//! to people and to AI agents, so every field is bounded and stripped of control characters.
use crate::{ServiceError, MAX_BUNDLE_BYTES};
use hairspring_core::bundle::Bundle;
use hairspring_engine::report::{FailureReport, ReplayOutcome, RunOutcome, Step};

pub const MAX_STEPS: usize = 1000;
pub const MAX_LABEL_CHARS: usize = 500;
pub const MAX_STATE_CHARS: usize = 4000;
pub const MAX_MESSAGE_CHARS: usize = 500;
pub const MAX_ACTION_ID_CHARS: usize = 64;

fn invalid(what: &str) -> ServiceError {
    ServiceError::InvalidRequest(what.to_string())
}

fn bounded(text: &str, max: usize, field: &str) -> Result<(), ServiceError> {
    if text.chars().count() > max {
        return Err(invalid(&format!("{field} exceeds {max} characters")));
    }
    Ok(())
}

/// Single-line text: every control character (including newlines) becomes a space.
fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Multi-line text: newlines and tabs survive, other control characters become spaces.
fn multi_line(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && !matches!(c, '\n' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Replay output comes from running target code, so it is cut to size and stripped of control characters
/// (rather than rejected) before anyone or any agent reads it.
pub fn replay_steps(out: ReplayOutcome) -> ReplayOutcome {
    let steps = out
        .steps
        .into_iter()
        .take(MAX_STEPS)
        .map(|s| Step {
            index: s.index,
            action_id: single_line(&s.action_id)
                .chars()
                .take(MAX_ACTION_ID_CHARS)
                .collect(),
            label: single_line(&s.label)
                .chars()
                .take(MAX_LABEL_CHARS)
                .collect(),
            state: multi_line(&s.state).chars().take(MAX_STATE_CHARS).collect(),
        })
        .collect();
    ReplayOutcome {
        reproduced: out.reproduced,
        steps,
    }
}

fn clean_step(step: &Step) -> Result<Step, ServiceError> {
    bounded(&step.label, MAX_LABEL_CHARS, "step label")?;
    bounded(&step.state, MAX_STATE_CHARS, "step state")?;
    bounded(&step.action_id, MAX_ACTION_ID_CHARS, "step action id")?;
    Ok(Step {
        index: step.index,
        action_id: single_line(&step.action_id),
        label: single_line(&step.label),
        state: multi_line(&step.state),
    })
}

fn clean_failure(f: &FailureReport, run_target: &str) -> Result<FailureReport, ServiceError> {
    if f.target != run_target {
        return Err(invalid("failure target does not match the run's target"));
    }
    if f.steps.len() > MAX_STEPS {
        return Err(invalid(&format!(
            "a failure may have at most {MAX_STEPS} steps"
        )));
    }
    if f.bundle.len() > MAX_BUNDLE_BYTES {
        return Err(invalid("bundle is too large"));
    }
    Bundle::decode(&f.bundle).map_err(|e| invalid(&format!("bundle is not valid: {e}")))?;
    bounded(&f.message, MAX_MESSAGE_CHARS, "failure message")?;
    bounded(&f.backend, MAX_ACTION_ID_CHARS, "backend name")?;
    let steps = f
        .steps
        .iter()
        .map(clean_step)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FailureReport {
        target: f.target.clone(),
        backend: single_line(&f.backend),
        message: single_line(&f.message),
        original_len: f.original_len,
        shrunk_len: f.shrunk_len,
        steps,
        bundle: f.bundle.clone(),
    })
}

/// Returns a bounded, sanitised copy of `outcome`, or an error if it breaks the limits.
pub fn outcome(outcome: &RunOutcome, run_target: &str) -> Result<RunOutcome, ServiceError> {
    if outcome.inconclusive > outcome.trials || outcome.distinct_schedules > outcome.trials {
        return Err(ServiceError::InvalidRequest(
            "inconclusive trials and distinct schedules cannot exceed the number of trials"
                .to_string(),
        ));
    }
    if let Some(e) = &outcome.error {
        bounded(e, MAX_MESSAGE_CHARS, "error text")?;
    }
    let failure = outcome
        .failure
        .as_ref()
        .map(|f| clean_failure(f, run_target))
        .transpose()?;
    Ok(RunOutcome {
        error: outcome.error.as_deref().map(single_line),
        failure,
        ..outcome.clone()
    })
}

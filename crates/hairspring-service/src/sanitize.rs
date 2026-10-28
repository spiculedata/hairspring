//! Validation and sanitising of results that come back from workers. A worker is a
//! separate (possibly remote, possibly compromised) process, and its text is later shown
//! to people and to AI agents, so every field is bounded and stripped of control characters.
use crate::{ServiceError, MAX_BUNDLE_BYTES};
use hairspring_core::bundle::Bundle;
use hairspring_engine::report::{FailureReport, ReplayOutcome, RunOutcome, RunStatus, Step};
use hairspring_engine::sweep::MAX_FAILURE_ROWS;

pub const MAX_STEPS: usize = 1000;
pub const MAX_LABEL_CHARS: usize = 500;
pub const MAX_STATE_CHARS: usize = 4000;
pub const MAX_MESSAGE_CHARS: usize = 500;
pub const MAX_ACTION_ID_CHARS: usize = 64;
pub const MAX_WARNINGS: usize = 20;
/// An environment warning names every changed field, so it is longer than a step label.
pub const MAX_WARNING_CHARS: usize = 2000;

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
        .map(|c| {
            if hairspring_core::environment::is_unsafe_display_char(c) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// An error message that is safe to show a caller: one line, no control or bidirectional characters, cut to size.
pub fn public_text(text: &str) -> String {
    single_line(text).chars().take(MAX_MESSAGE_CHARS).collect()
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
    let warnings = out
        .warnings
        .iter()
        .take(MAX_WARNINGS)
        .map(|w| single_line(w).chars().take(MAX_WARNING_CHARS).collect())
        .collect();
    ReplayOutcome {
        reproduced: out.reproduced,
        steps,
        warnings,
        environment_changed: out.environment_changed,
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

fn seed_of(failure: &FailureReport) -> Result<u64, ServiceError> {
    Bundle::decode(&failure.bundle)
        .map(|b| crate::confirm_claims::seed_of(&b))
        .map_err(|e| invalid(&format!("bundle is not valid: {e}")))
}

/// Checks the further failures of a result (see [`RunOutcome::more_failures`]): there are none unless the run is a
/// sweep and has a first failure, their seeds rise strictly from the first failure's, and the bundles of all of them
/// fit in one request. An outcome with none always fits.
pub fn check_more_failures(outcome: &RunOutcome, is_sweep: bool) -> Result<(), ServiceError> {
    if outcome.more_failures.is_empty() {
        return Ok(());
    }
    let Some(first) = &outcome.failure else {
        return Err(invalid("further failures need a first failure"));
    };
    if !is_sweep {
        return Err(invalid("only a sweep reports further failures"));
    }
    let mut last = seed_of(first)?;
    for f in &outcome.more_failures {
        let seed = seed_of(f)?;
        if seed <= last {
            return Err(invalid(
                "further failures must have distinct seeds, rising after the first failure's",
            ));
        }
        last = seed;
    }
    let total: usize = outcome
        .failure
        .iter()
        .chain(&outcome.more_failures)
        .map(|f| f.bundle.len())
        .sum();
    if total > crate::MAX_RUN_BUNDLE_BYTES {
        return Err(invalid(&format!(
            "the bundles of one run's failures total more than {} bytes",
            crate::MAX_RUN_BUNDLE_BYTES
        )));
    }
    Ok(())
}

/// Returns a bounded, sanitised copy of `outcome`, or an error if it breaks the limits.
pub fn outcome(outcome: &RunOutcome, run_target: &str) -> Result<RunOutcome, ServiceError> {
    if outcome.inconclusive > outcome.trials || outcome.distinct_schedules > outcome.trials {
        return Err(ServiceError::InvalidRequest(
            "inconclusive trials and distinct schedules cannot exceed the number of trials"
                .to_string(),
        ));
    }
    // A sweep's own summary is not read (the server counts the rows it holds), but a worker that sends one that
    // contradicts itself or its own trial count is sending data that cannot be right, so the result is refused.
    if let Some(s) = &outcome.sweep {
        if !s.is_consistent() || s.trials != outcome.trials {
            return Err(invalid(
                "the sweep summary contradicts itself or the number of trials",
            ));
        }
    }
    if let Some(e) = &outcome.error {
        bounded(e, MAX_MESSAGE_CHARS, "error text")?;
    }
    let failure = outcome
        .failure
        .as_ref()
        .map(|f| clean_failure(f, run_target))
        .transpose()?;
    if outcome.more_failures.len() >= MAX_FAILURE_ROWS {
        return Err(invalid(&format!(
            "a run may report at most {MAX_FAILURE_ROWS} failures"
        )));
    }
    let more_failures = outcome
        .more_failures
        .iter()
        .map(|f| clean_failure(f, run_target))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RunOutcome {
        error: outcome.error.as_deref().map(single_line),
        failure,
        more_failures,
        // Derived from the status, as the stored flag is: a worker's own claim is not read.
        stopped_at_first_failure: outcome.status == RunStatus::Failed,
        // A worker's own property statistics are never read: the server folds the per-trial digests it stored.
        // Trial records arrive through their own checked path, not inside an outcome.
        trial_log: Vec::new(),
        // The server's own summary is counted from the stored rows when a run is read.
        sweep: None,
        ..outcome.clone().without_properties()
    })
}

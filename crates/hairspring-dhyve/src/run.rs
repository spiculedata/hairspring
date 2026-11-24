//! `run` for the trial shape only: one prepare, then one restore for each seed, each judged by the engine's ladder. An
//! infrastructure failure ends the run with an error and is never counted as a finding. Replay, bundles that can be
//! replayed, branching and sweeps are not part of this stage.
use crate::error::InfraError;
use crate::fwcfg::{Inputs, GUEST_JOB};
use crate::judge::{judge_trial, Judged};
use crate::session::Session;
use crate::BACKEND_ID;
use hairspring_core::bundle::Bundle;
use hairspring_engine::backend::{Progress, RunRequest};
use hairspring_engine::config::{Budget, TargetKind, TargetSpec};
use hairspring_engine::report::{FailureReport, RunOutcome};
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_engine::verdict::Verdict;
use hairspring_engine::EngineError;
use std::time::{Duration, Instant};

/// The longest a trial may run until a log write rate has been measured on a host.
pub const MAX_TRIAL_SECS: u64 = 100;
const BUILD: &str = concat!("dhyve-", env!("CARGO_PKG_VERSION"));
const CANCELLED: &str = "cancelled";

/// Removes a checkpoint directory when the run ends, on every path.
struct RemoveOnDrop(std::path::PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Tally {
    trials: u64,
    inconclusive: u64,
    log: Vec<TrialReport>,
}

fn reported(verdict: &Verdict) -> (ReportedVerdict, String) {
    match verdict {
        Verdict::Pass => (ReportedVerdict::Pass, String::new()),
        Verdict::Violation(r) | Verdict::Deadlock(r) => (ReportedVerdict::Violation, r.clone()),
        Verdict::Hang(r) => (ReportedVerdict::Inconclusive, r.clone()),
        Verdict::Inconclusive(r) => (ReportedVerdict::Unusable, r.clone()),
        Verdict::Infra(r) => (ReportedVerdict::Infra, r.clone()),
    }
}

fn failure(spec: &TargetSpec, seed: u64, message: &str, checkpoint: &[String; 3]) -> FailureReport {
    let bundle = Bundle::new(seed, BUILD, Vec::new())
        .with_target(spec.name.clone())
        .with_backend(BACKEND_ID)
        .with_param("checkpoint_sha256", checkpoint.join(","));
    FailureReport {
        target: spec.name.clone(),
        backend: BACKEND_ID.to_string(),
        message: message.to_string(),
        original_len: 0,
        shrunk_len: 0,
        steps: Vec::new(),
        bundle: bundle.encode(),
    }
}

fn errored(e: &InfraError, tally: &Tally, started: Instant) -> RunOutcome {
    RunOutcome::error(e.to_string())
        .with_trials_attempted(tally.trials)
        .with_inconclusive(tally.inconclusive)
        .with_trial_log(tally.log.clone())
        .with_elapsed_ms(started.elapsed().as_millis() as u64)
}

/// Runs `budget.max_trials` trials, or until the wall budget or (in search mode) the first finding.
pub(crate) fn run_trials(
    session: &Session,
    spec: &TargetSpec,
    req: &RunRequest,
    budget: Budget,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<RunOutcome, EngineError> {
    // The target's `hang_is_violation` is not passed on: a trial with no exit report before its cap is inconclusive here,
    // because what a stop that left no exit report means has not been shown on a host.
    let TargetKind::Process {
        trial_timeout_secs, ..
    } = &spec.kind
    else {
        return Err(EngineError::BackendUnavailable(
            spec.kind.name().to_string(),
        ));
    };
    let timeout_secs = trial_timeout_secs
        .unwrap_or(MAX_TRIAL_SECS)
        .min(MAX_TRIAL_SECS);
    let session = session.with_trial_cap(Duration::from_secs(timeout_secs));
    let started = Instant::now();
    let mut tally = Tally {
        trials: 0,
        inconclusive: 0,
        log: Vec::new(),
    };
    let inputs = Inputs {
        seed: req.seed,
        job: GUEST_JOB,
    };
    let prepared = match session.prepare(inputs) {
        Ok(p) => p,
        Err(e) => return Ok(errored(&e, &tally, started)),
    };
    // The checkpoint is 256 MiB, and nothing else removes it.
    let _checkpoint = (!session.keep_all).then(|| RemoveOnDrop(prepared.record.dir.clone()));
    let wall = Duration::from_secs(budget.wall_secs);
    for ordinal in 0..budget.max_trials {
        if started.elapsed() >= wall {
            break;
        }
        let keep_going = progress(Progress {
            steps: tally.trials,
            trials: tally.trials,
            message: format!("trial {}", tally.trials + 1),
        });
        if !keep_going {
            return Ok(RunOutcome::error(CANCELLED));
        }
        let seed = req.seed.wrapping_add(ordinal);
        let ordinal32 = u32::try_from(ordinal).unwrap_or(u32::MAX);
        let trial_started = Instant::now();
        let trial = match session.restore_trial(&prepared, ordinal32, inputs.with_seed(seed)) {
            Ok(t) => t,
            Err(e) => return Ok(errored(&e, &tally, started)),
        };
        let judged = judge_trial(&trial, timeout_secs, false);
        session.clean_trial(
            &trial,
            !matches!(
                &judged,
                Ok(Judged {
                    verdict: Verdict::Pass,
                    ..
                })
            ),
        );
        let judged = match judged {
            Ok(j) => j,
            Err(e) => return Ok(errored(&e, &tally, started)),
        };
        let (kind, reason) = reported(&judged.verdict);
        let elapsed_ms = trial_started.elapsed().as_millis() as u64;
        tally.log.push(TrialReport::without_events(
            tally.trials,
            seed,
            kind,
            &reason,
            elapsed_ms,
        ));
        tally.trials += 1;
        match &judged.verdict {
            Verdict::Hang(_) | Verdict::Inconclusive(_) => tally.inconclusive += 1,
            Verdict::Violation(message) | Verdict::Deadlock(message) => {
                let failed = failure(spec, seed, message, &prepared.record.hashes);
                let elapsed = started.elapsed().as_millis() as u64;
                return Ok(
                    RunOutcome::failed(tally.trials, tally.trials, elapsed, failed)
                        .with_inconclusive(tally.inconclusive)
                        .with_trial_log(tally.log),
                );
            }
            Verdict::Pass | Verdict::Infra(_) => {}
        }
    }
    let elapsed = started.elapsed().as_millis() as u64;
    Ok(RunOutcome::passed(tally.trials, tally.trials, elapsed)
        .with_inconclusive(tally.inconclusive)
        .with_trial_log(tally.log))
}

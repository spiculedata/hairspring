//! One run of a confirmation, with the retry a skid overshoot gets.
//!
//! Hermit panics when a preemption lands late (a PMU skid overshoot) if the target set a margin. That run is
//! infrastructure, not a verdict about the program. It is retried once at the same flags, and then once at double the
//! skid margin. A run at a higher margin is a different schedule: the caller records it, with the caveat, and a check
//! that needs the original schedule (the reruns) does not count it. Each run is given what is left of the wall budget,
//! and the caller's cancel is heard between runs, so up to three runs never outlast either.
use super::pace::Pace;
use crate::backend::HermitBackend;
use crate::exec::{ProcessSpec, SpawnError, Trial};
use crate::trace::{skid_overshoot, Verdict};
use std::time::Duration;

/// The run that was kept and how it came about.
pub(super) struct Attempt {
    pub trial: Result<Trial, SpawnError>,
    /// Runs that overshot and were repeated at the same flags (0 or 1).
    pub retries: u32,
    /// The kept run used double the skid margin.
    pub doubled: bool,
    /// The time the kept run was given: its own limit, or what was left of the budget when that was less.
    pub timeout: Duration,
}

fn overshot(
    backend: &HermitBackend,
    p: &ProcessSpec,
    trial: &Result<Trial, SpawnError>,
    timeout: Duration,
) -> bool {
    trial.as_ref().is_ok_and(|t| {
        matches!(backend.verdict(p, t, timeout), Verdict::Infra(_)) && skid_overshoot(t)
    })
}

pub(super) fn run(
    backend: &HermitBackend,
    p: &ProcessSpec,
    seed: u64,
    own_limit: Duration,
    pace: &mut Pace,
) -> Attempt {
    let timeout = pace.run_timeout(own_limit);
    let first = backend.trial(p, seed, timeout);
    if !overshot(backend, p, &first, timeout) {
        return Attempt {
            trial: first,
            retries: 0,
            doubled: false,
            timeout,
        };
    }
    // Every run is a point where the caller may cancel and the wall budget is checked: a retry is not started once
    // either says stop, and the run that overshot is what is reported (as infrastructure, which is what it was).
    pace.ran("skid overshoot".to_string());
    if pace.stop().is_some() {
        return Attempt {
            trial: first,
            retries: 0,
            doubled: false,
            timeout,
        };
    }
    let timeout = pace.run_timeout(own_limit);
    let second = backend.trial(p, seed, timeout);
    if !overshot(backend, p, &second, timeout) {
        return Attempt {
            trial: second,
            retries: 1,
            doubled: false,
            timeout,
        };
    }
    pace.ran("skid overshoot".to_string());
    if pace.stop().is_some() {
        return Attempt {
            trial: second,
            retries: 1,
            doubled: false,
            timeout,
        };
    }
    let timeout = pace.run_timeout(own_limit);
    match p.scheduler.with_doubled_skid() {
        Some(doubled) => {
            let raised = ProcessSpec {
                scheduler: &doubled,
                ..*p
            };
            Attempt {
                trial: backend.trial(&raised, seed, timeout),
                retries: 1,
                doubled: true,
                timeout,
            }
        }
        None => Attempt {
            trial: second,
            retries: 1,
            doubled: false,
            timeout,
        },
    }
}

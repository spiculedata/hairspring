//! How the Hermit backend schedules threads for a `process` target.
//!
//! The scheduling strategy decides which interleavings a seed can reach, so it is part of the target definition and is
//! recorded in every bundle. Defaults reproduce the original behaviour: uniform random scheduling driven by the seed.
use serde::{Deserialize, Serialize};

/// Hermit flags the backend sets itself. Passing them again in `hermit_args` is either a usage error in Hermit or a
/// silent change to the schedule that replay's check would not see.
const OWNED_HERMIT_FLAGS: [&str; 15] = [
    "--chaos",
    "--sched-heuristic",
    "--sched-seed",
    "--sched-sticky-random-param",
    "--max-timeslice",
    "--preemption-timeout",
    "--skid-margin",
    "--epoch",
    "--summary-json",
    "--summary",
    "--seed",
    "--tmp",
    "--verify",
    "--panic-on-rbc-overshoot",
    "--panic-on-rcb-overshoot",
];

/// Smallest timeslice Hermit accepts: one retired conditional branch (10 virtual nanoseconds).
const MIN_TIMESLICE_NS: u64 = 10;
/// Hermit caps the PMU skid margin at this many retired conditional branches.
const MAX_SKID_MARGIN: u32 = 10_000;
const MAX_PERMILLE: u16 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Heuristic {
    /// Pick uniformly among runnable threads at every scheduling decision.
    Random,
    /// Stay on the current thread with a configured probability, otherwise pick randomly.
    StickyRandom,
    /// No heuristic: Hermit's own default policy (what `chaos` builds on).
    None,
}

impl Heuristic {
    /// The value Hermit's `--sched-heuristic` flag expects; `None` means the flag is omitted.
    pub fn hermit_value(self) -> Option<&'static str> {
        match self {
            Heuristic::Random => Some("random"),
            Heuristic::StickyRandom => Some("stickyrandom"),
            Heuristic::None => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Heuristic::Random => "random",
            Heuristic::StickyRandom => "sticky-random",
            Heuristic::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct SchedulerSpec {
    /// Explicit heuristic. When absent: `random`, or no heuristic when `chaos` is on.
    pub heuristic: Option<Heuristic>,
    /// Probability (in thousandths) of staying on the current thread; only with `sticky-random`.
    pub sticky_permille: Option<u16>,
    /// Randomise thread priorities at every timeslice (Hermit `--chaos`).
    pub chaos: bool,
    /// Longest a thread runs before preemption, in virtual nanoseconds (Hermit `--max-timeslice`).
    pub max_timeslice_ns: Option<u64>,
    /// PMU skid margin in retired conditional branches (Hermit `--skid-margin`).
    pub skid_margin: Option<u32>,
}

impl SchedulerSpec {
    pub fn effective_heuristic(&self) -> Heuristic {
        match (self.heuristic, self.chaos) {
            (Some(h), _) => h,
            (None, true) => Heuristic::None,
            (None, false) => Heuristic::Random,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let heuristic = self.effective_heuristic();
        if self.chaos && heuristic == Heuristic::Random {
            return Err(
                "`chaos` cannot be combined with the `random` heuristic (random ignores the priorities chaos sets)"
                    .to_string(),
            );
        }
        if self.heuristic == Some(Heuristic::None) && !self.chaos {
            return Err(
                "`heuristic = \"none\"` needs `chaos = true`; otherwise the seed has no effect on scheduling"
                    .to_string(),
            );
        }
        match (self.sticky_permille, heuristic) {
            (Some(_), h) if h != Heuristic::StickyRandom => {
                return Err("`sticky_permille` needs `heuristic = \"sticky-random\"`".to_string())
            }
            (Some(p), _) if p > MAX_PERMILLE => {
                return Err(format!("`sticky_permille` must be 0-{MAX_PERMILLE}"))
            }
            _ => {}
        }
        if let Some(m) = self.skid_margin {
            if m == 0 || m > MAX_SKID_MARGIN {
                return Err(format!("`skid_margin` must be 1-{MAX_SKID_MARGIN}"));
            }
        }
        if let Some(t) = self.max_timeslice_ns {
            if t < MIN_TIMESLICE_NS {
                return Err(format!(
                    "`max_timeslice_ns` must be at least {MIN_TIMESLICE_NS}"
                ));
            }
        }
        Ok(())
    }

    /// True when any option differs from the default (used to describe a target).
    pub fn is_default(&self) -> bool {
        *self == SchedulerSpec::default()
    }
}

/// Rejects `hermit_args` entries that name a flag the backend owns.
pub fn check_hermit_args(args: &[String]) -> Result<(), String> {
    for arg in args {
        let name = arg.split('=').next().unwrap_or(arg);
        if OWNED_HERMIT_FLAGS.contains(&name) {
            return Err(format!(
                "`hermit_args` must not contain `{name}`: Hairspring sets it from the scheduler options and the run's seed"
            ));
        }
    }
    Ok(())
}

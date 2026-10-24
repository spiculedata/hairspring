//! How long a confirmation may take, and whether its caller still wants it.
use hairspring_engine::backend::Progress;
use std::time::{Duration, Instant};

/// A run is never given less than this, so a budget nearly used up cannot kill a trial before it starts.
const MIN_RUN: Duration = Duration::from_secs(1);

/// Why a confirmation stopped before every check was done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    Cancelled,
    WallBudget,
}

impl Stop {
    pub(super) fn text(self) -> &'static str {
        match self {
            Stop::Cancelled => "cancelled",
            Stop::WallBudget => "wall budget exhausted",
        }
    }
}

pub(super) struct Pace<'a> {
    started: Instant,
    wall: Duration,
    progress: &'a mut dyn FnMut(Progress) -> bool,
    runs: u64,
    cancelled: bool,
}

impl<'a> Pace<'a> {
    pub(super) fn new(wall: Duration, progress: &'a mut dyn FnMut(Progress) -> bool) -> Pace<'a> {
        Pace {
            started: Instant::now(),
            wall,
            progress,
            runs: 0,
            cancelled: false,
        }
    }

    /// Why no further run should start, if it should not.
    pub(super) fn stop(&self) -> Option<Stop> {
        if self.cancelled {
            Some(Stop::Cancelled)
        } else if self.started.elapsed() >= self.wall {
            Some(Stop::WallBudget)
        } else {
            None
        }
    }

    /// The longest the next run may take: its own limit, or what is left of the budget when that is less.
    pub(super) fn run_timeout(&self, own_limit: Duration) -> Duration {
        let left = self.wall.saturating_sub(self.started.elapsed());
        own_limit.min(left.max(MIN_RUN))
    }

    /// Reports that one more run finished. A caller that asks to stop is heard at the next [`Pace::stop`].
    pub(super) fn ran(&mut self, message: String) {
        self.runs += 1;
        let keep_going = (self.progress)(Progress {
            steps: self.runs,
            trials: self.runs,
            message,
        });
        self.cancelled = self.cancelled || !keep_going;
    }
}

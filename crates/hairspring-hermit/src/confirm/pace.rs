//! How long a confirmation may take, and whether its caller still wants it.
use hairspring_engine::backend::Progress;
use std::time::{Duration, Instant};

/// How long a run that may have been cut short is given for the caller's stop to be heard, and how often it is asked.
/// A service manager signals the server and its Hermit processes together, so the run can end a moment before the
/// stop reaches the caller.
const STOP_GRACE: Duration = Duration::from_millis(500);
const STOP_POLL: Duration = Duration::from_millis(50);

/// The text a run cut short by the caller's stop is recorded with.
pub(super) const CUT_BY_STOP: &str =
    "stopped: cancelled while this run was in progress, so it says nothing";

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

    /// Whether the caller asked to stop by the time the run that has just ended was over, as [`Pace::ran`] heard it
    /// (call this after `ran`). A run that ended with the stop is not a result about the program (its process may have
    /// been killed by the same stop), so the caller records it inconclusive, never as a miss. `may_be_cut` is true when
    /// the run's end looks like a kill or a miss, and then the stop is waited for a moment, because it can arrive just
    /// after the run does; a run that ended well gets no wait and costs the caller no extra call. A run that finished
    /// well, and matched, is still evidence and is kept by the callers whatever this says.
    pub(super) fn stop_arrived(&mut self, may_be_cut: bool) -> bool {
        if self.cancelled {
            return true;
        }
        if may_be_cut {
            let until = Instant::now() + STOP_GRACE;
            while Instant::now() < until {
                std::thread::sleep(STOP_POLL);
                if self.asked_to_stop() {
                    self.cancelled = true;
                    return true;
                }
            }
        }
        false
    }

    fn asked_to_stop(&mut self) -> bool {
        !(self.progress)(Progress {
            steps: self.runs,
            trials: self.runs,
            message: "checking whether to stop".to_string(),
        })
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

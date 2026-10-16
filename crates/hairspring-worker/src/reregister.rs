//! Pacing for re-registration. A worker the server forgets registers again, but a server that keeps rejecting fresh
//! credentials must not be answered with a tight register loop, so each consecutive re-registration waits longer.
use std::time::Duration;

const BASE_DELAY: Duration = Duration::from_millis(500);
const MAX_DELAY: Duration = Duration::from_secs(60);
/// Past this many doublings the delay is at the cap anyway; keeps the shift well inside `u32`.
const MAX_SHIFT: u32 = 16;

/// How many re-registrations in a row have happened without one successful lease in between.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReregisterBackoff {
    consecutive: u32,
}

impl ReregisterBackoff {
    /// How long to wait before the next re-registration.
    pub(crate) fn delay(self) -> Duration {
        let factor = 1u32 << self.consecutive.min(MAX_SHIFT);
        (BASE_DELAY * factor).min(MAX_DELAY)
    }

    /// The state after one more re-registration.
    pub(crate) fn after_attempt(self) -> ReregisterBackoff {
        ReregisterBackoff {
            consecutive: self.consecutive.saturating_add(1),
        }
    }

    /// The state after the server accepted the worker's credential again.
    pub(crate) fn reset(self) -> ReregisterBackoff {
        ReregisterBackoff::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_consecutive_reregistration_waits_twice_as_long() {
        let first = ReregisterBackoff::default();
        let second = first.after_attempt();
        let third = second.after_attempt();
        assert_eq!(first.delay(), BASE_DELAY);
        assert_eq!(second.delay(), BASE_DELAY * 2);
        assert_eq!(third.delay(), BASE_DELAY * 4);
    }

    #[test]
    fn the_wait_is_capped() {
        let mut state = ReregisterBackoff::default();
        for _ in 0..200 {
            state = state.after_attempt();
        }
        assert_eq!(state.delay(), MAX_DELAY);
    }

    #[test]
    fn an_accepted_credential_starts_the_pacing_over() {
        let worn = ReregisterBackoff::default()
            .after_attempt()
            .after_attempt()
            .after_attempt();
        assert_eq!(worn.reset(), ReregisterBackoff::default());
        assert_eq!(worn.reset().delay(), BASE_DELAY);
    }

    #[test]
    fn after_attempt_does_not_change_the_original() {
        let before = ReregisterBackoff::default();
        let _ = before.after_attempt();
        assert_eq!(before, ReregisterBackoff::default());
    }
}

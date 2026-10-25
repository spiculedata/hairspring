//! The one note a sweep prints when it did not simply run its trial budget, chosen from how the server says the run
//! ended (the same split the explorer's completion label uses), so the note and the rate lines cannot disagree.
use hairspring_service::RunCompletion;

/// The note for a finished sweep that ran `trials` of `budget` trials, or `None` when it needs none.
pub fn note(completion: &RunCompletion, trials: u64, budget: u64) -> Option<String> {
    let of = format!("{trials} of {budget} trials");
    match completion {
        RunCompletion::TimeOrStepLimited { .. } => Some(format!(
            "note: ended after {of}, before its trial budget: it reached its step or wall-clock limit (raise --budget-steps or --budget-secs to run more)"
        )),
        RunCompletion::EndedEarly => Some(format!(
            "note: stopped after {of} without reaching a limit the server can verify (a cancel, a worker that predates sweeps, or a worker memory limit); counts only, no rate is stated"
        )),
        // The label is the allowance's whenever a stand-in exists, so a short run needs saying: the server cannot tell
        // whether a limit was reached, and states no rate.
        RunCompletion::DigestAllowanceUsedUp if trials < budget => Some(format!(
            "note: stopped after {of}, and the server's digest allowance was also used up, so it cannot tell whether a limit was reached; counts only, no rate is stated"
        )),
        RunCompletion::DigestAllowanceUsedUp => Some(format!(
            "note: the server's digest allowance was used up, so property statistics are incomplete; the {of} that ran are counted by verdict and seed"
        )),
        RunCompletion::Complete
        | RunCompletion::Running
        | RunCompletion::StoppedAtFirstFailure
        | RunCompletion::Aborted => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_that_ran_its_budget_needs_no_note() {
        assert_eq!(note(&RunCompletion::Complete, 10, 10), None);
        assert_eq!(note(&RunCompletion::Aborted, 3, 10), None);
    }

    #[test]
    fn a_limit_the_server_verified_is_named_as_one() {
        let limited = RunCompletion::TimeOrStepLimited {
            trials_run: 1,
            trials_budget: 10,
        };
        let text = note(&limited, 1, 10).unwrap();
        assert!(
            text.contains("ended after 1 of 10 trials, before its trial budget"),
            "{text}"
        );
        assert!(
            text.contains("reached its step or wall-clock limit"),
            "{text}"
        );
        assert!(!text.contains("without reaching a limit"), "{text}");
    }

    #[test]
    fn a_stop_without_a_limit_says_so_and_says_no_rate_is_stated() {
        let text = note(&RunCompletion::EndedEarly, 3, 10).unwrap();
        assert!(
            text.contains("stopped after 3 of 10 trials without reaching a limit"),
            "{text}"
        );
        assert!(text.contains("no rate is stated"), "{text}");
        assert!(
            !text.contains("reached its step or wall-clock limit"),
            "{text}"
        );
    }

    #[test]
    fn capped_digests_on_a_short_sweep_say_no_rate_is_stated() {
        let text = note(&RunCompletion::DigestAllowanceUsedUp, 3, 10).unwrap();
        assert!(text.contains("stopped after 3 of 10 trials"), "{text}");
        assert!(text.contains("digest allowance was also used up"), "{text}");
        assert!(text.contains("no rate is stated"), "{text}");
    }

    #[test]
    fn capped_digests_are_not_called_an_early_stop() {
        let text = note(&RunCompletion::DigestAllowanceUsedUp, 10, 10).unwrap();
        assert!(text.contains("digest allowance was used up"), "{text}");
        assert!(!text.contains("without reaching a limit"), "{text}");
        assert!(!text.contains("ended after"), "{text}");
    }
}

//! What the server stores in place of a trial digest that would pass the run's digest allowance.
//!
//! The trial is still recorded: same sequence number, seed, verdict, counters, kept detail and output. Only the
//! digest, which is what the property fold and the coverage series read, is replaced by an empty one marked as
//! excluded, and the trial's reason starts with [`DIGEST_CAPPED_REASON`] so every reader can tell. The trial is not
//! rated, so it counts toward no rate, and the run finishes marked incomplete so no "nothing is missing" claim is made.
//!
//! The marker is text the worker cannot send: a reason that starts with it is refused at ingest, so a stored reason
//! that starts with it was written here.
use hairspring_props::{DigestFlags, TrialDigest};
use hairspring_store::{NewTrial, MAX_TRIAL_REASON_CHARS};

/// The start of the reason of a trial whose digest was replaced because the run's digest allowance was used up.
pub const DIGEST_CAPPED_REASON: &str =
    "digest not stored: the run's digest allowance is used up. Worker's reason: ";

/// Whether a stored reason is the one [`stand_in`] writes.
pub(crate) fn is_capped_reason(reason: &str) -> bool {
    reason.starts_with(DIGEST_CAPPED_REASON)
}

/// The digest that stands in for a dropped one: no events, excluded from every statistic.
fn excluded_digest() -> Vec<u8> {
    TrialDigest {
        flags: DigestFlags {
            excluded: true,
            ..DigestFlags::default()
        },
        ..TrialDigest::default()
    }
    .encode()
}

/// `trial` as it is stored when its digest does not fit: the same trial with an excluded digest and no ratings.
pub(crate) fn stand_in(trial: &NewTrial) -> NewTrial {
    let reason: String = format!("{DIGEST_CAPPED_REASON}{}", trial.reason)
        .chars()
        .take(MAX_TRIAL_REASON_CHARS)
        .collect();
    NewTrial {
        reason,
        rated: false,
        claimable: false,
        max_idx: None,
        digest: excluded_digest(),
        on_digest_cap: None,
        ..trial.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_store::TrialVerdict;

    fn trial(reason: &str) -> NewTrial {
        NewTrial {
            seq: 7,
            seed: 99,
            verdict: TrialVerdict::Violation,
            reason: reason.to_owned(),
            exit_code: Some(1),
            signal: None,
            timed_out: true,
            degraded: true,
            perturbing: true,
            report_missing: true,
            rated: true,
            claimable: true,
            behaviour_hash: Some("abc".into()),
            schedule_fp: None,
            syscalls: 5,
            work_ops: 6,
            elapsed_ms: 7,
            max_idx: Some(3),
            digest: vec![1; 400],
            detail: Some(b"{}".to_vec()),
            output: None,
            on_digest_cap: None,
        }
    }

    #[test]
    fn the_stand_in_is_the_same_trial_with_nothing_rated() {
        let t = trial("boom");
        let s = stand_in(&t);
        assert_eq!((s.seq, s.seed, s.verdict), (7, 99, TrialVerdict::Violation));
        assert_eq!(
            (s.exit_code, s.syscalls, s.work_ops, s.elapsed_ms),
            (Some(1), 5, 6, 7)
        );
        assert_eq!(s.detail, t.detail);
        assert_eq!(s.behaviour_hash, t.behaviour_hash);
        assert!(!s.rated && !s.claimable);
        assert!(
            s.timed_out && s.degraded && s.perturbing && s.report_missing,
            "what the worker reported about the trial is kept"
        );
        assert_eq!(s.max_idx, None);
        assert!(s.on_digest_cap.is_none());
    }

    #[test]
    fn the_stand_in_digest_is_a_valid_excluded_digest_with_no_entries() {
        let d = TrialDigest::decode(&stand_in(&trial("")).digest).unwrap();
        assert!(d.flags.excluded);
        assert!(d.entries.is_empty());
    }

    #[test]
    fn the_stand_in_reason_is_marked_keeps_the_workers_words_and_stays_within_the_limit() {
        let s = stand_in(&trial("boom"));
        assert!(is_capped_reason(&s.reason));
        assert!(s.reason.ends_with("boom"));
        let long = stand_in(&trial(&"x".repeat(MAX_TRIAL_REASON_CHARS)));
        assert_eq!(long.reason.chars().count(), MAX_TRIAL_REASON_CHARS);
        assert!(is_capped_reason(&long.reason));
    }

    #[test]
    fn a_reason_the_worker_wrote_is_not_the_marker() {
        assert!(!is_capped_reason("digest not stored"));
        assert!(!is_capped_reason(""));
    }

    #[test]
    fn the_marker_leaves_room_for_the_workers_reason() {
        assert!(DIGEST_CAPPED_REASON.chars().count() < MAX_TRIAL_REASON_CHARS / 2);
    }
}

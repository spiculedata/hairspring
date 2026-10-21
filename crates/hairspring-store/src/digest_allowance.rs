//! The per-run allowance of stored trial digests, and the other limits of the explorer's storage needs.
//!
//! A registered worker may report up to [`crate::MAX_TRIALS_PER_BATCH`] trials of up to
//! [`crate::MAX_DIGEST_BLOB_BYTES`] each, batch after batch, and every read of the run decodes those digests. The
//! allowance bounds what one run may hold. A digest that does not fit is not refused and not dropped: the caller
//! offers a smaller stand-in with each trial ([`crate::NewTrial::on_digest_cap`]), the store keeps that instead, and
//! counts the substitution so the run can say that it happened.
use crate::trials::{NewTrial, MAX_CATALOG_ROWS};

/// Default allowance of stored digest bytes for one run: 64 MiB.
///
/// This is a proposal, not a measured figure. A run holds at most 10,000 trials, so it leaves an average of about
/// 6.7 KiB per digest, and a digest of a few dozen properties is well under 1 KiB. The service reads it from its
/// configuration (`digest_bytes_per_run`), so changing it needs no schema change.
pub const MAX_DIGEST_BYTES_PER_RUN: u64 = 64 * 1024 * 1024;
/// Most seeds one [`crate::Store::reporters_of`] request may name (one failing seed per catalogued property at most).
pub const MAX_REPORTER_SEEDS: usize = MAX_CATALOG_ROWS;
/// Largest coverage series the store keeps with a run, in bytes.
pub const MAX_COVERAGE_JSON_BYTES: usize = 512 * 1024;

/// How much of a run's digest allowance is used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigestUse {
    /// Bytes of digest held by the run's trial rows (stand-ins included).
    pub bytes: u64,
    /// Trials whose own digest did not fit and was replaced by the stand-in.
    pub capped_trials: u64,
}

/// What to store for one new trial.
pub(crate) enum DigestForm<'a> {
    /// The trial as sent.
    Original(&'a NewTrial),
    /// The trial's stand-in, because its digest would pass the allowance.
    Replaced(&'a NewTrial),
    /// The digest does not fit and the caller offered no stand-in.
    Refused,
}

/// Chooses between a trial and its stand-in. `used` is what the run already holds.
pub(crate) fn digest_form(t: &NewTrial, used: u64, cap: u64) -> DigestForm<'_> {
    let needed = used.saturating_add(t.digest.len() as u64);
    if needed <= cap {
        return DigestForm::Original(t);
    }
    match t.on_digest_cap.as_deref() {
        Some(stand_in) => DigestForm::Replaced(stand_in),
        None => DigestForm::Refused,
    }
}

impl DigestUse {
    /// The use after one more trial was stored in the given form.
    pub(crate) fn after(self, stored: &NewTrial, replaced: bool) -> DigestUse {
        DigestUse {
            bytes: self.bytes.saturating_add(stored.digest.len() as u64),
            capped_trials: self.capped_trials + u64::from(replaced),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trials::TrialVerdict;

    fn trial(len: usize, stand_in: Option<NewTrial>) -> NewTrial {
        NewTrial {
            seq: 0,
            seed: 1,
            verdict: TrialVerdict::Pass,
            reason: String::new(),
            exit_code: None,
            signal: None,
            timed_out: false,
            degraded: false,
            perturbing: false,
            report_missing: false,
            rated: true,
            claimable: true,
            behaviour_hash: None,
            schedule_fp: None,
            syscalls: 0,
            work_ops: 0,
            elapsed_ms: 0,
            max_idx: None,
            digest: vec![0; len],
            detail: None,
            output: None,
            on_digest_cap: stand_in.map(Box::new),
        }
    }

    #[test]
    fn a_digest_that_fits_exactly_is_kept() {
        assert!(matches!(
            digest_form(&trial(4, None), 6, 10),
            DigestForm::Original(_)
        ));
    }

    #[test]
    fn a_digest_one_byte_over_is_replaced_when_a_stand_in_is_offered() {
        let t = trial(5, Some(trial(1, None)));
        assert!(matches!(digest_form(&t, 6, 10), DigestForm::Replaced(r) if r.digest.len() == 1));
    }

    #[test]
    fn a_digest_over_the_allowance_with_no_stand_in_is_refused() {
        assert!(matches!(
            digest_form(&trial(5, None), 6, 10),
            DigestForm::Refused
        ));
    }

    #[test]
    fn a_used_count_near_the_limit_of_the_type_does_not_wrap() {
        assert!(matches!(
            digest_form(&trial(5, None), u64::MAX - 1, u64::MAX - 2),
            DigestForm::Refused
        ));
    }

    #[test]
    fn use_counts_bytes_and_substitutions() {
        let stand_in = trial(2, None);
        let used = DigestUse::default()
            .after(&trial(4, None), false)
            .after(&stand_in, true);
        assert_eq!(
            used,
            DigestUse {
                bytes: 6,
                capped_trials: 1
            }
        );
    }
}

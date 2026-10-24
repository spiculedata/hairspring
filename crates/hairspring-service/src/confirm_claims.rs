//! What a run's stored confirmation checks say about its failing seeds, for the claim gate and the claim wording.
//!
//! A failure belongs to a seed through its bundle (the scheduling seed it recorded). Its checks give a state, and the
//! claim ladder sees only that state: nothing is "reproduced" unless the checks derive confirmed or verified.
use crate::confirm_view::standing;
use crate::{Result, Service};
use hairspring_core::bundle::Bundle;
use hairspring_engine::confirm::FindingStatus;
use std::collections::BTreeMap;

/// A failing seed's stored checks, reduced to what a claim needs.
pub(crate) struct CheckedFailure {
    pub status: FindingStatus,
    /// The checks ran in the environment the bundle recorded, and it still matched.
    pub pinned: bool,
    /// Who ran the newest check, and whether that runner was trusted when it reported.
    pub runner: String,
    pub runner_trusted: bool,
}

/// The scheduling seed a bundle records, read as a replay reads it.
fn seed_of(bundle: &Bundle) -> u64 {
    bundle
        .params
        .get("sched_seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(bundle.seed)
}

impl Service {
    /// The run's failures that have stored checks, by seed. A failure whose bundle cannot be read has no seed to be
    /// matched to and is left out, which leaves its seed unconfirmed.
    pub(crate) fn checked_failures(&self, run_id: &str) -> Result<BTreeMap<u64, CheckedFailure>> {
        let mut out = BTreeMap::new();
        for failure in self.store.list_failures(run_id)? {
            let records = self.store.list_failure_checks(&failure.id)?;
            let Some(newest) = records.last() else {
                continue;
            };
            let Some(text) = self.get_bundle(&failure.bundle_hash)? else {
                continue;
            };
            let Ok(bundle) = Bundle::decode(&text) else {
                continue;
            };
            let (status, pinned) = standing(&records)?;
            out.insert(
                seed_of(&bundle),
                CheckedFailure {
                    status,
                    pinned,
                    runner: newest.runner.clone(),
                    runner_trusted: newest.runner_trusted,
                },
            );
        }
        Ok(out)
    }
}

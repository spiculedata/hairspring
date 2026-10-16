//! When a rate may feed a claim.
//!
//! Folding digests on the server stops forged run-level figures, but a compromised or buggy worker can still invent
//! per-trial results that pass every size and consistency check. So a figure may back a headline or a generated claim
//! only if every trial behind it came from a trusted worker and every failure it rests on was reproduced by a trusted
//! confirmation replay. Anything else is still shown, labelled "unverified worker data", and never claimed.
use hairspring_store::Contributor;

/// One replay of a reported failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub replayer: String,
    pub replayer_trusted: bool,
    /// The replay failed the same properties with the same behaviour.
    pub reproduced: bool,
}

/// A failure a claim rests on, and the replays of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureEvidence {
    /// The worker that reported the failure.
    pub reporter: String,
    pub confirmations: Vec<Confirmation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimGate {
    pub allowed: bool,
    /// Why not, in words fit for a results page.
    pub reasons: Vec<&'static str>,
}

fn confirmed(failure: &FailureEvidence, trusted_workers: usize) -> bool {
    failure.confirmations.iter().any(|c| {
        c.reproduced
            && c.replayer_trusted
            // With more than one trusted worker, the reporter does not get to confirm its own report.
            && (trusted_workers <= 1 || c.replayer != failure.reporter)
    })
}

/// Decides whether a run's figures may back a claim. `trusted_workers` is how many trusted workers exist.
pub fn claim_gate(
    contributors: &[Contributor],
    failures: &[FailureEvidence],
    trusted_workers: usize,
) -> ClaimGate {
    let mut reasons = Vec::new();
    if contributors.is_empty() {
        reasons.push("no trial data was stored for this run");
    }
    if contributors.iter().any(|c| !c.trusted) {
        reasons.push("some trials came from a worker that is not trusted (unverified worker data)");
    }
    if failures.iter().any(|f| !confirmed(f, trusted_workers)) {
        reasons.push("a failure has no reproduction on a trusted worker");
    }
    ClaimGate {
        allowed: reasons.is_empty(),
        reasons,
    }
}

//! When a rate may feed a claim.
//!
//! Folding digests on the server stops forged run-level figures, but a compromised or buggy worker can still invent
//! per-trial results that pass every size and consistency check. So a figure may back a headline or a generated claim
//! only if every trial behind it came from a trusted worker and every failure it rests on was reproduced by a trusted
//! confirmation replay. Anything else is still shown, labelled "unverified worker data", and never claimed.
use hairspring_engine::claim_standing::backend_may_back_claims;
use hairspring_store::Contributor;

/// Why a run of an experimental backend is never claimed.
pub const EXPERIMENTAL_BACKEND: &str =
    "this backend is experimental, and no claim may rest on its runs";

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

/// [`claim_gate`], and refused outright for a backend whose runs may not back a claim, whatever its trials found and
/// whoever reported them.
pub fn claim_gate_for(
    backend_id: &str,
    contributors: &[Contributor],
    failures: &[FailureEvidence],
    trusted_workers: usize,
) -> ClaimGate {
    let gate = claim_gate(contributors, failures, trusted_workers);
    if backend_may_back_claims(backend_id) {
        return gate;
    }
    let mut reasons = gate.reasons;
    reasons.push(EXPERIMENTAL_BACKEND);
    ClaimGate {
        allowed: false,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted() -> Vec<Contributor> {
        vec![Contributor {
            worker_id: "w1".into(),
            trusted: true,
            trials: 10,
        }]
    }

    #[test]
    fn a_run_with_no_failures_from_trusted_workers_is_allowed_for_a_shipped_backend() {
        for backend in ["sim", "wasm", "hermit"] {
            let gate = claim_gate_for(backend, &trusted(), &[], 1);
            assert!(gate.allowed, "{backend}: {:?}", gate.reasons);
            assert_eq!(gate, claim_gate(&trusted(), &[], 1));
        }
    }

    #[test]
    fn the_same_run_is_refused_for_the_experimental_backend() {
        let gate = claim_gate_for("dhyve", &trusted(), &[], 1);
        assert!(!gate.allowed);
        assert_eq!(gate.reasons, vec![EXPERIMENTAL_BACKEND]);
    }

    #[test]
    fn the_refusal_is_added_to_the_reasons_the_gate_already_has() {
        let untrusted = vec![Contributor {
            trusted: false,
            ..trusted().remove(0)
        }];
        let gate = claim_gate_for("dhyve", &untrusted, &[], 1);
        assert_eq!(gate.reasons.len(), 2);
        assert!(gate.reasons.contains(&EXPERIMENTAL_BACKEND));
        assert!(!claim_gate_for("hermit", &untrusted, &[], 1)
            .reasons
            .contains(&EXPERIMENTAL_BACKEND));
    }
}

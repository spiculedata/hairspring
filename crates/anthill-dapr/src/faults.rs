use serde::Deserialize;

/// Probabilities (0..=1) of each injected fault. All default to zero.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FaultPlan {
    /// Redeliver a message that was already processed successfully.
    pub duplicate: f64,
    /// Deliver pending messages out of order.
    pub reorder: f64,
    /// Lose a delivery attempt in transit (the message is retried).
    pub drop: f64,
    /// Kill and restart the app after a delivery, losing its memory.
    pub crash_app: f64,
    /// Answer a state write with 409 (as if another writer won the ETag race).
    pub state_conflict: f64,
    /// Answer a state call with 500.
    pub state_error: f64,
}

impl FaultPlan {
    pub fn none() -> FaultPlan {
        FaultPlan::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        let named = [
            ("duplicate", self.duplicate),
            ("reorder", self.reorder),
            ("drop", self.drop),
            ("crash_app", self.crash_app),
            ("state_conflict", self.state_conflict),
            ("state_error", self.state_error),
        ];
        for (name, p) in named {
            if !(0.0..=1.0).contains(&p) || p.is_nan() {
                return Err(format!(
                    "fault probability `{name}` must be between 0 and 1 (got {p})"
                ));
            }
        }
        Ok(())
    }
}

/// Draws true with probability `p`, consuming randomness only when `p` is positive.
pub fn chance(rng: &mut anthill_core::rng::Rng, p: f64) -> bool {
    p > 0.0 && (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p
}

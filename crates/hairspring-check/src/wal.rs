//! Write-ahead-log checker: the recovered log is a prefix of the attempted appends and holds every acknowledged one.

use crate::common::operations_or_unknown;
use crate::history::{History, Operation, Outcome};
use crate::limits::Limits;
use crate::verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind};

/// Check `recovered` (the log's records after recovery, in order) against the `append` operations on `key`.
///
/// The attempted log is the appends that did not fail, in invocation order. `recovered` must equal the first
/// `recovered.len()` of them (a gap, a reordering or a torn record is `NotPrefix`; records beyond the attempted log are
/// a `Phantom`) and must reach at least the last acknowledged (`ok`) append (`TruncatedBelowAcknowledged`). Unknown
/// appends (`info`, or never completed) may be present or absent, but nothing after a missing one can be present.
///
/// This models one writer: appends whose invoke-to-completion intervals overlap give `Unknown`, as does any function
/// other than `append`. Linear time.
pub fn check_wal_prefix(
    history: &History,
    key: &str,
    recovered: &[i64],
    limits: &Limits,
) -> Verdict {
    let ops = match operations_or_unknown(history, key, limits) {
        Ok(ops) => ops,
        Err(reason) => return Verdict::Unknown(reason),
    };
    let mut attempted: Vec<(&Operation, i64)> = Vec::new();
    let mut last_acked: Option<usize> = None; // 1-based position in `attempted`
    let mut latest_completion: u64 = 0;
    for op in &ops {
        let Some(value) = op.arg.filter(|_| op.f == "append") else {
            return Verdict::Unknown(UnknownReason::UnsupportedOperation {
                key: key.to_string(),
                f: op.f.clone(),
            });
        };
        if op.invoke_seq < latest_completion {
            return Verdict::Unknown(UnknownReason::UnsupportedShape {
                key: key.to_string(),
                what: "overlapping appends",
            });
        }
        if let (Outcome::Ok(_) | Outcome::Fail, Some(done)) = (op.outcome, op.complete_seq) {
            latest_completion = latest_completion.max(done);
        }
        if op.outcome != Outcome::Fail {
            attempted.push((op, value));
            if matches!(op.outcome, Outcome::Ok(_)) {
                last_acked = Some(attempted.len());
            }
        }
    }
    let violation = |kind: ViolationKind, witness: Vec<&Operation>| {
        Verdict::Violation(Box::new(Violation {
            checker: CheckerId::WalPrefix,
            key: key.to_string(),
            kind,
            witness: witness.into_iter().cloned().collect(),
            witness_minimal: true,
        }))
    };

    let shared = recovered.len().min(attempted.len());
    if let Some(i) = (0..shared).find(|&i| recovered[i] != attempted[i].1) {
        return violation(ViolationKind::NotPrefix { index: i }, vec![attempted[i].0]);
    }
    if recovered.len() > attempted.len() {
        return violation(
            ViolationKind::Phantom {
                value: recovered[attempted.len()],
            },
            Vec::new(),
        );
    }
    let required = last_acked.unwrap_or(0);
    if recovered.len() < required {
        return violation(
            ViolationKind::TruncatedBelowAcknowledged {
                recovered: recovered.len(),
                required,
            },
            vec![attempted[required - 1].0],
        );
    }
    Verdict::Ok
}

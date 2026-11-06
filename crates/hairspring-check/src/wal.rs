//! Write-ahead-log checker: the recovered log is a prefix of the attempted appends and holds every acknowledged one.

use crate::common::operations_or_unknown;
use crate::history::{History, Operation, Outcome};
use crate::limits::Limits;
use crate::verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind};

/// The appends that may be in the log, in invocation order.
struct Attempted<'a> {
    appends: Vec<(&'a Operation, i64)>,
    /// 1-based position in `appends` of the last acknowledged append, if any.
    last_acked: Option<usize>,
}

fn unsupported_shape(key: &str, what: &'static str) -> UnknownReason {
    UnknownReason::UnsupportedShape {
        key: key.to_string(),
        what,
    }
}

/// Collect the non-failed appends, or the reason this history is outside the model.
fn scan<'a>(ops: &[&'a Operation], key: &str) -> Result<Attempted<'a>, UnknownReason> {
    let mut appends: Vec<(&Operation, i64)> = Vec::new();
    let mut last_acked = None;
    let mut latest_completion: u64 = 0;
    let mut unknown_seen = false;
    for &op in ops {
        let Some(value) = op.arg.filter(|_| op.f == "append") else {
            return Err(UnknownReason::UnsupportedOperation {
                key: key.to_string(),
                f: op.f.clone(),
            });
        };
        if op.invoke_seq < latest_completion {
            return Err(unsupported_shape(key, "overlapping appends"));
        }
        if let (Outcome::Ok(_) | Outcome::Fail, Some(done)) = (op.outcome, op.complete_seq) {
            latest_completion = latest_completion.max(done);
        }
        if op.outcome == Outcome::Fail {
            continue;
        }
        if unknown_seen {
            return Err(unsupported_shape(key, "append after an unknown append"));
        }
        appends.push((op, value));
        match op.outcome {
            Outcome::Ok(_) => last_acked = Some(appends.len()),
            _ => unknown_seen = true,
        }
    }
    Ok(Attempted {
        appends,
        last_acked,
    })
}

/// Check `recovered` (the log's records after recovery, in order) against the `append` operations on `key`.
///
/// The attempted log is the appends that did not fail, in invocation order. `recovered` must equal the first
/// `recovered.len()` of them (a gap, a reordering or a torn record is `NotPrefix`; records beyond the attempted log are
/// a `Phantom`) and must reach at least the last acknowledged (`ok`) append (`TruncatedBelowAcknowledged`).
///
/// This models one writer at a time, and is sound only for histories it models; anything else is `Unknown`, never a
/// guess:
/// - An unknown append (`info`, or never completed) that is followed by any later non-failed append. A timed-out
///   append may never land or may land late, so many recovered logs could be explained and no violation can be
///   claimed. An unknown append at the very end of the log is fine (it may or may not be present).
/// - Appends whose invoke-to-completion intervals overlap.
/// - Any function other than `append`, or an append with no value.
///
/// Linear time.
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
    let attempted = match scan(&ops, key) {
        Ok(attempted) => attempted,
        Err(reason) => return Verdict::Unknown(reason),
    };
    let violation = |kind: ViolationKind, witness: Vec<&Operation>| {
        Verdict::Violation(Box::new(Violation {
            checker: CheckerId::WalPrefix,
            key: key.to_string(),
            kind,
            witness: witness.into_iter().cloned().collect(),
            witness_minimal: true,
        }))
    };
    let appends = &attempted.appends;

    let shared = recovered.len().min(appends.len());
    if let Some(i) = (0..shared).find(|&i| recovered[i] != appends[i].1) {
        return violation(ViolationKind::NotPrefix { index: i }, vec![appends[i].0]);
    }
    if recovered.len() > appends.len() {
        let value = recovered[appends.len()];
        // The appends of that value (any outcome), else the last append on the key.
        let mut witness: Vec<&Operation> = ops
            .iter()
            .copied()
            .filter(|o| o.arg == Some(value))
            .collect();
        if witness.is_empty() {
            witness.extend(ops.last().copied());
        }
        return violation(ViolationKind::Phantom { value }, witness);
    }
    let required = attempted.last_acked.unwrap_or(0);
    if recovered.len() < required {
        return violation(
            ViolationKind::TruncatedBelowAcknowledged {
                recovered: recovered.len(),
                required,
            },
            vec![appends[required - 1].0],
        );
    }
    Verdict::Ok
}

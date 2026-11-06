//! Write-ahead-log checker: the recovered log is a prefix of the attempted appends and holds every acknowledged one.

use std::collections::BTreeMap;

use crate::common::operations_or_unknown;
use crate::history::{History, Operation, Outcome};
use crate::limits::Limits;
use crate::verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind};

/// The appends that may be in the log, in invocation order.
struct Attempted<'a> {
    appends: Vec<(&'a Operation, i64)>,
    /// 1-based position in `appends` of the last acknowledged append, if any.
    last_acked: Option<usize>,
    /// An unknown append is followed by a later non-failed append, so the order of the log is not determined.
    unordered: bool,
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
    let (mut unknown_seen, mut unordered) = (false, false);
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
        unordered |= unknown_seen;
        appends.push((op, value));
        match op.outcome {
            Outcome::Ok(_) => last_acked = Some(appends.len()),
            _ => unknown_seen = true,
        }
    }
    Ok(Attempted {
        appends,
        last_acked,
        unordered,
    })
}

fn violation(key: &str, kind: ViolationKind, witness: Vec<&Operation>) -> Verdict {
    Verdict::Violation(Box::new(Violation {
        checker: CheckerId::WalPrefix,
        key: key.to_string(),
        kind,
        witness: witness.into_iter().cloned().collect(),
        witness_minimal: true,
    }))
}

/// A value that no valid execution could have put in the log: witnessed by the appends of that value (any outcome),
/// else by the last append on the key.
fn phantom(key: &str, ops: &[&Operation], value: i64) -> Verdict {
    let mut witness: Vec<&Operation> = ops
        .iter()
        .copied()
        .filter(|o| o.arg == Some(value))
        .collect();
    if witness.is_empty() {
        witness.extend(ops.last().copied());
    }
    violation(key, ViolationKind::Phantom { value }, witness)
}

/// The order of the log is not determined (an unknown append may land anywhere after its invoke, or never), so decide
/// only what holds whatever it does: a value in the log more often than non-failed appends could have put it there is
/// a phantom, and an acknowledged value present less often than it was acknowledged is a lost acknowledgement.
/// Anything that depends on order is `Unknown`.
fn decide_unordered(
    key: &str,
    ops: &[&Operation],
    attempted: &Attempted<'_>,
    recovered: &[i64],
) -> Verdict {
    let mut recovered_count: BTreeMap<i64, usize> = BTreeMap::new();
    for v in recovered {
        *recovered_count.entry(*v).or_default() += 1;
    }
    let mut possible: BTreeMap<i64, usize> = BTreeMap::new();
    let mut acked: BTreeMap<i64, Vec<&Operation>> = BTreeMap::new();
    for &(op, value) in &attempted.appends {
        *possible.entry(value).or_default() += 1;
        if matches!(op.outcome, Outcome::Ok(_)) {
            acked.entry(value).or_default().push(op);
        }
    }
    for (&value, &seen) in &recovered_count {
        if seen > possible.get(&value).copied().unwrap_or(0) {
            return phantom(key, ops, value);
        }
    }
    for (&value, appends) in &acked {
        if recovered_count.get(&value).copied().unwrap_or(0) < appends.len() {
            return violation(
                key,
                ViolationKind::LostAcknowledgedWrite { value },
                appends.iter().take(1).copied().collect(),
            );
        }
    }
    Verdict::Unknown(unsupported_shape(key, "append after an unknown append"))
}

/// Check `recovered` (the log's records after recovery, in order) against the `append` operations on `key`.
///
/// The attempted log is the appends that did not fail, in invocation order. `recovered` must equal the first
/// `recovered.len()` of them (a gap, a reordering or a torn record is `NotPrefix`; records beyond the attempted log are
/// a `Phantom`) and must reach at least the last acknowledged (`ok`) append (`TruncatedBelowAcknowledged`).
///
/// This models one writer at a time, and is sound only for histories it models; anything else is `Unknown`, never a
/// guess:
/// - An unknown append (`info`, or never completed) followed by any later non-failed append. A timed-out append may
///   never land or may land late, so the order of the log is not determined and order-dependent claims are `Unknown`.
///   Two things still hold whatever the unknown append does, and are decided: a value in `recovered` more times than
///   there are non-failed appends of it is a `Phantom`, and an acknowledged value present fewer times than it was
///   acknowledged is a `LostAcknowledgedWrite`. A clean result in this shape is `Unknown`, not `Ok`. An unknown
///   append at the very end of the log is fine (it may or may not be present) and is checked exactly.
/// - Appends whose invoke-to-completion intervals overlap.
/// - Any function other than `append`, or an append with no value.
///
/// A restarted writer must use a NEW client name: a client that is killed mid-append leaves its append without a
/// completion record, and reusing the name for the next append is a second open operation for that client
/// ([`crate::HistoryError::ClientOverlap`]). A client that records `info` and carries on may keep its name.
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
    if attempted.unordered {
        return decide_unordered(key, &ops, &attempted, recovered);
    }
    let appends = &attempted.appends;

    let shared = recovered.len().min(appends.len());
    if let Some(i) = (0..shared).find(|&i| recovered[i] != appends[i].1) {
        return violation(
            key,
            ViolationKind::NotPrefix { index: i },
            vec![appends[i].0],
        );
    }
    if recovered.len() > appends.len() {
        return phantom(key, &ops, recovered[appends.len()]);
    }
    let required = attempted.last_acked.unwrap_or(0);
    if recovered.len() < required {
        return violation(
            key,
            ViolationKind::TruncatedBelowAcknowledged {
                recovered: recovered.len(),
                required,
            },
            vec![appends[required - 1].0],
        );
    }
    Verdict::Ok
}

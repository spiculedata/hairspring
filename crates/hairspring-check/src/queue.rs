//! Queue checker: no acknowledged enqueue is lost, nothing is delivered twice, nothing is delivered that was not
//! enqueued. Order is not checked.

use std::collections::BTreeMap;

use crate::common::operations_or_unknown;
use crate::history::{History, Operation, Outcome};
use crate::limits::Limits;
use crate::verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind};

#[derive(Default)]
struct PerValue<'a> {
    acked: Vec<&'a Operation>,
    unknown: Vec<&'a Operation>,
    failed: Vec<&'a Operation>,
    delivered: Vec<&'a Operation>,
    remaining: u64,
}

/// Check the `enqueue` and `dequeue` operations on `key` against `remaining`, the queue's contents after the run.
///
/// Values are counted, not assumed unique. For each value, the deliveries (acknowledged dequeues that returned it,
/// plus copies still in `remaining`) must not exceed the enqueues that might have happened (`ok` or `info`), and must
/// cover the `ok` enqueues. A dequeue that completed `info` (or never completed) may have removed one element without
/// us seeing which, so up to that many missing elements are excused in total. A dequeue that returned null (empty) is
/// ignored. Any other function gives `Unknown`.
///
/// Linear time, deterministic (ascending value order).
pub fn check_queue(history: &History, key: &str, remaining: &[i64], limits: &Limits) -> Verdict {
    let ops = match operations_or_unknown(history, key, limits) {
        Ok(ops) => ops,
        Err(reason) => return Verdict::Unknown(reason),
    };
    let mut values: BTreeMap<i64, PerValue> = BTreeMap::new();
    let mut unknown_dequeues: u64 = 0;
    for op in &ops {
        match (op.f.as_str(), op.arg, op.outcome) {
            ("enqueue", Some(v), outcome) => {
                let slot = values.entry(v).or_default();
                match outcome {
                    Outcome::Ok(_) => slot.acked.push(op),
                    Outcome::Info => slot.unknown.push(op),
                    Outcome::Fail => slot.failed.push(op),
                }
            }
            ("dequeue", _, Outcome::Ok(Some(v))) => values.entry(v).or_default().delivered.push(op),
            ("dequeue", _, Outcome::Info) => unknown_dequeues += 1,
            ("dequeue", _, _) => {}
            _ => {
                return Verdict::Unknown(UnknownReason::UnsupportedOperation {
                    key: key.to_string(),
                    f: op.f.clone(),
                });
            }
        }
    }
    for v in remaining {
        values.entry(*v).or_default().remaining += 1;
    }
    let violation = |kind: ViolationKind, witness: Vec<&Operation>| {
        Verdict::Violation(Box::new(Violation {
            checker: CheckerId::QueueNoLossNoDup,
            key: key.to_string(),
            kind,
            witness: witness.into_iter().cloned().collect(),
            witness_minimal: true,
        }))
    };

    let mut missing_total: u64 = 0;
    let mut first_missing: Option<(i64, &PerValue)> = None;
    for (&v, slot) in &values {
        let possible = (slot.acked.len() + slot.unknown.len()) as u64;
        let seen = slot.delivered.len() as u64 + slot.remaining;
        let mut involved: Vec<&Operation> = slot.failed.clone();
        involved.extend(slot.acked.iter().chain(&slot.unknown));
        involved.extend(&slot.delivered);
        involved.sort_by_key(|o| o.invoke_seq);
        if possible == 0 && seen > 0 {
            return violation(ViolationKind::Phantom { value: v }, involved);
        }
        if seen > possible {
            return violation(
                ViolationKind::Duplicate {
                    value: v,
                    seen,
                    at_most: possible,
                },
                involved,
            );
        }
        let required = slot.acked.len() as u64;
        if seen < required {
            missing_total += required - seen;
            first_missing.get_or_insert((v, slot));
        }
    }
    if missing_total > unknown_dequeues {
        if let Some((v, slot)) = first_missing {
            return violation(
                ViolationKind::LostMessage { value: v },
                slot.acked.iter().take(1).copied().collect(),
            );
        }
    }
    Verdict::Ok
}

//! Set checker: after the run, every acknowledged `add` is in the final set and nothing is in it that was not added.

use std::collections::BTreeMap;

use crate::common::operations_or_unknown;
use crate::history::{History, Operation, Outcome};
use crate::limits::Limits;
use crate::verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind};

/// Check `final_set` (the set's contents after the quiet period, as a list) against the `add` operations on `key`.
///
/// Operations: `add` (argument required); `read` is allowed and ignored; any other function gives `Unknown`, because
/// it might change the set. An `add` that completed `ok` must be present; one that is `info` (or has no completion)
/// may or may not be; one that `fail`ed must not be. The final set listing a value twice is reported as a duplicate.
///
/// Linear time. Violations are found in a fixed order (lost acknowledged add by invocation order, then final-state
/// anomalies by ascending value), so the verdict is deterministic.
pub fn check_set(history: &History, key: &str, final_set: &[i64], limits: &Limits) -> Verdict {
    let ops = match operations_or_unknown(history, key, limits) {
        Ok(ops) => ops,
        Err(reason) => return Verdict::Unknown(reason),
    };
    let mut adds: BTreeMap<i64, Vec<&Operation>> = BTreeMap::new();
    for op in &ops {
        match (op.f.as_str(), op.arg) {
            ("add", Some(v)) => adds.entry(v).or_default().push(op),
            ("read", _) => {}
            _ => {
                return Verdict::Unknown(UnknownReason::UnsupportedOperation {
                    key: key.to_string(),
                    f: op.f.clone(),
                });
            }
        }
    }
    let mut counts: BTreeMap<i64, u64> = BTreeMap::new();
    for v in final_set {
        *counts.entry(*v).or_insert(0) += 1;
    }
    let violation = |kind: ViolationKind, witness: Vec<&Operation>| {
        Verdict::Violation(Box::new(Violation {
            checker: CheckerId::SetNoLostAcks,
            key: key.to_string(),
            kind,
            witness: witness.into_iter().cloned().collect(),
            witness_minimal: true,
        }))
    };

    for op in &ops {
        let (Some(v), Outcome::Ok(_)) = (op.arg, op.outcome) else {
            continue;
        };
        if op.f == "add" && !counts.contains_key(&v) {
            return violation(ViolationKind::LostAcknowledgedWrite { value: v }, vec![op]);
        }
    }
    for (&v, &seen) in &counts {
        let all = adds.get(&v).map(Vec::as_slice).unwrap_or(&[]);
        let possible: Vec<&Operation> = all
            .iter()
            .copied()
            .filter(|o| o.outcome != Outcome::Fail)
            .collect();
        if possible.is_empty() {
            return violation(ViolationKind::Phantom { value: v }, all.to_vec());
        }
        if seen > 1 {
            return violation(
                ViolationKind::Duplicate {
                    value: v,
                    seen,
                    at_most: 1,
                },
                possible,
            );
        }
    }
    Verdict::Ok
}

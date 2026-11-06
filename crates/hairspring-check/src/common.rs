//! Checks every checker makes before looking at operations.

use crate::history::{Completeness, History, Operation};
use crate::limits::Limits;
use crate::verdict::UnknownReason;

/// `Some` when `key` has more operations than `limits` allow.
pub(crate) fn too_many_operations(
    key: &str,
    operations: usize,
    limits: &Limits,
) -> Option<UnknownReason> {
    (operations > limits.max_ops_per_key).then(|| UnknownReason::TooManyOperations {
        key: key.to_string(),
        operations,
        max: limits.max_ops_per_key,
    })
}

/// Operations on `key`, or the reason the checker must answer `Unknown` without looking further.
pub(crate) fn operations_or_unknown<'a>(
    history: &'a History,
    key: &'a str,
    limits: &Limits,
) -> Result<Vec<&'a Operation>, UnknownReason> {
    if history.completeness() == Completeness::Incomplete {
        return Err(UnknownReason::IncompleteHistory);
    }
    let ops: Vec<&Operation> = history.operations_for_key(key).collect();
    match too_many_operations(key, ops.len(), limits) {
        Some(reason) => Err(reason),
        None => Ok(ops),
    }
}

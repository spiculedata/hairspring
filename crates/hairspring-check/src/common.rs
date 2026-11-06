//! Checks every checker makes before looking at operations.

use crate::history::{Completeness, History, Operation};
use crate::limits::Limits;
use crate::verdict::UnknownReason;

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
    if ops.len() > limits.max_ops_per_key {
        return Err(UnknownReason::TooManyOperations {
            key: key.to_string(),
            operations: ops.len(),
            max: limits.max_ops_per_key,
        });
    }
    Ok(ops)
}

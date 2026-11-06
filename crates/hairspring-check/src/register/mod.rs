//! Single-register linearizability, checked per key.
//!
//! Semantics. A register starts at an initial value (`None` is "never written"; see [`RegisterInitials`]). `write`
//! sets the value. `read` returns the value. `cas` (expected in `expect`, new value in the argument) sets the new value
//! when the register holds the expected one. Real time is the global sequence number: an operation that completed
//! before another was invoked must be ordered before it.
//!
//! - `ok` operations must take effect, exactly once, between invoke and completion. A `cas` that returned `ok`
//!   must have found its expected value.
//! - `fail` operations did not happen and are dropped (record a `cas` whose precondition did not hold as `fail`).
//! - `info` and never-completed writes and `cas`es may take effect at any point after their invoke, or never.
//! - `info` reads have no effect and are dropped.
//!
//! An empty history is linearizable (`Ok`) unless it is flagged incomplete (`Unknown`).

mod search;
mod witness;

use std::collections::BTreeMap;

use crate::common::{operations_or_unknown, too_many_operations};
use crate::history::{Completeness, History, Operation, Outcome};
use crate::limits::Limits;
use crate::verdict::{CheckerId, UnknownReason, Verdict, Violation, ViolationKind};
use search::{Exhausted, Kind, RegOp};

/// The value each register holds before the history starts: one default for every key, with optional per-key
/// overrides. `None` means "never written" (a read returns null).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RegisterInitials {
    default: Option<i64>,
    per_key: BTreeMap<String, Option<i64>>,
}

impl RegisterInitials {
    /// The same initial value for every key.
    pub fn all(initial: Option<i64>) -> Self {
        RegisterInitials {
            default: initial,
            per_key: BTreeMap::new(),
        }
    }

    /// A copy with `key` starting at `initial` instead of the default.
    pub fn with_key(mut self, key: impl Into<String>, initial: Option<i64>) -> Self {
        self.per_key.insert(key.into(), initial);
        self
    }

    /// The initial value for `key`.
    pub fn for_key(&self, key: &str) -> Option<i64> {
        self.per_key.get(key).copied().unwrap_or(self.default)
    }
}

impl From<Option<i64>> for RegisterInitials {
    fn from(initial: Option<i64>) -> Self {
        RegisterInitials::all(initial)
    }
}

/// Check one key. See the module documentation for the model. Exhausting `limits.max_steps` or
/// `limits.max_memo_entries` gives `Unknown`, never `Ok`.
pub fn check_register_key(
    history: &History,
    key: &str,
    initial: Option<i64>,
    limits: &Limits,
) -> Verdict {
    match operations_or_unknown(history, key, limits) {
        Ok(ops) => check_ops(&ops, key, initial, limits),
        Err(reason) => Verdict::Unknown(reason),
    }
}

/// Check every key of the history independently (operations on different keys cannot constrain each other). Keys are
/// grouped in one pass, so the cost is linear in the number of keys plus each key's own search. The first violation,
/// in key order, wins; otherwise any `Unknown` makes the answer `Unknown`; only if every key is decided and clean is
/// the answer `Ok`. An incomplete history is always `Unknown`.
///
/// `initial` is an `Option<i64>` (the same initial value for every key) or a [`RegisterInitials`] with per-key values.
pub fn check_register_linearizable(
    history: &History,
    initial: impl Into<RegisterInitials>,
    limits: &Limits,
) -> Verdict {
    if history.completeness() == Completeness::Incomplete {
        return Verdict::Unknown(UnknownReason::IncompleteHistory);
    }
    let initials = initial.into();
    let mut first_unknown: Option<Verdict> = None;
    for (key, ops) in history.operations_by_key() {
        let verdict = match too_many_operations(key, ops.len(), limits) {
            Some(reason) => Verdict::Unknown(reason),
            None => check_ops(&ops, key, initials.for_key(key), limits),
        };
        match verdict {
            Verdict::Ok => {}
            violation @ Verdict::Violation(_) => return violation,
            unknown => {
                first_unknown.get_or_insert(unknown);
            }
        }
    }
    first_unknown.unwrap_or(Verdict::Ok)
}

fn check_ops(ops: &[&Operation], key: &str, initial: Option<i64>, limits: &Limits) -> Verdict {
    let specs = match to_specs(ops, key) {
        Ok(specs) => specs,
        Err(reason) => return Verdict::Unknown(reason),
    };
    let mut steps = limits.max_steps;
    match search::linearizable(&specs, initial, &mut steps, limits.max_memo_entries) {
        Ok(true) => Verdict::Ok,
        Ok(false) => {
            let shrunk = witness::shrink(&specs, initial, limits);
            Verdict::Violation(Box::new(Violation {
                checker: CheckerId::RegisterLinearizable,
                key: key.to_string(),
                kind: ViolationKind::NotLinearizable,
                witness: shrunk.ops.iter().map(|s| ops[s.source].clone()).collect(),
                witness_minimal: shrunk.minimal,
            }))
        }
        Err(Exhausted::Steps) => Verdict::Unknown(UnknownReason::StepBudgetExhausted {
            max_steps: limits.max_steps,
        }),
        Err(Exhausted::Memo) => Verdict::Unknown(UnknownReason::MemoBudgetExhausted {
            max_entries: limits.max_memo_entries,
        }),
    }
}

fn to_specs(ops: &[&Operation], key: &str) -> Result<Vec<RegOp>, UnknownReason> {
    let unsupported = |op: &Operation| UnknownReason::UnsupportedOperation {
        key: key.to_string(),
        f: op.f.clone(),
    };
    let mut specs = Vec::new();
    for (source, op) in ops.iter().enumerate() {
        let (kind, required) = match (op.f.as_str(), op.outcome) {
            ("write", outcome) => (
                Kind::Write(op.arg.ok_or_else(|| unsupported(op))?),
                is_ok(outcome),
            ),
            ("cas", outcome) => {
                let (Some(expect), Some(new)) = (op.expect, op.arg) else {
                    return Err(unsupported(op));
                };
                (Kind::Cas(expect, new), is_ok(outcome))
            }
            ("read", Outcome::Ok(result)) => (Kind::Read(result), true),
            ("read", _) => continue,
            _ => return Err(unsupported(op)),
        };
        if op.outcome == Outcome::Fail {
            continue;
        }
        let complete = match (required, op.complete_seq) {
            (true, Some(done)) => done,
            _ => u64::MAX,
        };
        specs.push(RegOp {
            source,
            invoke: op.invoke_seq,
            complete,
            required,
            kind,
        });
    }
    Ok(specs)
}

fn is_ok(outcome: Outcome) -> bool {
    matches!(outcome, Outcome::Ok(_))
}

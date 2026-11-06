//! Wing-Gong-Lowe style search with a memo of failed states.
//!
//! A state is (set of operations already placed in the order, register value). From a state, any unplaced operation
//! whose invoke precedes the earliest completion among unplaced required operations may be placed next (anything else
//! would have to come after an operation that finished before it started). If every required operation can be placed
//! the history is linearizable. Failed states are remembered, so each is expanded once.
//!
//! The search is an explicit-stack depth-first search (no recursion), tries candidates in invoke order (deterministic),
//! and counts each candidate tried as one step.

use std::collections::HashSet;

/// What an operation does to the register.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    Write(i64),
    Read(Option<i64>),
    /// `(expected, new)`
    Cas(i64, i64),
}

/// An operation as the search sees it. Slices of these must be ordered by `invoke`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RegOp {
    /// Index of the operation in the key's operation list (to rebuild a witness).
    pub source: usize,
    pub invoke: u64,
    /// `u64::MAX` for an operation that never has to finish.
    pub complete: u64,
    /// Must be placed. Unknown writes and `cas`es are optional.
    pub required: bool,
    pub kind: Kind,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Exhausted {
    Steps,
    Memo,
}

struct Frame {
    /// Next index to consider as a candidate.
    cursor: usize,
    /// Earliest completion among unplaced required operations.
    horizon: u64,
    /// First unplaced operation in invoke order, and in completion order (both only move forward down the stack).
    first_unplaced: usize,
    first_unplaced_by_completion: usize,
    /// The operation placed from this node, and the register value before it.
    placed: Option<(usize, Option<i64>)>,
}

fn is_set(bits: &[u64], i: usize) -> bool {
    bits[i / 64] & (1 << (i % 64)) != 0
}

fn toggle(bits: &mut [u64], i: usize) {
    bits[i / 64] ^= 1 << (i % 64);
}

fn apply(kind: Kind, register: Option<i64>) -> Option<Option<i64>> {
    match kind {
        Kind::Write(v) => Some(Some(v)),
        Kind::Read(seen) => (register == seen).then_some(register),
        Kind::Cas(expect, new) => (register == Some(expect)).then_some(Some(new)),
    }
}

fn new_frame(
    parent: (usize, usize),
    bits: &[u64],
    ops: &[RegOp],
    by_completion: &[usize],
) -> Frame {
    let mut first = parent.0;
    while first < ops.len() && is_set(bits, first) {
        first += 1;
    }
    let mut first_c = parent.1;
    while first_c < by_completion.len() && is_set(bits, by_completion[first_c]) {
        first_c += 1;
    }
    // Callers only build a frame while a required operation is unplaced, so `first_c` is in range.
    let horizon = by_completion
        .get(first_c)
        .map_or(u64::MAX, |&i| ops[i].complete);
    Frame {
        cursor: first,
        horizon,
        first_unplaced: first,
        first_unplaced_by_completion: first_c,
        placed: None,
    }
}

/// Is there an order of `ops` that respects real time and the register's behaviour? `steps_left` is decremented for
/// every candidate tried and is shared across calls; `Err` means the answer is not known.
pub(crate) fn linearizable(
    ops: &[RegOp],
    initial: Option<i64>,
    steps_left: &mut u64,
    max_memo_entries: usize,
) -> Result<bool, Exhausted> {
    let mut required_left = ops.iter().filter(|o| o.required).count();
    if required_left == 0 {
        return Ok(true);
    }
    let mut by_completion: Vec<usize> = (0..ops.len()).filter(|&i| ops[i].required).collect();
    by_completion.sort_by_key(|&i| (ops[i].complete, i));

    let mut bits = vec![0u64; ops.len().div_ceil(64)];
    let mut register = initial;
    let mut memo: HashSet<(Vec<u64>, Option<i64>)> = HashSet::new();
    let mut stack = vec![new_frame((0, 0), &bits, ops, &by_completion)];

    loop {
        let depth = stack.len() - 1;
        let (mut i, horizon) = (stack[depth].cursor, stack[depth].horizon);
        while i < ops.len() && ops[i].invoke < horizon && is_set(&bits, i) {
            i += 1;
        }
        if i >= ops.len() || ops[i].invoke >= horizon {
            // Dead end: remember the state, then undo the placement that led here.
            if memo.len() >= max_memo_entries {
                return Err(Exhausted::Memo);
            }
            memo.insert((bits.clone(), register));
            stack.pop();
            let Some(parent) = stack.last_mut() else {
                return Ok(false);
            };
            let (placed, before) = parent
                .placed
                .take()
                .expect("a parent frame has placed an operation");
            toggle(&mut bits, placed);
            register = before;
            if ops[placed].required {
                required_left += 1;
            }
            continue;
        }

        stack[depth].cursor = i + 1;
        if *steps_left == 0 {
            return Err(Exhausted::Steps);
        }
        *steps_left -= 1;
        let Some(next) = apply(ops[i].kind, register) else {
            continue;
        };

        let before = register;
        toggle(&mut bits, i);
        register = next;
        if ops[i].required {
            required_left -= 1;
            if required_left == 0 {
                return Ok(true);
            }
        }
        if memo.contains(&(bits.clone(), register)) {
            toggle(&mut bits, i);
            register = before;
            if ops[i].required {
                required_left += 1;
            }
            continue;
        }
        stack[depth].placed = Some((i, before));
        let parent = (
            stack[depth].first_unplaced,
            stack[depth].first_unplaced_by_completion,
        );
        stack.push(new_frame(parent, &bits, ops, &by_completion));
    }
}

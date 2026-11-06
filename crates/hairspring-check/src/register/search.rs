//! Wing-Gong-Lowe style search with a memo of failed states.
//!
//! A state is (set of operations already placed in the order, register value). From a state, any unplaced operation
//! whose invoke precedes the earliest completion among unplaced required operations may be placed next (anything else
//! would have to come after an operation that finished before it started). If every required operation can be placed
//! the history is linearizable. Failed states are remembered, so each is expanded once.
//!
//! The search is an explicit-stack depth-first search (no recursion), tries candidates in invoke order (deterministic),
//! and counts one step for every candidate tried and for every already-placed operation skipped while looking for the
//! next candidate.

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

fn spend(steps_left: &mut u64) -> Result<(), Exhausted> {
    if *steps_left == 0 {
        return Err(Exhausted::Steps);
    }
    *steps_left -= 1;
    Ok(())
}

struct Search<'a> {
    ops: &'a [RegOp],
    /// Required operations by completion time.
    by_completion: Vec<usize>,
    /// Placed operations, as a bit set over `ops`.
    bits: Vec<u64>,
    register: Option<i64>,
    required_left: usize,
    memo: HashSet<(Vec<u64>, Option<i64>)>,
    stack: Vec<Frame>,
}

impl<'a> Search<'a> {
    fn new(ops: &'a [RegOp], initial: Option<i64>) -> Self {
        let mut by_completion: Vec<usize> = (0..ops.len()).filter(|&i| ops[i].required).collect();
        by_completion.sort_by_key(|&i| (ops[i].complete, i));
        Search {
            ops,
            required_left: by_completion.len(),
            by_completion,
            bits: vec![0u64; ops.len().div_ceil(64)],
            register: initial,
            memo: HashSet::new(),
            stack: Vec::new(),
        }
    }

    /// A node for the current state; `parent` holds the parent's first-unplaced positions (a valid starting point,
    /// since a child has placed everything its parent had).
    fn frame(&self, parent: (usize, usize)) -> Frame {
        let mut first = parent.0;
        while first < self.ops.len() && is_set(&self.bits, first) {
            first += 1;
        }
        let mut first_c = parent.1;
        while first_c < self.by_completion.len() && is_set(&self.bits, self.by_completion[first_c])
        {
            first_c += 1;
        }
        let horizon = self
            .by_completion
            .get(first_c)
            .map_or(u64::MAX, |&i| self.ops[i].complete);
        Frame {
            cursor: first,
            horizon,
            first_unplaced: first,
            first_unplaced_by_completion: first_c,
            placed: None,
        }
    }

    /// The next unplaced operation that may be placed from the top node, skipping placed ones (each skip is a step).
    fn next_candidate(&mut self, steps_left: &mut u64) -> Result<Option<usize>, Exhausted> {
        let depth = self.stack.len() - 1;
        let horizon = self.stack[depth].horizon;
        let mut i = self.stack[depth].cursor;
        while i < self.ops.len() && self.ops[i].invoke < horizon {
            if !is_set(&self.bits, i) {
                self.stack[depth].cursor = i + 1;
                return Ok(Some(i));
            }
            spend(steps_left)?;
            i += 1;
        }
        self.stack[depth].cursor = i;
        Ok(None)
    }

    fn place(&mut self, i: usize, next_register: Option<i64>) {
        toggle(&mut self.bits, i);
        self.register = next_register;
        if self.ops[i].required {
            self.required_left -= 1;
        }
    }

    fn unplace(&mut self, i: usize, before: Option<i64>) {
        toggle(&mut self.bits, i);
        self.register = before;
        if self.ops[i].required {
            self.required_left += 1;
        }
    }

    /// The top node has no candidates left: remember its state and undo the placement that led to it. `Ok(false)`
    /// means that was the root, so nothing is left to try.
    fn backtrack(&mut self, max_memo_entries: usize) -> Result<bool, Exhausted> {
        if self.memo.len() >= max_memo_entries {
            return Err(Exhausted::Memo);
        }
        self.memo.insert((self.bits.clone(), self.register));
        self.stack.pop();
        let Some(parent) = self.stack.last_mut() else {
            return Ok(false);
        };
        let (placed, before) = parent
            .placed
            .take()
            .expect("a parent frame has placed an operation");
        self.unplace(placed, before);
        Ok(true)
    }

    fn run(&mut self, steps_left: &mut u64, max_memo_entries: usize) -> Result<bool, Exhausted> {
        if self.required_left == 0 {
            return Ok(true);
        }
        let root = self.frame((0, 0));
        self.stack.push(root);
        loop {
            let Some(i) = self.next_candidate(steps_left)? else {
                if !self.backtrack(max_memo_entries)? {
                    return Ok(false);
                }
                continue;
            };
            spend(steps_left)?;
            let Some(next) = apply(self.ops[i].kind, self.register) else {
                continue;
            };
            let before = self.register;
            self.place(i, next);
            if self.required_left == 0 {
                return Ok(true);
            }
            if self.memo.contains(&(self.bits.clone(), self.register)) {
                self.unplace(i, before);
                continue;
            }
            let depth = self.stack.len() - 1;
            self.stack[depth].placed = Some((i, before));
            let parent = (
                self.stack[depth].first_unplaced,
                self.stack[depth].first_unplaced_by_completion,
            );
            let child = self.frame(parent);
            self.stack.push(child);
        }
    }
}

/// Is there an order of `ops` that respects real time and the register's behaviour? `steps_left` is decremented for
/// every candidate tried or skipped and is shared across calls; `Err` means the answer is not known.
///
/// Cost: each remembered failed state holds a copy of the placed-set (`ops.len() / 8` bytes) plus the register, so
/// memory is about `max_memo_entries * (ops.len() / 8 + 40)` bytes, and each step may clone that set once to probe the
/// memo.
pub(crate) fn linearizable(
    ops: &[RegOp],
    initial: Option<i64>,
    steps_left: &mut u64,
    max_memo_entries: usize,
) -> Result<bool, Exhausted> {
    Search::new(ops, initial).run(steps_left, max_memo_entries)
}

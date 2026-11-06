//! Shrinking a violating history to a 1-minimal witness (delta debugging over whole operations).

use super::search::{linearizable, Exhausted, RegOp};
use crate::limits::Limits;

pub(crate) struct Shrunk {
    pub ops: Vec<RegOp>,
    /// True when no single operation can be removed without making the history linearizable.
    pub minimal: bool,
}

/// Shrink `ops` (known not linearizable) within `limits.max_witness_steps`. The result is still a violating
/// sub-history in the original order; `minimal` is false when the budget ran out first.
pub(crate) fn shrink(ops: &[RegOp], initial: Option<i64>, limits: &Limits) -> Shrunk {
    let mut current: Vec<RegOp> = ops.to_vec();
    let mut steps_left = limits.max_witness_steps;
    let mut minimal = true;
    let mut granularity = 2usize;

    while current.len() > 1 {
        let chunk = current.len().div_ceil(granularity);
        let mut reduced = false;
        let mut start = 0;
        while start < current.len() {
            let end = (start + chunk).min(current.len());
            let complement: Vec<RegOp> = current[..start]
                .iter()
                .chain(&current[end..])
                .copied()
                .collect();
            match linearizable(
                &complement,
                initial,
                &mut steps_left,
                limits.max_memo_entries,
            ) {
                Ok(false) => {
                    current = complement;
                    granularity = granularity.saturating_sub(1).max(2);
                    reduced = true;
                    break;
                }
                Ok(true) => {}
                Err(Exhausted::Steps) => {
                    return Shrunk {
                        ops: current,
                        minimal: false,
                    }
                }
                Err(Exhausted::Memo) => minimal = false,
            }
            start = end;
        }
        if reduced {
            continue;
        }
        if granularity >= current.len() {
            break;
        }
        granularity = (granularity * 2).min(current.len());
    }
    Shrunk {
        ops: current,
        minimal,
    }
}

#[cfg(test)]
mod tests {
    use super::super::search::Kind;
    use super::*;

    /// Six overlapping writes, then reads of 1 and 2 in sequence: not linearizable, and the search needs several
    /// remembered states to see it.
    fn hard_ops() -> Vec<RegOp> {
        let op = |source, invoke, complete, kind| RegOp {
            source,
            invoke,
            complete,
            required: true,
            kind,
        };
        let mut ops: Vec<RegOp> = (0..6u64)
            .map(|i| op(i as usize, i, 6 + i, Kind::Write(i as i64)))
            .collect();
        ops.push(op(6, 12, 13, Kind::Read(Some(1))));
        ops.push(op(7, 14, 15, Kind::Read(Some(2))));
        ops
    }

    #[test]
    fn memo_budget_hit_inside_the_shrink_means_not_minimal() {
        let limits = Limits {
            max_memo_entries: 1,
            max_witness_steps: 1_000_000,
            ..Limits::default()
        };
        let shrunk = shrink(&hard_ops(), None, &limits);
        assert!(!shrunk.minimal);
        assert!(!shrunk.ops.is_empty());
    }

    #[test]
    fn with_enough_budget_the_shrink_is_minimal() {
        let shrunk = shrink(&hard_ops(), None, &Limits::default());
        assert!(shrunk.minimal);
        assert!(shrunk.ops.len() <= 4);
    }
}

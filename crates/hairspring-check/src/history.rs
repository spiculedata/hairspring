//! The in-memory operation history the checkers consume, and the seam that builds it.
//!
//! The wire form (the `op` lines of the guest protocol addendum, still a draft) is not parsed here. A future parser
//! produces [`OpRecord`]s, one per line, and [`History::from_records`] pairs invokes with completions and checks the
//! structure. Nothing else in this crate knows about the wire form.

use std::collections::{BTreeMap, BTreeSet};

/// Upper bound on records accepted by [`History::from_records`]. The protocol budget is far lower (200,000 lines);
/// this only stops unbounded memory use from a caller that skipped its own limit.
pub const MAX_RECORDS: usize = 1_000_000;

/// The `type` of an `op` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpType {
    /// Before the request is sent.
    Invoke,
    /// The operation definitely happened.
    Ok,
    /// The operation definitely did not happen.
    Fail,
    /// Unknown: timed out, connection lost.
    Info,
}

/// One `op` line, field for field. No validation beyond types: [`History::from_records`] does that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpRecord {
    pub client: String,
    /// The client's own operation number; pairs an invoke with its completion.
    pub id: u64,
    pub kind: OpType,
    /// Function name: `read`, `write`, `cas`, `add`, `enqueue`, `dequeue`, `append`, ...
    pub f: String,
    pub key: String,
    /// On an invoke: the argument. On `ok`: the result (a read's value, a dequeued element). `None` is JSON null.
    pub value: Option<i64>,
    /// The expected value of a `cas` (on the invoke).
    pub expect: Option<i64>,
    /// Global sequence number: the real-time order. Unique within a history.
    pub seq: u64,
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Definitely happened, with this result (`None` for null).
    Ok(Option<i64>),
    /// Definitely did not happen.
    Fail,
    /// Unknown. Also used for an invoke with no completion at all (see [`Operation::is_pending`]).
    Info,
}

/// An invoke paired with its completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub client: String,
    pub id: u64,
    pub f: String,
    pub key: String,
    /// The argument given at invoke.
    pub arg: Option<i64>,
    /// The `cas` expected value given at invoke.
    pub expect: Option<i64>,
    pub outcome: Outcome,
    pub invoke_seq: u64,
    /// Sequence number of the completion, if there was one. For `Info` it is informational: an unknown operation may
    /// take effect at any time after its invoke.
    pub complete_seq: Option<u64>,
}

impl Operation {
    /// An invoke with no completion line (the client never reported back).
    pub fn is_pending(&self) -> bool {
        self.complete_seq.is_none()
    }
}

/// Whether the history is known to be whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    /// A client died without `end`, or lines were dropped (`history_over_limit`). Every checker answers `Unknown`.
    Incomplete,
}

/// Why records could not be turned into a history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    TooManyRecords {
        records: usize,
        max: usize,
    },
    DuplicateSeq {
        seq: u64,
    },
    DuplicateInvoke {
        client: String,
        id: u64,
    },
    /// A completion with no earlier invoke from the same client and id.
    CompletionWithoutInvoke {
        client: String,
        id: u64,
    },
    DuplicateCompletion {
        client: String,
        id: u64,
    },
    /// The completion names a different function or key than its invoke.
    MismatchedCompletion {
        client: String,
        id: u64,
    },
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::TooManyRecords { records, max } => {
                write!(f, "{records} records, at most {max}")
            }
            HistoryError::DuplicateSeq { seq } => write!(f, "sequence number {seq} used twice"),
            HistoryError::DuplicateInvoke { id, .. } => {
                write!(f, "operation {id} invoked twice by one client")
            }
            HistoryError::CompletionWithoutInvoke { id, .. } => {
                write!(f, "operation {id} completed without an invoke")
            }
            HistoryError::DuplicateCompletion { id, .. } => {
                write!(f, "operation {id} completed twice")
            }
            HistoryError::MismatchedCompletion { id, .. } => {
                write!(f, "operation {id} completion does not match its invoke")
            }
        }
    }
}

impl std::error::Error for HistoryError {}

/// An immutable, structurally valid history: operations ordered by invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    operations: Vec<Operation>,
    completeness: Completeness,
}

impl History {
    /// Pair `invoke` records with completions (by client and id), ordering by `seq` first, so the input order does
    /// not matter. Errors are explicit: duplicate `seq`, an id invoked twice, a completion with no invoke before it
    /// (in `seq` order), a second completion, or a completion that names another function or key.
    pub fn from_records(
        records: &[OpRecord],
        completeness: Completeness,
    ) -> Result<History, HistoryError> {
        if records.len() > MAX_RECORDS {
            return Err(HistoryError::TooManyRecords {
                records: records.len(),
                max: MAX_RECORDS,
            });
        }
        let mut ordered: Vec<&OpRecord> = records.iter().collect();
        ordered.sort_by_key(|r| r.seq);
        if let Some(pair) = ordered.windows(2).find(|w| w[0].seq == w[1].seq) {
            return Err(HistoryError::DuplicateSeq { seq: pair[0].seq });
        }
        let mut slots: Vec<Operation> = Vec::new();
        let mut index: BTreeMap<(&str, u64), usize> = BTreeMap::new();
        for r in ordered {
            let at = index.get(&(r.client.as_str(), r.id)).copied();
            match (r.kind, at) {
                (OpType::Invoke, Some(_)) => {
                    return Err(HistoryError::DuplicateInvoke {
                        client: r.client.clone(),
                        id: r.id,
                    });
                }
                (OpType::Invoke, None) => {
                    index.insert((r.client.as_str(), r.id), slots.len());
                    slots.push(Operation {
                        client: r.client.clone(),
                        id: r.id,
                        f: r.f.clone(),
                        key: r.key.clone(),
                        arg: r.value,
                        expect: r.expect,
                        outcome: Outcome::Info,
                        invoke_seq: r.seq,
                        complete_seq: None,
                    });
                }
                (_, None) => {
                    return Err(HistoryError::CompletionWithoutInvoke {
                        client: r.client.clone(),
                        id: r.id,
                    });
                }
                (kind, Some(i)) => {
                    let op = &mut slots[i];
                    if op.complete_seq.is_some() {
                        return Err(HistoryError::DuplicateCompletion {
                            client: r.client.clone(),
                            id: r.id,
                        });
                    }
                    if op.f != r.f || op.key != r.key {
                        return Err(HistoryError::MismatchedCompletion {
                            client: r.client.clone(),
                            id: r.id,
                        });
                    }
                    op.outcome = match kind {
                        OpType::Ok => Outcome::Ok(r.value),
                        OpType::Fail => Outcome::Fail,
                        _ => Outcome::Info,
                    };
                    op.complete_seq = Some(r.seq);
                }
            }
        }
        Ok(History {
            operations: slots,
            completeness,
        })
    }

    /// Operations ordered by invocation sequence number.
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn completeness(&self) -> Completeness {
        self.completeness
    }

    /// The same operations, flagged incomplete.
    pub fn marked_incomplete(&self) -> History {
        History {
            operations: self.operations.clone(),
            completeness: Completeness::Incomplete,
        }
    }

    /// Keys in sorted order.
    pub fn keys(&self) -> Vec<&str> {
        let set: BTreeSet<&str> = self.operations.iter().map(|o| o.key.as_str()).collect();
        set.into_iter().collect()
    }

    /// Operations on one key, ordered by invocation.
    pub fn operations_for_key<'a>(
        &'a self,
        key: &'a str,
    ) -> impl Iterator<Item = &'a Operation> + 'a {
        self.operations.iter().filter(move |o| o.key == key)
    }
}

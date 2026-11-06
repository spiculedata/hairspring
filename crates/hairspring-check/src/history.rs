//! The in-memory operation history the checkers consume, and the seam that builds it.
//!
//! Guest history records (one per recorded operation event) are not parsed here. A parser produces [`OpRecord`]s, and
//! [`History::from_records`] pairs invokes with completions and checks the structure. Nothing else in this crate knows
//! about the wire form.
//!
//! An empty history is valid. Every checker answers `Ok` for it when it is complete and the final state (if any) is
//! empty, and `Unknown` when it is flagged incomplete.

use std::collections::{BTreeMap, BTreeSet};

/// Upper bound on records accepted by [`History::from_records`]. Callers are expected to bound their own input far lower (for example 200,000 lines);
/// this only stops unbounded memory use from a caller that skipped its own limit.
pub const MAX_RECORDS: usize = 1_000_000;

/// The `type` of a history record.
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

/// One history record, field for field. No validation beyond types: [`History::from_records`] does that.
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
    /// An invoke with no completion record (the client never reported back).
    pub fn is_pending(&self) -> bool {
        self.complete_seq.is_none()
    }
}

/// Whether the history is known to be whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    /// A client died without finishing, or records were dropped. Every checker answers `Unknown`.
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
    /// A client invoked an operation while an earlier one of its own had not completed (`id` is the second one).
    /// A client runs one operation at a time; an operation that timed out must be completed as `info` first.
    ClientOverlap {
        client: String,
        id: u64,
    },
    /// A completion whose invoke exists but has a higher sequence number: the sequence numbers disagree with the
    /// client's own order.
    CompletionBeforeInvoke {
        client: String,
        id: u64,
    },
    /// A completion with no invoke at all from the same client and id.
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
            HistoryError::ClientOverlap { id, .. } => {
                write!(
                    f,
                    "operation {id} invoked while the client had another open"
                )
            }
            HistoryError::CompletionBeforeInvoke { id, .. } => {
                write!(
                    f,
                    "operation {id} completed before its invoke in sequence order"
                )
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

/// Pairing state while records are applied in sequence order.
#[derive(Default)]
struct Pairing {
    slots: Vec<Operation>,
    index: BTreeMap<(String, u64), usize>,
    open: BTreeMap<String, u64>,
}

impl Pairing {
    fn invoke(&mut self, r: &OpRecord) -> Result<(), HistoryError> {
        let id = (r.client.clone(), r.id);
        if self.index.contains_key(&id) {
            return Err(HistoryError::DuplicateInvoke {
                client: r.client.clone(),
                id: r.id,
            });
        }
        if self.open.contains_key(&r.client) {
            return Err(HistoryError::ClientOverlap {
                client: r.client.clone(),
                id: r.id,
            });
        }
        self.open.insert(r.client.clone(), r.id);
        self.index.insert(id, self.slots.len());
        self.slots.push(Operation {
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
        Ok(())
    }

    fn complete(&mut self, r: &OpRecord, invoked_later: bool) -> Result<(), HistoryError> {
        let (client, id) = (r.client.clone(), r.id);
        let Some(&at) = self.index.get(&(client.clone(), id)) else {
            return Err(if invoked_later {
                HistoryError::CompletionBeforeInvoke { client, id }
            } else {
                HistoryError::CompletionWithoutInvoke { client, id }
            });
        };
        let op = &mut self.slots[at];
        if op.complete_seq.is_some() {
            return Err(HistoryError::DuplicateCompletion { client, id });
        }
        if op.f != r.f || op.key != r.key {
            return Err(HistoryError::MismatchedCompletion { client, id });
        }
        op.outcome = match r.kind {
            OpType::Ok => Outcome::Ok(r.value),
            OpType::Fail => Outcome::Fail,
            _ => Outcome::Info,
        };
        op.complete_seq = Some(r.seq);
        self.open.remove(&client);
        Ok(())
    }
}

impl History {
    /// Pair `invoke` records with completions (by client and id), ordering by `seq` first, so the input order does
    /// not matter. Errors are explicit: too many records, duplicate `seq`, an id invoked twice, a client invoking
    /// while it has another operation open, a completion with no invoke (or an invoke with a higher `seq`), a second
    /// completion, or a completion that names another function or key. An invoke with no completion is kept as a
    /// pending operation.
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
        let invoked: BTreeSet<(&str, u64)> = ordered
            .iter()
            .filter(|r| r.kind == OpType::Invoke)
            .map(|r| (r.client.as_str(), r.id))
            .collect();
        let mut pairing = Pairing::default();
        for r in ordered {
            match r.kind {
                OpType::Invoke => pairing.invoke(r)?,
                _ => pairing.complete(r, invoked.contains(&(r.client.as_str(), r.id)))?,
            }
        }
        Ok(History {
            operations: pairing.slots,
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

    /// Operations on one key, ordered by invocation. Scans every operation: to visit many keys use
    /// [`History::operations_by_key`].
    pub fn operations_for_key<'a>(
        &'a self,
        key: &'a str,
    ) -> impl Iterator<Item = &'a Operation> + 'a {
        self.operations.iter().filter(move |o| o.key == key)
    }

    /// Every key with its operations ordered by invocation, grouped in one pass (keys in sorted order).
    pub fn operations_by_key(&self) -> BTreeMap<&str, Vec<&Operation>> {
        let mut groups: BTreeMap<&str, Vec<&Operation>> = BTreeMap::new();
        for op in &self.operations {
            groups.entry(op.key.as_str()).or_default().push(op);
        }
        groups
    }
}

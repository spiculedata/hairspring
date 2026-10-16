//! The run catalog: every property id a run has seen, with a small stable index that trial digests refer to.
//!
//! The sender (the code that judges trials) builds the catalog with [`RunCatalog::record`] and ships the changes as
//! [`CatalogUpdate`]s next to each digest. The receiver rebuilds it with [`RunCatalog::apply`], which re-checks every
//! rule, so a catalog is as trustworthy as its checks and not as its sender.

use crate::digest::{
    Decl, DigestEntry, DigestFlags, TrialDigest, MAX_DIGEST_DECLS, MAX_DIGEST_WITNESSES,
};
use crate::event::{AssertKind, Loc};
use crate::limits::{MAX_ID_BYTES, MAX_LOC_FILE_BYTES, MAX_LOC_FUNC_BYTES, MAX_MSG_BYTES};
use crate::trial::{failure_basis, TrialProps};
use crate::valid::{is_ident, is_plain_text};
use std::collections::HashMap;
use std::fmt;

/// What a run knows about one id.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CatalogRow {
    pub id: String,
    pub kind: AssertKind,
    pub msg: String,
    pub loc: Option<Loc>,
    /// False while only inconclusive trials have mentioned the id: their declarations are not trusted, and the first
    /// conclusive trial replaces them.
    pub decided: bool,
}

/// One new or re-decided row, with the index it occupies.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CatalogUpdate {
    pub idx: u32,
    pub row: CatalogRow,
}

/// Why a catalog update was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "catalog: {}", self.0)
    }
}

impl std::error::Error for CatalogError {}

fn refuse<T>(why: impl Into<String>) -> Result<T, CatalogError> {
    Err(CatalogError(why.into()))
}

impl CatalogRow {
    /// Re-checks every guest-supplied field against the protocol's own limits.
    pub fn validate(&self) -> Result<(), CatalogError> {
        if !is_ident(&self.id, MAX_ID_BYTES) {
            return refuse("an id is empty, too long or has characters outside the id set");
        }
        if !is_plain_text(&self.msg, MAX_MSG_BYTES) {
            return refuse("a message is too long or has control or format characters");
        }
        if let Some(loc) = &self.loc {
            let func_ok = loc
                .func
                .as_deref()
                .is_none_or(|f| is_plain_text(f, MAX_LOC_FUNC_BYTES));
            if !is_plain_text(&loc.file, MAX_LOC_FILE_BYTES) || !func_ok {
                return refuse("a location is too long or has control or format characters");
            }
        }
        Ok(())
    }
}

/// The ids of a run, in first-seen order.
#[derive(Debug, Clone, Default)]
pub struct RunCatalog {
    rows: Vec<CatalogRow>,
    index: HashMap<String, u32>,
    /// Declarations already attached to kind-mismatch entries, run-wide.
    decls_used: usize,
}

/// A trial recorded against a catalog: the catalog afterwards, the digest, and the rows that changed.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub catalog: RunCatalog,
    pub digest: TrialDigest,
    pub updates: Vec<CatalogUpdate>,
}

impl RunCatalog {
    pub fn new() -> RunCatalog {
        RunCatalog::default()
    }

    /// A catalog loaded from stored rows (index = position). Ids must be unique and valid.
    pub fn from_rows(rows: Vec<CatalogRow>) -> Result<RunCatalog, CatalogError> {
        let mut index = HashMap::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            row.validate()?;
            if index.insert(row.id.clone(), i as u32).is_some() {
                return refuse("two rows have the same id");
            }
        }
        Ok(RunCatalog {
            rows,
            index,
            decls_used: 0,
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self) -> &[CatalogRow] {
        &self.rows
    }

    pub fn get(&self, idx: u32) -> Option<&CatalogRow> {
        self.rows.get(idx as usize)
    }

    pub fn index_of(&self, id: &str) -> Option<u32> {
        self.index.get(id).copied()
    }

    /// Applies one update from an untrusted sender. A new row must take the next index; an existing row may only be
    /// decided once, keeping its id.
    pub fn apply(
        mut self,
        update: &CatalogUpdate,
        id_cap: usize,
    ) -> Result<RunCatalog, CatalogError> {
        update.row.validate()?;
        let idx = update.idx as usize;
        if idx == self.rows.len() {
            if idx >= id_cap {
                return refuse(format!("a run may track at most {id_cap} ids"));
            }
            if self.index.contains_key(&update.row.id) {
                return refuse("an id was added twice");
            }
            self.index.insert(update.row.id.clone(), update.idx);
            self.rows.push(update.row.clone());
            return Ok(self);
        }
        let Some(existing) = self.rows.get(idx) else {
            return refuse("an update skips an index");
        };
        if existing.decided {
            return refuse("a decided row cannot change");
        }
        if existing.id != update.row.id {
            return refuse("a decision cannot rename an id");
        }
        if !update.row.decided {
            return refuse("an update to an existing row must decide it");
        }
        self.rows[idx] = update.row.clone();
        Ok(self)
    }

    /// Records one trial: assigns indexes to new ids (up to `id_cap` ids in the run), lets the first conclusive trial
    /// decide an id's declaration, and builds the trial's digest. Entries follow the trial's own property order,
    /// then any conflict-only entries.
    #[must_use]
    pub fn record(self, trial: &TrialProps, inconclusive: bool, id_cap: usize) -> Recorded {
        let mut cat = self;
        let mut updates = Vec::new();
        let mut entries: Vec<DigestEntry> = Vec::with_capacity(trial.properties.len());
        let mut witnesses = 0usize;
        let mut dropped = 0u64;
        for p in &trial.properties {
            let Some(idx) = cat.place(p, inconclusive, id_cap, &mut updates) else {
                dropped += 1;
                continue;
            };
            let row_kind = cat.rows[idx as usize].kind;
            let failing = failure_basis(p.kind, p.hits, p.falses).is_some();
            let witness = p
                .failure()
                .and_then(|f| f.witness)
                .filter(|_| failing && witnesses < MAX_DIGEST_WITNESSES);
            witnesses += usize::from(witness.is_some());
            let mismatch = !inconclusive && row_kind != p.kind;
            let decl = (mismatch && cat.decls_used < MAX_DIGEST_DECLS).then(|| Decl {
                msg: p.msg.clone(),
                loc: p.loc.clone(),
            });
            cat.decls_used += usize::from(decl.is_some());
            entries.push(DigestEntry {
                idx,
                kind: p.kind,
                catalogued: true,
                conflicted: false,
                hits: p.hits,
                trues: p.trues,
                falses: p.falses,
                witness,
                decl,
            });
        }
        for c in &trial.conflicts {
            let Some(idx) = cat.index_of(&c.id) else {
                continue;
            };
            match entries.iter_mut().find(|e| e.idx == idx) {
                Some(e) => e.conflicted = true,
                None => entries.push(DigestEntry {
                    idx,
                    kind: cat.rows[idx as usize].kind,
                    catalogued: false,
                    conflicted: true,
                    hits: 0,
                    trues: 0,
                    falses: 0,
                    witness: None,
                    decl: None,
                }),
            }
        }
        let digest = TrialDigest {
            flags: DigestFlags {
                inconclusive,
                degraded: trial.is_degraded(),
                perturbing: trial.perturbing,
                report_missing: !trial.report_missing().is_empty(),
                has_conflicts: !trial.conflicts.is_empty(),
                excluded: false,
            },
            ids_dropped: dropped,
            entries,
        };
        Recorded {
            catalog: cat,
            digest,
            updates,
        }
    }

    /// The index of a property's id, adding or deciding its row as the fold rules require. `None` when the run is
    /// already tracking `id_cap` ids and this one is new.
    fn place(
        &mut self,
        p: &crate::trial::TrialProperty,
        inconclusive: bool,
        id_cap: usize,
        updates: &mut Vec<CatalogUpdate>,
    ) -> Option<u32> {
        let row = |decided| CatalogRow {
            id: p.id.clone(),
            kind: p.kind,
            msg: p.msg.clone(),
            loc: p.loc.clone(),
            decided,
        };
        match self.index.get(&p.id).copied() {
            Some(idx) => {
                if !self.rows[idx as usize].decided && !inconclusive {
                    self.rows[idx as usize] = row(true);
                    updates.push(CatalogUpdate {
                        idx,
                        row: row(true),
                    });
                }
                Some(idx)
            }
            None if self.rows.len() >= id_cap => None,
            None => {
                let idx = self.rows.len() as u32;
                self.index.insert(p.id.clone(), idx);
                self.rows.push(row(!inconclusive));
                updates.push(CatalogUpdate {
                    idx,
                    row: row(!inconclusive),
                });
                Some(idx)
            }
        }
    }
}

//! Retention on SQLite: the reads and deletes behind [`crate::retention`]. See that module for the rules.
use crate::retention::*;
use crate::sqlite_tx::write_tx;
use crate::types::{Result, RunState};
use rusqlite::{params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};

/// The file whose lock is the retention lock of the database at `db`.
pub(crate) fn lock_path(db: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{db}-gc.lock"))
}

/// Takes the retention lock without waiting: an exclusive lock on a file next to the database, held while the file
/// stays open (the operating system releases it if the process ends). The file is left in place afterwards: removing
/// it could let two processes lock two different files of the same name. An in-memory database has no file, and no
/// other process can open it, so there is nothing to lock.
pub(crate) fn try_lock(db: Option<&str>) -> Result<Option<RetentionLock>> {
    let Some(db) = db.filter(|p| !p.is_empty()) else {
        return Ok(Some(RetentionLock::new(Box::new(()))));
    };
    let path = lock_path(db);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| {
            crate::types::StoreError::Backend(format!("cannot open {}: {e}", path.display()))
        })?;
    match file.try_lock() {
        Ok(()) => Ok(Some(RetentionLock::new(Box::new(file)))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(crate::types::StoreError::Backend(format!(
            "cannot lock {}: {e}",
            path.display()
        ))),
    }
}

pub(crate) fn scan(conn: &Connection, cutoff_ms: i64) -> Result<Vec<RetentionCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, state, COALESCE(finished_at, created_at) AS aged FROM runs \
         WHERE COALESCE(finished_at, created_at) < ? ORDER BY aged, id",
    )?;
    let rows = stmt
        .query_map([cutoff_ms], |r| {
            let state: String = r.get(1)?;
            Ok(RetentionCandidate {
                id: r.get(0)?,
                // A state this build does not know is never treated as finished.
                state: RunState::parse(&state).unwrap_or(RunState::Running),
                aged_from: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes (or, for a dry run, only counts) in the way the trait documents. A dry run is one read transaction: in WAL
/// mode it reads one snapshot and never takes the write lock, so writers carry on meanwhile (its working lists are in
/// temporary tables, which are private to the connection). An apply is one write transaction per batch of whole units,
/// so a failure leaves every earlier batch fully deleted and every later one untouched.
pub(crate) fn delete(
    conn: &mut Connection,
    run_ids: &[String],
    cutoff_ms: i64,
    dry_run: bool,
) -> Result<RetentionReport> {
    let pass = Pass {
        named: run_ids.iter().map(String::as_str).collect(),
        cutoff_ms,
    };
    let list = anchors(run_ids);
    let mut report = RetentionReport {
        applied: !dry_run,
        ..RetentionReport::default()
    };
    let mut done = BTreeSet::new();
    let mut next = 0;
    create_work_tables(conn)?;
    if dry_run {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut ops = Ops::new(&tx, true);
        process_batch(
            &mut ops,
            &pass,
            &list,
            &mut next,
            usize::MAX,
            &mut done,
            &mut report,
        )?;
        tx.rollback()?;
        return Ok(report);
    }
    while next < list.len() {
        let tx = write_tx(conn)?;
        let mut batch = RetentionReport::default();
        let outcome = process_batch(
            &mut Ops::new(&tx, false),
            &pass,
            &list,
            &mut next,
            BATCH_RUNS,
            &mut done,
            &mut batch,
        )
        .and_then(|()| Ok(tx.commit()?));
        match outcome {
            Ok(()) => report.absorb(batch),
            Err(e) => {
                report.error = Some(e.to_string());
                break;
            }
        }
    }
    Ok(report)
}

/// The connection's own lists of candidate blobs and of runs a dry run counted as removed. Temporary tables live
/// outside the database file, so filling them takes no lock on it.
fn create_work_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS gc_candidates (h TEXT PRIMARY KEY); \
         CREATE TEMP TABLE IF NOT EXISTS gc_gone (id TEXT PRIMARY KEY);",
    )?;
    Ok(())
}

struct Ops<'a> {
    tx: &'a Transaction<'a>,
    /// A dry run: read only, and count instead of delete.
    dry: bool,
    /// The runs a dry run counted as removed; their rows still exist, so they must not count as references.
    gone: Vec<String>,
}

impl<'a> Ops<'a> {
    fn new(tx: &'a Transaction<'a>, dry: bool) -> Self {
        Ops {
            tx,
            dry,
            gone: Vec::new(),
        }
    }

    fn strings(&self, sql: &str, arg: &str) -> Result<Vec<String>> {
        let mut stmt = self.tx.prepare_cached(sql)?;
        let rows = stmt
            .query_map([arg], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Deletes the rows of `table` that match `cond`, or for a dry run counts them.
    fn remove<P: rusqlite::Params>(&self, table: &str, cond: &str, args: P) -> Result<u64> {
        if self.dry {
            let n: i64 = self.tx.query_row(
                &format!("SELECT count(*) FROM {table} WHERE {cond}"),
                args,
                |r| r.get(0),
            )?;
            return Ok(n.max(0) as u64);
        }
        Ok(self
            .tx
            .execute(&format!("DELETE FROM {table} WHERE {cond}"), args)? as u64)
    }

    /// The blobs a run's rows point at, and whether each is a failure bundle.
    fn blobs_of_run(&self, run: &str, into: &mut BTreeMap<String, bool>) -> Result<()> {
        for h in self.strings(
            "SELECT detail_ref FROM trials WHERE run_id=?1 AND detail_ref IS NOT NULL \
             UNION SELECT output_ref FROM trials WHERE run_id=?1 AND output_ref IS NOT NULL",
            run,
        )? {
            into.entry(h).or_insert(false);
        }
        for h in self.strings("SELECT bundle_hash FROM failures WHERE run_id=?", run)? {
            into.insert(h, true);
        }
        Ok(())
    }

    /// Replaces the contents of a work table with `values`.
    fn fill(&self, table: &str, column: &str, values: &[String]) -> Result<()> {
        self.tx.execute(&format!("DELETE FROM temp.{table}"), [])?;
        let mut stmt = self.tx.prepare_cached(&format!(
            "INSERT OR IGNORE INTO temp.{table}({column}) VALUES (?)"
        ))?;
        for v in values {
            stmt.execute([v])?;
        }
        Ok(())
    }

    fn column_values(&self, sql: &str, columns: usize, into: &mut BTreeSet<String>) -> Result<()> {
        let mut stmt = self.tx.prepare(sql)?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            for i in 0..columns {
                if let Some(h) = r.get::<_, Option<String>>(i)? {
                    into.insert(h);
                }
            }
        }
        Ok(())
    }
}

impl RetentionOps for Ops<'_> {
    fn run_facts(&mut self, id: &str) -> Result<Option<RunFacts>> {
        let row = self
            .tx
            .query_row(
                "SELECT state, COALESCE(finished_at, created_at), \
                 EXISTS (SELECT 1 FROM jobs WHERE run_id=?1 AND status IN ('queued','running')) \
                 FROM runs WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(state, aged_from, has_active_job)| RunFacts {
            state: RunState::parse(&state).unwrap_or(RunState::Running),
            aged_from,
            has_active_job,
        }))
    }

    fn tree_of_run(&mut self, id: &str) -> Result<Option<String>> {
        Ok(self
            .tx
            .query_row(
                "SELECT id FROM trees WHERE id=?1 UNION SELECT tree_id FROM tree_branches WHERE run_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn lock_tree(&mut self, tree_id: &str) -> Result<bool> {
        // An apply's transaction holds SQLite's one write lock already, and a dry run reads one snapshot; nothing can
        // add to the tree in between.
        Ok(self
            .tx
            .query_row("SELECT 1 FROM trees WHERE id=?", [tree_id], |_| Ok(()))
            .optional()?
            .is_some())
    }

    fn tree_members(&mut self, tree_id: &str) -> Result<Vec<String>> {
        self.strings(
            "SELECT id FROM trees WHERE id=?1 UNION SELECT run_id FROM tree_branches WHERE tree_id=?1 \
             UNION SELECT run_id FROM trial_lineage WHERE tree_id=?1 ORDER BY 1",
            tree_id,
        )
    }

    fn delete_unit(&mut self, tree_id: Option<&str>, runs: &[String]) -> Result<UnitRemoved> {
        let mut out = UnitRemoved::default();
        for run in runs {
            self.blobs_of_run(run, &mut out.blobs)?;
            out.checks += self.remove(
                "failure_checks",
                "failure_id IN (SELECT id FROM failures WHERE run_id=?)",
                [run],
            )?;
        }
        if let Some(tree) = tree_id {
            // One statement per table, so a node whose parent is another member's node goes in the same statement.
            let marks = vec!["?"; runs.len()].join(",");
            let args = || std::iter::once(tree).chain(runs.iter().map(String::as_str));
            let mut stmt = self.tx.prepare(&format!(
                "SELECT edge_ref FROM trial_lineage WHERE tree_id=? AND run_id IN ({marks}) AND edge_ref IS NOT NULL"
            ))?;
            let edges = stmt
                .query_map(params_from_iter(args()), |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for h in edges {
                out.blobs.entry(h).or_insert(false);
            }
            let in_unit = format!("tree_id=? AND run_id IN ({marks})");
            out.nodes += self.remove("trial_lineage", &in_unit, params_from_iter(args()))?;
            self.remove("tree_branches", &in_unit, params_from_iter(args()))?;
            // Numbered, because the tree and the runs are each used twice.
            let numbered = (2..=runs.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let outside: i64 = self.tx.query_row(
                &format!(
                    "SELECT (SELECT count(*) FROM trial_lineage WHERE tree_id=?1 AND run_id NOT IN ({numbered})) \
                     + (SELECT count(*) FROM tree_branches WHERE tree_id=?1 AND run_id NOT IN ({numbered}))"
                ),
                params_from_iter(args()),
                |r| r.get(0),
            )?;
            if outside > 0 {
                return Err(tree_changed(tree));
            }
            self.remove("trees", "id=?", [tree])?;
        }
        for run in runs {
            for table in [
                "run_coverage",
                "run_digest_use",
                "run_properties",
                "run_catalog",
                "run_identity",
                "run_inputs",
            ] {
                self.remove(table, "run_id=?", [run])?;
            }
            out.trials += self.remove("trials", "run_id=?", [run])?;
            out.failures += self.remove("failures", "run_id=?", [run])?;
            out.jobs += self.remove("jobs", "run_id=?", [run])?;
            out.events += self.remove("events", "run_id=?", [run])?;
            self.remove("runs", "id=?", [run])?;
        }
        if self.dry {
            self.gone.extend(runs.iter().cloned());
        }
        Ok(out)
    }

    fn lock_blobs(&mut self) -> Result<()> {
        // An apply holds the one write lock, so no blob or trial can be half stored; a dry run takes nothing.
        Ok(())
    }

    fn referenced(&mut self, candidates: &[String]) -> Result<BTreeSet<String>> {
        // `x IN temp.table` is a lookup in the work table's index, so each table is read once whatever the number of
        // candidates. Nothing indexes these columns except tree edges: the trials pass is a whole-table read.
        self.fill("gc_candidates", "h", candidates)?;
        self.fill("gc_gone", "id", &self.gone)?;
        let mut found = BTreeSet::new();
        self.column_values(
            "SELECT output_ref, detail_ref FROM trials \
             WHERE (output_ref IN temp.gc_candidates OR detail_ref IN temp.gc_candidates) \
             AND run_id NOT IN temp.gc_gone",
            2,
            &mut found,
        )?;
        self.column_values(
            "SELECT bundle_hash FROM failures WHERE bundle_hash IN temp.gc_candidates \
             AND run_id NOT IN temp.gc_gone \
             UNION SELECT edge_ref FROM trial_lineage WHERE edge_ref IN temp.gc_candidates \
             AND run_id NOT IN temp.gc_gone",
            1,
            &mut found,
        )?;
        // A matching trial row's other column may name a blob that is not a candidate.
        let wanted: BTreeSet<&str> = candidates.iter().map(String::as_str).collect();
        found.retain(|h| wanted.contains(h.as_str()));
        let rest: Vec<String> = candidates
            .iter()
            .filter(|h| !found.contains(*h))
            .cloned()
            .collect();
        if !rest.is_empty() {
            let mut stmt = self.tx.prepare(
                "SELECT fc.artefacts_json FROM failure_checks fc WHERE NOT EXISTS \
                 (SELECT 1 FROM failures f WHERE f.id = fc.failure_id AND f.run_id IN temp.gc_gone)",
            )?;
            let texts = stmt.query_map([], |r| r.get::<_, String>(0))?;
            found.extend(named_in_texts(&rest, texts.map(|t| Ok(t?)))?);
        }
        Ok(found)
    }

    fn remove_blobs(&mut self, hashes: &[String]) -> Result<Vec<(String, u64)>> {
        let mut removed = Vec::new();
        let mut size = self
            .tx
            .prepare_cached("SELECT length(data) FROM blobs WHERE hash=?")?;
        for hash in hashes {
            let Some(bytes) = size.query_row([hash], |r| r.get::<_, i64>(0)).optional()? else {
                continue;
            };
            if !self.dry {
                self.tx.execute("DELETE FROM blobs WHERE hash=?", [hash])?;
            }
            removed.push((hash.clone(), bytes.max(0) as u64));
        }
        Ok(removed)
    }
}

#[cfg(test)]
#[path = "sqlite_retention_tests.rs"]
mod tests;

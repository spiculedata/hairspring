//! Retention on SQLite: the reads and deletes behind [`crate::retention`]. See that module for the rules.
use crate::retention::*;
use crate::sqlite_tx::write_tx;
use crate::types::{Result, RunState};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};
use std::collections::{BTreeMap, BTreeSet};

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

/// Deletes (or, for a dry run, only counts) in the way the trait documents. A dry run is one transaction that is rolled
/// back; an apply is one transaction per tree or run, so a failure leaves every earlier unit fully deleted and every
/// later one untouched.
pub(crate) fn delete(
    conn: &mut Connection,
    run_ids: &[String],
    cutoff_ms: i64,
    dry_run: bool,
) -> Result<RetentionReport> {
    let named: BTreeSet<&str> = run_ids.iter().map(String::as_str).collect();
    let mut report = RetentionReport {
        applied: !dry_run,
        ..RetentionReport::default()
    };
    let mut done = BTreeSet::new();
    if dry_run {
        let tx = write_tx(conn)?;
        let mut ops = Ops { tx: &tx };
        for anchor in anchors(run_ids) {
            let mut unit = RetentionReport::default();
            process_anchor(&mut ops, anchor, &named, cutoff_ms, &mut done, &mut unit)?;
            report.absorb(unit);
        }
        tx.rollback()?;
        return Ok(report);
    }
    for anchor in anchors(run_ids) {
        if done.contains(anchor) {
            continue;
        }
        let tx = write_tx(conn)?;
        let mut unit = RetentionReport::default();
        let outcome = process_anchor(
            &mut Ops { tx: &tx },
            anchor,
            &named,
            cutoff_ms,
            &mut done,
            &mut unit,
        )
        .and_then(|()| Ok(tx.commit()?));
        match outcome {
            Ok(()) => report.absorb(unit),
            Err(e) => {
                report.error = Some(e.to_string());
                break;
            }
        }
    }
    Ok(report)
}

struct Ops<'a> {
    tx: &'a Transaction<'a>,
}

impl Ops<'_> {
    fn strings(&self, sql: &str, arg: &str) -> Result<Vec<String>> {
        let mut stmt = self.tx.prepare_cached(sql)?;
        let rows = stmt
            .query_map([arg], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    fn run(&self, sql: &str, arg: &str) -> Result<u64> {
        Ok(self.tx.execute(sql, [arg])? as u64)
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

    /// Removes the blob when nothing that remains refers to it: no trial row, failure, tree edge, or stored check that
    /// names its hash. Returns its size when it was removed.
    fn remove_if_unreferenced(&self, hash: &str) -> Result<Option<u64>> {
        let size: Option<i64> = self
            .tx
            .query_row(
                "SELECT length(data) FROM blobs WHERE hash=?1 \
                 AND NOT EXISTS (SELECT 1 FROM trials WHERE detail_ref=?1 OR output_ref=?1) \
                 AND NOT EXISTS (SELECT 1 FROM failures WHERE bundle_hash=?1) \
                 AND NOT EXISTS (SELECT 1 FROM trial_lineage WHERE edge_ref=?1) \
                 AND NOT EXISTS (SELECT 1 FROM failure_checks WHERE instr(artefacts_json, ?1) > 0)",
                [hash],
                |r| r.get(0),
            )
            .optional()?;
        let Some(size) = size else { return Ok(None) };
        self.tx.execute("DELETE FROM blobs WHERE hash=?", [hash])?;
        Ok(Some(size.max(0) as u64))
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
        // The transaction holds SQLite's one write lock already; nothing else can add to the tree meanwhile.
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
        let mut blobs: BTreeMap<String, bool> = BTreeMap::new();
        for run in runs {
            self.blobs_of_run(run, &mut blobs)?;
            out.checks += self.run(
                "DELETE FROM failure_checks WHERE failure_id IN (SELECT id FROM failures WHERE run_id=?)",
                run,
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
                blobs.entry(h).or_insert(false);
            }
            out.nodes += self.tx.execute(
                &format!("DELETE FROM trial_lineage WHERE tree_id=? AND run_id IN ({marks})"),
                params_from_iter(args()),
            )? as u64;
            self.tx.execute(
                &format!("DELETE FROM tree_branches WHERE tree_id=? AND run_id IN ({marks})"),
                params_from_iter(args()),
            )?;
            let left: i64 = self.tx.query_row(
                "SELECT (SELECT count(*) FROM trial_lineage WHERE tree_id=?1) \
                 + (SELECT count(*) FROM tree_branches WHERE tree_id=?1)",
                [tree],
                |r| r.get(0),
            )?;
            if left > 0 {
                return Err(tree_changed(tree));
            }
            self.run("DELETE FROM trees WHERE id=?", tree)?;
        }
        for run in runs {
            for table in [
                "run_coverage",
                "run_digest_use",
                "run_properties",
                "run_catalog",
                "run_identity",
            ] {
                self.run(&format!("DELETE FROM {table} WHERE run_id=?"), run)?;
            }
            out.trials += self.run("DELETE FROM trials WHERE run_id=?", run)?;
            out.failures += self.run("DELETE FROM failures WHERE run_id=?", run)?;
            out.jobs += self.run("DELETE FROM jobs WHERE run_id=?", run)?;
            out.events += self.run("DELETE FROM events WHERE run_id=?", run)?;
            self.tx
                .execute("DELETE FROM runs WHERE id=?", params![run])?;
        }
        for (hash, is_bundle) in blobs {
            if let Some(bytes) = self.remove_if_unreferenced(&hash)? {
                out.blobs.push(RemovedBlob {
                    hash,
                    bytes,
                    is_bundle,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
#[path = "sqlite_retention_tests.rs"]
mod tests;

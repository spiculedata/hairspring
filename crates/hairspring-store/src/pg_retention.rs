//! Retention on PostgreSQL: the reads and deletes behind [`crate::retention`]. See that module for the rules. The
//! statements are the SQLite ones, so both backends keep and delete the same things.
use crate::retention::*;
use crate::types::{Result, RunState};
use postgres::{Client, Transaction};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn scan(c: &mut Client, cutoff_ms: i64) -> Result<Vec<RetentionCandidate>> {
    let rows = c.query(
        "SELECT id, state, COALESCE(finished_at, created_at) AS aged FROM runs \
         WHERE COALESCE(finished_at, created_at) < $1 ORDER BY aged, id",
        &[&cutoff_ms],
    )?;
    Ok(rows
        .iter()
        .map(|r| {
            let state: String = r.get(1);
            RetentionCandidate {
                id: r.get(0),
                // A state this build does not know is never treated as finished.
                state: RunState::parse(&state).unwrap_or(RunState::Running),
                aged_from: r.get(2),
            }
        })
        .collect())
}

/// Deletes (or, for a dry run, only counts) in the way the trait documents. A dry run is one transaction that is rolled
/// back; an apply is one transaction per tree or run, so a failure leaves every earlier unit fully deleted and every
/// later one untouched.
pub(crate) fn delete(
    c: &mut Client,
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
        let mut tx = c.transaction()?;
        for anchor in anchors(run_ids) {
            let mut unit = RetentionReport::default();
            let mut ops = Ops { tx: &mut tx };
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
        let mut unit = RetentionReport::default();
        let outcome = apply_unit(c, anchor, &named, cutoff_ms, &mut done, &mut unit);
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

fn apply_unit(
    c: &mut Client,
    anchor: &str,
    named: &BTreeSet<&str>,
    cutoff_ms: i64,
    done: &mut BTreeSet<String>,
    unit: &mut RetentionReport,
) -> Result<()> {
    let mut tx = c.transaction()?;
    process_anchor(
        &mut Ops { tx: &mut tx },
        anchor,
        named,
        cutoff_ms,
        done,
        unit,
    )?;
    tx.commit()?;
    Ok(())
}

struct Ops<'a, 'b> {
    tx: &'a mut Transaction<'b>,
}

impl Ops<'_, '_> {
    fn strings(&mut self, sql: &str, arg: &str) -> Result<Vec<String>> {
        Ok(self
            .tx
            .query(sql, &[&arg])?
            .iter()
            .map(|r| r.get(0))
            .collect())
    }

    fn run(&mut self, sql: &str, arg: &str) -> Result<u64> {
        Ok(self.tx.execute(sql, &[&arg])?)
    }

    /// The blobs a run's rows point at, and whether each is a failure bundle.
    fn blobs_of_run(&mut self, run: &str, into: &mut BTreeMap<String, bool>) -> Result<()> {
        for h in self.strings(
            "SELECT detail_ref FROM trials WHERE run_id=$1 AND detail_ref IS NOT NULL \
             UNION SELECT output_ref FROM trials WHERE run_id=$1 AND output_ref IS NOT NULL",
            run,
        )? {
            into.entry(h).or_insert(false);
        }
        for h in self.strings("SELECT bundle_hash FROM failures WHERE run_id=$1", run)? {
            into.insert(h, true);
        }
        Ok(())
    }

    /// Removes the blob when nothing that remains refers to it: no trial row, failure, tree edge, or stored check that
    /// names its hash. Returns its size when it was removed.
    fn remove_if_unreferenced(&mut self, hash: &str) -> Result<Option<u64>> {
        let row = self.tx.query_opt(
            "SELECT octet_length(data)::BIGINT FROM blobs WHERE hash=$1 \
             AND NOT EXISTS (SELECT 1 FROM trials WHERE detail_ref=$1 OR output_ref=$1) \
             AND NOT EXISTS (SELECT 1 FROM failures WHERE bundle_hash=$1) \
             AND NOT EXISTS (SELECT 1 FROM trial_lineage WHERE edge_ref=$1) \
             AND NOT EXISTS (SELECT 1 FROM failure_checks WHERE position($1 in artefacts_json) > 0)",
            &[&hash],
        )?;
        let Some(row) = row else { return Ok(None) };
        let size: i64 = row.get(0);
        self.tx
            .execute("DELETE FROM blobs WHERE hash=$1", &[&hash])?;
        Ok(Some(size.max(0) as u64))
    }
}

impl RetentionOps for Ops<'_, '_> {
    fn run_facts(&mut self, id: &str) -> Result<Option<RunFacts>> {
        // Same order as finish_job (jobs, then the run), so the two can never deadlock.
        self.tx
            .execute("SELECT id FROM jobs WHERE run_id=$1 FOR UPDATE", &[&id])?;
        let row = self.tx.query_opt(
            "SELECT state, COALESCE(finished_at, created_at), \
             EXISTS (SELECT 1 FROM jobs WHERE run_id=$1 AND status IN ('queued','running')) \
             FROM runs WHERE id=$1 FOR UPDATE",
            &[&id],
        )?;
        Ok(row.map(|r| {
            let state: String = r.get(0);
            RunFacts {
                state: RunState::parse(&state).unwrap_or(RunState::Running),
                aged_from: r.get(1),
                has_active_job: r.get(2),
            }
        }))
    }

    fn tree_of_run(&mut self, id: &str) -> Result<Option<String>> {
        Ok(self
            .tx
            .query_opt(
                "SELECT id FROM trees WHERE id=$1 UNION SELECT tree_id FROM tree_branches WHERE run_id=$1",
                &[&id],
            )?
            .map(|r| r.get(0)))
    }

    fn tree_members(&mut self, tree_id: &str) -> Result<Vec<String>> {
        self.strings(
            "SELECT id FROM trees WHERE id=$1 UNION SELECT run_id FROM tree_branches WHERE tree_id=$1 \
             UNION SELECT run_id FROM trial_lineage WHERE tree_id=$1 ORDER BY 1",
            tree_id,
        )
    }

    fn delete_unit(&mut self, tree_id: Option<&str>, runs: &[String]) -> Result<UnitRemoved> {
        let mut out = UnitRemoved::default();
        let mut blobs: BTreeMap<String, bool> = BTreeMap::new();
        for run in runs {
            self.blobs_of_run(run, &mut blobs)?;
            out.checks += self.run(
                "DELETE FROM failure_checks WHERE failure_id IN (SELECT id FROM failures WHERE run_id=$1)",
                run,
            )?;
        }
        if let Some(tree) = tree_id {
            for h in self.strings(
                "SELECT edge_ref FROM trial_lineage WHERE tree_id=$1 AND edge_ref IS NOT NULL",
                tree,
            )? {
                blobs.entry(h).or_insert(false);
            }
            out.nodes += self.run("DELETE FROM trial_lineage WHERE tree_id=$1", tree)?;
            self.run("DELETE FROM tree_branches WHERE tree_id=$1", tree)?;
            self.run("DELETE FROM trees WHERE id=$1", tree)?;
        }
        for run in runs {
            for table in [
                "run_coverage",
                "run_digest_use",
                "run_properties",
                "run_catalog",
                "run_identity",
            ] {
                self.run(&format!("DELETE FROM {table} WHERE run_id=$1"), run)?;
            }
            out.trials += self.run("DELETE FROM trials WHERE run_id=$1", run)?;
            out.failures += self.run("DELETE FROM failures WHERE run_id=$1", run)?;
            out.jobs += self.run("DELETE FROM jobs WHERE run_id=$1", run)?;
            out.events += self.run("DELETE FROM events WHERE run_id=$1", run)?;
            self.run("DELETE FROM runs WHERE id=$1", run)?;
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

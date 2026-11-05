//! Retention on PostgreSQL: the reads and deletes behind [`crate::retention`]. See that module for the rules. The
//! statements are the SQLite ones, so both backends keep and delete the same things.
use crate::retention::*;
use crate::types::{Result, RunState};
use postgres::fallible_iterator::FallibleIterator;
use postgres::{Client, IsolationLevel, Transaction};
use std::collections::{BTreeMap, BTreeSet};

/// The first key of the retention advisory lock; the second is a hash of the schema, so stores in different schemas
/// of one database do not wait for each other.
const GC_LOCK_CLASS: i32 = 0x6863_6763; // "hcgc"

/// A pooled connection that holds the retention advisory lock for its session; dropping it unlocks it before the
/// connection goes back to the pool. If the unlock fails the connection is broken, and its session, with the lock,
/// ends with it.
struct GcLock(r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager<postgres::NoTls>>);

impl Drop for GcLock {
    fn drop(&mut self) {
        let _ = self.0.execute(
            "SELECT pg_advisory_unlock($1, hashtext(current_schema()))",
            &[&GC_LOCK_CLASS],
        );
    }
}

/// Takes the retention lock without waiting, on a connection of its own that is kept until the lock is dropped.
pub(crate) fn try_lock(
    mut c: r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager<postgres::NoTls>>,
) -> Result<Option<RetentionLock>> {
    let got: bool = c
        .query_one(
            "SELECT pg_try_advisory_lock($1, hashtext(current_schema()))",
            &[&GC_LOCK_CLASS],
        )?
        .get(0);
    Ok(got.then(|| RetentionLock::new(Box::new(GcLock(c)))))
}

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

/// Deletes (or, for a dry run, only counts) in the way the trait documents. A dry run is one read-only transaction on
/// one snapshot (`REPEATABLE READ`), which takes no row or table lock a writer waits for. An apply is one transaction
/// per batch of whole units, so a failure leaves every earlier batch fully deleted and every later one untouched.
pub(crate) fn delete(
    c: &mut Client,
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
    if dry_run {
        let mut tx = c
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()?;
        let mut ops = Ops::new(&mut tx, true);
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
        let mut batch = RetentionReport::default();
        match apply_batch(c, &pass, &list, &mut next, &mut done, &mut batch) {
            Ok(()) => report.absorb(batch),
            Err(e) => {
                report.error = Some(e.to_string());
                break;
            }
        }
    }
    Ok(report)
}

fn apply_batch(
    c: &mut Client,
    pass: &Pass,
    list: &[&str],
    next: &mut usize,
    done: &mut BTreeSet<String>,
    batch: &mut RetentionReport,
) -> Result<()> {
    let mut tx = c.transaction()?;
    process_batch(
        &mut Ops::new(&mut tx, false),
        pass,
        list,
        next,
        BATCH_RUNS,
        done,
        batch,
    )?;
    tx.commit()?;
    Ok(())
}

struct Ops<'a, 'b> {
    tx: &'a mut Transaction<'b>,
    /// A dry run: read only, and count instead of delete.
    dry: bool,
    /// The runs a dry run counted as removed; their rows still exist, so they must not count as references.
    gone: Vec<String>,
}

impl<'a, 'b> Ops<'a, 'b> {
    fn new(tx: &'a mut Transaction<'b>, dry: bool) -> Self {
        Ops {
            tx,
            dry,
            gone: Vec::new(),
        }
    }

    fn strings(&mut self, sql: &str, arg: &str) -> Result<Vec<String>> {
        Ok(self
            .tx
            .query(sql, &[&arg])?
            .iter()
            .map(|r| r.get(0))
            .collect())
    }

    /// Deletes the rows of `table` that match `cond` (one parameter, `$1`), or for a dry run counts them.
    fn remove(&mut self, table: &str, cond: &str, arg: &str) -> Result<u64> {
        if self.dry {
            let n: i64 = self
                .tx
                .query_one(
                    &format!("SELECT count(*) FROM {table} WHERE {cond}"),
                    &[&arg],
                )?
                .get(0);
            return Ok(n.max(0) as u64);
        }
        Ok(self
            .tx
            .execute(&format!("DELETE FROM {table} WHERE {cond}"), &[&arg])?)
    }

    /// As [`Ops::remove`], for a condition on the tree (`$1`) and the unit's runs (`$2`).
    fn remove_in_tree(&mut self, table: &str, tree: &str, runs: &[&str]) -> Result<u64> {
        let cond = "tree_id=$1 AND run_id = ANY($2)";
        if self.dry {
            let n: i64 = self
                .tx
                .query_one(
                    &format!("SELECT count(*) FROM {table} WHERE {cond}"),
                    &[&tree, &runs],
                )?
                .get(0);
            return Ok(n.max(0) as u64);
        }
        Ok(self.tx.execute(
            &format!("DELETE FROM {table} WHERE {cond}"),
            &[&tree, &runs],
        )?)
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

    /// Rows of the tree that belong to runs outside the unit. With the tree locked (or, for a dry run, read from one
    /// snapshot) there are none; if there are, nothing of the unit may go.
    fn tree_rows_outside(&mut self, tree: &str, runs: &[&str]) -> Result<i64> {
        Ok(self
            .tx
            .query_one(
                "SELECT (SELECT count(*) FROM trial_lineage WHERE tree_id=$1 AND NOT run_id = ANY($2)) \
                 + (SELECT count(*) FROM tree_branches WHERE tree_id=$1 AND NOT run_id = ANY($2))",
                &[&tree, &runs],
            )?
            .get(0))
    }
}

impl RetentionOps for Ops<'_, '_> {
    fn run_facts(&mut self, id: &str) -> Result<Option<RunFacts>> {
        // Same order as finish_job (jobs, then the run), so the two can never deadlock. A dry run locks nothing.
        let lock = if self.dry { "" } else { " FOR UPDATE" };
        if !self.dry {
            self.tx
                .execute("SELECT id FROM jobs WHERE run_id=$1 FOR UPDATE", &[&id])?;
        }
        let row = self.tx.query_opt(
            &format!(
                "SELECT state, COALESCE(finished_at, created_at), \
                 EXISTS (SELECT 1 FROM jobs WHERE run_id=$1 AND status IN ('queued','running')) \
                 FROM runs WHERE id=$1{lock}"
            ),
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

    fn lock_tree(&mut self, tree_id: &str) -> Result<bool> {
        // FOR NO KEY UPDATE conflicts with the FOR UPDATE that add_branch and add_node take on the tree, so neither
        // can add to the tree until this transaction ends, but not with the key-share locks foreign keys take. A dry
        // run reads one snapshot and locks nothing.
        let lock = if self.dry { "" } else { " FOR NO KEY UPDATE" };
        Ok(self
            .tx
            .query_opt(
                &format!("SELECT id FROM trees WHERE id=$1{lock}"),
                &[&tree_id],
            )?
            .is_some())
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
        for run in runs {
            self.blobs_of_run(run, &mut out.blobs)?;
            out.checks += self.remove(
                "failure_checks",
                "failure_id IN (SELECT id FROM failures WHERE run_id=$1)",
                run,
            )?;
        }
        if let Some(tree) = tree_id {
            let members: Vec<&str> = runs.iter().map(String::as_str).collect();
            for r in self.tx.query(
                "SELECT edge_ref FROM trial_lineage WHERE tree_id=$1 AND run_id = ANY($2) AND edge_ref IS NOT NULL",
                &[&tree, &members],
            )? {
                out.blobs.entry(r.get(0)).or_insert(false);
            }
            out.nodes += self.remove_in_tree("trial_lineage", tree, &members)?;
            self.remove_in_tree("tree_branches", tree, &members)?;
            if self.tree_rows_outside(tree, &members)? > 0 {
                return Err(tree_changed(tree));
            }
            self.remove("trees", "id=$1", tree)?;
        }
        for run in runs {
            for table in [
                "run_coverage",
                "run_digest_use",
                "run_properties",
                "run_catalog",
                "run_identity",
            ] {
                self.remove(table, "run_id=$1", run)?;
            }
            out.trials += self.remove("trials", "run_id=$1", run)?;
            out.failures += self.remove("failures", "run_id=$1", run)?;
            out.jobs += self.remove("jobs", "run_id=$1", run)?;
            out.events += self.remove("events", "run_id=$1", run)?;
            self.remove("runs", "id=$1", run)?;
        }
        if self.dry {
            self.gone.extend(runs.iter().cloned());
        }
        Ok(out)
    }

    fn lock_blobs(&mut self) -> Result<()> {
        if self.dry {
            return Ok(());
        }
        // Wait for every transaction that is storing a blob to end, and keep new ones out until this one ends. An
        // ingest stores its blob (ON CONFLICT DO NOTHING, which locks no row) and the trial naming it in one
        // transaction; until it commits, the reference checks cannot see that trial.
        self.tx
            .batch_execute("LOCK TABLE blobs IN SHARE ROW EXCLUSIVE MODE")?;
        Ok(())
    }

    fn referenced(&mut self, candidates: &[String]) -> Result<BTreeSet<String>> {
        // `= ANY($1)` over a bound array is a hashed lookup, so each table is read once whatever the number of
        // candidates. Nothing indexes these columns: the trials pass is a whole-table read.
        let gone = &self.gone;
        let mut found = BTreeSet::new();
        for r in self.tx.query(
            "SELECT output_ref, detail_ref FROM trials \
             WHERE (output_ref = ANY($1) OR detail_ref = ANY($1)) AND NOT run_id = ANY($2)",
            &[&candidates, gone],
        )? {
            found.extend(r.get::<_, Option<String>>(0));
            found.extend(r.get::<_, Option<String>>(1));
        }
        for r in self.tx.query(
            "SELECT bundle_hash FROM failures WHERE bundle_hash = ANY($1) AND NOT run_id = ANY($2) \
             UNION SELECT edge_ref FROM trial_lineage WHERE edge_ref = ANY($1) AND NOT run_id = ANY($2)",
            &[&candidates, gone],
        )? {
            found.insert(r.get::<_, String>(0));
        }
        // A matching trial row's other column may name a blob that is not a candidate.
        let wanted: BTreeSet<&str> = candidates.iter().map(String::as_str).collect();
        found.retain(|h| wanted.contains(h.as_str()));
        let rest: Vec<String> = candidates
            .iter()
            .filter(|h| !found.contains(*h))
            .cloned()
            .collect();
        if rest.is_empty() {
            return Ok(found);
        }
        let texts = self.tx.query_raw(
            "SELECT fc.artefacts_json FROM failure_checks fc WHERE NOT EXISTS \
             (SELECT 1 FROM failures f WHERE f.id = fc.failure_id AND f.run_id = ANY($1))",
            [gone],
        )?;
        let texts = texts.map(|r| Ok(r.get::<_, String>(0))).iterator();
        found.extend(named_in_texts(
            &rest,
            texts.map(|t| t.map_err(crate::types::StoreError::from)),
        )?);
        Ok(found)
    }

    fn remove_blobs(&mut self, hashes: &[String]) -> Result<Vec<(String, u64)>> {
        if hashes.is_empty() {
            return Ok(Vec::new());
        }
        let sql = if self.dry {
            "SELECT hash, octet_length(data)::BIGINT FROM blobs WHERE hash = ANY($1)"
        } else {
            "DELETE FROM blobs WHERE hash = ANY($1) RETURNING hash, octet_length(data)::BIGINT"
        };
        Ok(self
            .tx
            .query(sql, &[&hashes])?
            .iter()
            .map(|r| (r.get(0), r.get::<_, i64>(1).max(0) as u64))
            .collect())
    }
}

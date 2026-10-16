//! Offline downgrade of a SQLite database from schema version 3 to version 2. See [`crate::downgrade`].
use crate::downgrade::*;
use crate::types::{Result, StoreError};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Every branch run: marked `mode = 'branch'`, or registered with a tree.
fn branch_runs(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM runs WHERE mode='branch' UNION SELECT run_id FROM tree_branches ORDER BY 1",
    )?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

fn edge_blobs(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT edge_ref FROM trial_lineage WHERE edge_ref IS NOT NULL")?;
    let hashes = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hashes)
}

fn blobs_of_run(conn: &Connection, run_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT detail_ref FROM trials WHERE run_id=?1 AND detail_ref IS NOT NULL \
         UNION SELECT output_ref FROM trials WHERE run_id=?1 AND output_ref IS NOT NULL \
         UNION SELECT bundle_hash FROM failures WHERE run_id=?1",
    )?;
    let hashes = stmt
        .query_map([run_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(hashes)
}

fn delete_run(conn: &Connection, run_id: &str) -> Result<()> {
    for table in RUN_OWNED_TABLES {
        conn.execute(&format!("DELETE FROM {table} WHERE run_id=?"), [run_id])?;
    }
    conn.execute("DELETE FROM runs WHERE id=?", [run_id])?;
    Ok(())
}

/// Removes the blobs among `hashes` that no remaining trial row or failure refers to.
fn collect_orphans(conn: &Connection, hashes: &[String]) -> Result<u64> {
    let mut removed = 0u64;
    for h in hashes {
        removed += conn.execute(
            "DELETE FROM blobs WHERE hash=?1 AND NOT EXISTS (SELECT 1 FROM trials WHERE detail_ref=?1 OR output_ref=?1) \
             AND NOT EXISTS (SELECT 1 FROM failures WHERE bundle_hash=?1)",
            [h],
        )? as u64;
    }
    Ok(removed)
}

fn apply(
    conn: &Connection,
    branch_runs: &[String],
    opts: &DowngradeOptions,
) -> Result<(Vec<String>, u64)> {
    let mut candidates = edge_blobs(conn)?;
    if opts.drop_branch_runs {
        for run in branch_runs {
            candidates.extend(blobs_of_run(conn, run)?);
        }
    }
    for table in V3_TABLES_DROP_ORDER {
        conn.execute_batch(&format!("DROP TABLE {table}"))?;
    }
    let mut dropped = Vec::new();
    if opts.drop_branch_runs {
        for run in branch_runs {
            delete_run(conn, run)?;
            dropped.push(run.clone());
        }
    }
    let blobs = collect_orphans(conn, &candidates)?;
    conn.pragma_update(None, "user_version", DOWNGRADE_TARGET_VERSION)?;
    Ok((dropped, blobs))
}

/// Downgrades the database file at `path` to schema version 2 in one transaction. The file must exist. Other
/// processes should not be using it: the downgrade holds the write lock, but readers of the old schema would fail
/// afterwards. A database already at version 2 is left alone.
pub fn downgrade_sqlite(path: &Path, opts: &DowngradeOptions) -> Result<DowngradeReport> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let outcome = downgrade_in_transaction(&conn, opts);
    match outcome {
        Ok((report, true)) => {
            conn.execute_batch("COMMIT")?;
            Ok(report)
        }
        Ok((report, false)) => {
            conn.execute_batch("ROLLBACK")?;
            Ok(report)
        }
        Err(e) => {
            // The original error is the useful one.
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The report, and whether the transaction should be committed.
fn downgrade_in_transaction(
    conn: &Connection,
    opts: &DowngradeOptions,
) -> Result<(DowngradeReport, bool)> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version == DOWNGRADE_TARGET_VERSION {
        let report = DowngradeReport {
            from_version: version,
            to_version: version,
            ..DowngradeReport::default()
        };
        return Ok((report, false));
    }
    if version != DOWNGRADE_TARGET_VERSION + 1 {
        return Err(StoreError::Backend(format!(
            "cannot downgrade a database at schema version {version} (only version {} can be downgraded to {})",
            DOWNGRADE_TARGET_VERSION + 1,
            DOWNGRADE_TARGET_VERSION
        )));
    }
    let branches = branch_runs(conn)?;
    let mut report = DowngradeReport {
        from_version: version,
        to_version: DOWNGRADE_TARGET_VERSION,
        branch_runs: branches.clone(),
        ..DowngradeReport::default()
    };
    // A dry run does the work for real inside the transaction (schema changes are transactional) so the counts are
    // true, then the caller rolls it back.
    let (dropped_runs, dropped_blobs) = apply(conn, &branches, opts)?;
    report.dropped_runs = dropped_runs;
    report.dropped_blobs = dropped_blobs;
    report.applied = !opts.dry_run;
    Ok((report, !opts.dry_run))
}

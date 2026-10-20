//! Offline downgrade of a PostgreSQL schema from version 3 or 4 to version 2. See [`crate::downgrade`].
use crate::downgrade::*;
use crate::pg::{valid_schema, MIGRATION_LOCK};
use crate::types::{Result, StoreError};
use postgres::{Config, NoTls, Transaction};

fn strings(
    tx: &mut Transaction,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
) -> Result<Vec<String>> {
    Ok(tx.query(sql, params)?.iter().map(|r| r.get(0)).collect())
}

/// Every branch run: marked `mode = 'branch'`, or registered with a tree.
fn branch_runs(tx: &mut Transaction) -> Result<Vec<String>> {
    strings(
        tx,
        "SELECT id FROM runs WHERE mode='branch' UNION SELECT run_id FROM tree_branches ORDER BY 1",
        &[],
    )
}

fn blobs_of_run(tx: &mut Transaction, run_id: &str) -> Result<Vec<String>> {
    strings(
        tx,
        "SELECT detail_ref FROM trials WHERE run_id=$1 AND detail_ref IS NOT NULL \
         UNION SELECT output_ref FROM trials WHERE run_id=$1 AND output_ref IS NOT NULL \
         UNION SELECT bundle_hash FROM failures WHERE run_id=$1",
        &[&run_id],
    )
}

fn delete_run(tx: &mut Transaction, run_id: &str) -> Result<()> {
    for table in RUN_OWNED_TABLES {
        tx.execute(&format!("DELETE FROM {table} WHERE run_id=$1"), &[&run_id])?;
    }
    tx.execute("DELETE FROM runs WHERE id=$1", &[&run_id])?;
    Ok(())
}

/// Removes the blobs among `hashes` that no remaining trial row or failure refers to.
fn collect_orphans(tx: &mut Transaction, hashes: &[String]) -> Result<u64> {
    let mut removed = 0u64;
    for h in hashes {
        removed += tx.execute(
            "DELETE FROM blobs WHERE hash=$1 AND NOT EXISTS (SELECT 1 FROM trials WHERE detail_ref=$1 OR output_ref=$1) \
             AND NOT EXISTS (SELECT 1 FROM failures WHERE bundle_hash=$1)",
            &[h],
        )?;
    }
    Ok(removed)
}

fn apply(
    tx: &mut Transaction,
    version: i64,
    branches: &[String],
    opts: &DowngradeOptions,
) -> Result<(Vec<String>, u64)> {
    let mut candidates = strings(
        tx,
        "SELECT DISTINCT edge_ref FROM trial_lineage WHERE edge_ref IS NOT NULL",
        &[],
    )?;
    if opts.drop_branch_runs {
        for run in branches {
            candidates.extend(blobs_of_run(tx, run)?);
        }
    }
    if version >= FIRST_VERSION_WITH_V4_TABLES {
        for table in V4_TABLES_DROP_ORDER {
            tx.batch_execute(&format!("DROP TABLE {table}"))?;
        }
    }
    for table in V3_TABLES_DROP_ORDER {
        tx.batch_execute(&format!("DROP TABLE {table}"))?;
    }
    let mut dropped = Vec::new();
    if opts.drop_branch_runs {
        for run in branches {
            delete_run(tx, run)?;
            dropped.push(run.clone());
        }
    }
    let blobs = collect_orphans(tx, &candidates)?;
    tx.execute(
        "UPDATE schema_version SET version=$1",
        &[&(DOWNGRADE_TARGET_VERSION as i32)],
    )?;
    Ok((dropped, blobs))
}

/// Downgrades `schema` of the database at `url` to schema version 2 in one transaction, under the same advisory lock
/// the migration takes. The schema must exist. A schema already at version 2 is left alone.
pub fn downgrade_postgres(
    url: &str,
    schema: &str,
    opts: &DowngradeOptions,
) -> Result<DowngradeReport> {
    if !valid_schema(schema) {
        return Err(StoreError::Backend("invalid schema name".to_string()));
    }
    // Parse errors are reported without echoing the URL (it contains the password).
    let mut cfg: Config = url
        .parse()
        .map_err(|_| StoreError::Backend("invalid database URL".to_string()))?;
    cfg.options(&format!("-c search_path=\"{schema}\""));
    let mut client = cfg.connect(NoTls)?;
    let mut tx = client.transaction()?;
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])?;
    let version: i32 = tx
        .query_opt("SELECT version FROM schema_version LIMIT 1", &[])?
        .map(|r| r.get(0))
        .ok_or_else(|| StoreError::Backend("the schema has no recorded version".to_string()))?;
    let version = i64::from(version);
    if version == DOWNGRADE_TARGET_VERSION {
        return Ok(DowngradeReport {
            from_version: version,
            to_version: version,
            ..DowngradeReport::default()
        });
    }
    if !DOWNGRADE_SOURCE_VERSIONS.contains(&version) {
        return Err(StoreError::Backend(format!(
            "cannot downgrade a schema at version {version} (only versions {DOWNGRADE_SOURCE_VERSIONS:?} can be downgraded to {DOWNGRADE_TARGET_VERSION})"
        )));
    }
    let branches = branch_runs(&mut tx)?;
    let mut report = DowngradeReport {
        from_version: version,
        to_version: DOWNGRADE_TARGET_VERSION,
        branch_runs: branches.clone(),
        ..DowngradeReport::default()
    };
    // A dry run does the work for real inside the transaction (schema changes are transactional) so the counts are
    // true, then drops the transaction without committing.
    let (dropped_runs, dropped_blobs) = apply(&mut tx, version, &branches, opts)?;
    report.dropped_runs = dropped_runs;
    report.dropped_blobs = dropped_blobs;
    if opts.dry_run {
        return Ok(report);
    }
    report.applied = true;
    tx.commit()?;
    Ok(report)
}

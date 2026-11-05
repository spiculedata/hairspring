//! Filtered, paged lists of runs and failures on SQLite. See [`crate::listing`].
use crate::listing::{failure_clause, run_clause, window, FailureFilter, RunFilter};
use crate::sqlite::{failure_from_row, run_from_row};
use crate::types::{FailureRecord, Result, RunRecord};
use rusqlite::{params_from_iter, Connection};

fn marker(n: usize) -> String {
    format!("?{n}")
}

pub(crate) fn list_runs(
    conn: &Connection,
    filter: &RunFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<RunRecord>> {
    let clause = run_clause(filter, marker);
    let sql = format!(
        "SELECT * FROM runs{} ORDER BY created_at DESC, rowid DESC{}",
        clause.sql,
        window(offset, limit)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(clause.args.iter()), run_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn list_failures(
    conn: &Connection,
    filter: &FailureFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<FailureRecord>> {
    let clause = failure_clause(filter, marker);
    let sql = format!(
        "SELECT * FROM failures{} ORDER BY created_at DESC, rowid DESC{}",
        clause.sql,
        window(offset, limit)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(clause.args.iter()), failure_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

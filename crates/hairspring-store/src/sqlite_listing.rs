//! Filtered, paged lists of runs and failures on SQLite. See [`crate::listing`].
use crate::listing::{
    failure_clause, run_clause, window, Clause, FailureDialect, FailureFilter, RunFilter,
};
use crate::sqlite::{failure_from_row, run_from_row};
use crate::types::{FailureRecord, Result, RunRecord};
use rusqlite::{params_from_iter, Connection};

/// The column that orders rows stored in the same millisecond.
const ORDER: &str = "rowid";

const DIALECT: FailureDialect = FailureDialect {
    marker,
    order: ORDER,
    postgres: false,
};

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
        "SELECT * FROM runs{} ORDER BY created_at DESC, {ORDER} DESC{}",
        clause.sql,
        window(offset, limit)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(clause.args.iter()), run_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn failures_statement(filter: &FailureFilter, offset: u64, limit: u32) -> (String, Clause) {
    let clause = failure_clause(filter, &DIALECT);
    let sql = format!(
        "SELECT * FROM failures{} ORDER BY created_at DESC, {ORDER} DESC{}",
        clause.sql,
        window(offset, limit)
    );
    (sql, clause)
}

pub(crate) fn list_failures(
    conn: &Connection,
    filter: &FailureFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<FailureRecord>> {
    let (sql, clause) = failures_statement(filter, offset, limit);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(clause.args.iter()), failure_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The steps of SQLite's plan for [`list_failures`], one line each (`EXPLAIN QUERY PLAN`), for tests that assert it.
pub(crate) fn explain_failures(
    conn: &Connection,
    filter: &FailureFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<String>> {
    let (sql, clause) = failures_statement(filter, offset, limit);
    // `EXPLAIN QUERY PLAN` is planned from the schema this connection last read, which may predate a change made
    // through another connection. Reading a table first makes SQLite notice such a change and read the schema again.
    conn.execute_batch("SELECT 1 FROM failures LIMIT 0")?;
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
    let steps = stmt
        .query_map(params_from_iter(clause.args.iter()), |r| {
            r.get::<_, String>(3)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(steps)
}

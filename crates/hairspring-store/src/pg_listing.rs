//! Filtered, paged lists of runs and failures on PostgreSQL. See [`crate::listing`].
use crate::listing::{failure_clause, run_clause, window, Clause, FailureFilter, RunFilter};
use crate::pg::{failure_from_row, run_from_row};
use crate::types::{FailureRecord, Result, RunRecord};
use postgres::types::ToSql;
use postgres::{Client, Row};

fn marker(n: usize) -> String {
    format!("${n}")
}

fn query<T>(c: &mut Client, sql: &str, clause: &Clause, map: fn(&Row) -> T) -> Result<Vec<T>> {
    let args: Vec<&(dyn ToSql + Sync)> = clause
        .args
        .iter()
        .map(|a| a as &(dyn ToSql + Sync))
        .collect();
    Ok(c.query(sql, &args)?.iter().map(map).collect())
}

pub(crate) fn list_runs(
    c: &mut Client,
    filter: &RunFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<RunRecord>> {
    let clause = run_clause(filter, marker);
    let sql = format!(
        "SELECT * FROM runs{} ORDER BY created_at DESC, seq DESC{}",
        clause.sql,
        window(offset, limit)
    );
    query(c, &sql, &clause, run_from_row)
}

pub(crate) fn list_failures(
    c: &mut Client,
    filter: &FailureFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<FailureRecord>> {
    let clause = failure_clause(filter, marker);
    let sql = format!(
        "SELECT * FROM failures{} ORDER BY created_at DESC, seq DESC{}",
        clause.sql,
        window(offset, limit)
    );
    query(c, &sql, &clause, failure_from_row)
}

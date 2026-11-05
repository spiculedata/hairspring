//! Filtered, paged lists of runs and failures on PostgreSQL. See [`crate::listing`].
use crate::listing::{
    failure_clause, failure_order, run_clause, window, Clause, FailureDialect, FailureFilter,
    RunFilter,
};
use crate::pg::{failure_from_row, run_from_row};
use crate::types::{FailureRecord, Result, RunRecord};
use postgres::types::ToSql;
use postgres::{Client, Row};

/// The column that orders rows stored in the same millisecond.
const ORDER: &str = "seq";

const DIALECT: FailureDialect = FailureDialect {
    marker,
    order: ORDER,
    postgres: true,
};

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
        "SELECT * FROM runs{} ORDER BY created_at DESC, {ORDER} DESC{}",
        clause.sql,
        window(offset, limit)
    );
    query(c, &sql, &clause, run_from_row)
}

fn failures_statement(filter: &FailureFilter, offset: u64, limit: u32) -> (String, Clause) {
    let clause = failure_clause(filter, &DIALECT);
    let sql = format!(
        "SELECT * FROM failures{}{}{}",
        clause.sql,
        failure_order(filter, ORDER),
        window(offset, limit)
    );
    (sql, clause)
}

pub(crate) fn list_failures(
    c: &mut Client,
    filter: &FailureFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<FailureRecord>> {
    let (sql, clause) = failures_statement(filter, offset, limit);
    query(c, &sql, &clause, failure_from_row)
}

/// PostgreSQL's plan for [`list_failures`] with the real bound values, one line each (`EXPLAIN (COSTS OFF)`, which
/// plans the statement without running it), for tests that assert it.
pub(crate) fn explain_failures(
    c: &mut Client,
    filter: &FailureFilter,
    offset: u64,
    limit: u32,
) -> Result<Vec<String>> {
    let (sql, clause) = failures_statement(filter, offset, limit);
    query(c, &format!("EXPLAIN (COSTS OFF) {sql}"), &clause, |r| {
        r.get::<_, String>(0)
    })
}

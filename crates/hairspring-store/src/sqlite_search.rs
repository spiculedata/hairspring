//! Bounded search on SQLite. See [`crate::search`].
use crate::search::{search, Arg, Cell, Dialect, Execute, Row, SearchHits, SearchQuery, Shape};
use crate::types::Result;
use rusqlite::types::ToSql;
use rusqlite::Connection;

const DIALECT: Dialect = Dialect {
    marker: "?",
    order: "rowid",
    lateral: false,
    bytewise: "",
};

struct Sqlite<'a>(&'a Connection);

fn param(arg: &Arg) -> &dyn ToSql {
    match arg {
        Arg::Text(t) => t,
        Arg::Int(n) => n,
    }
}

impl Execute for Sqlite<'_> {
    fn rows(&mut self, sql: &str, args: &[Arg], shape: Shape) -> Result<Vec<Row>> {
        let mut stmt = self.0.prepare(sql)?;
        let params: Vec<&dyn ToSql> = args.iter().map(param).collect();
        let mapped = stmt.query_map(params.as_slice(), |row| {
            shape
                .chars()
                .enumerate()
                .map(|(i, kind)| match kind {
                    'n' => row.get::<_, i64>(i).map(Cell::Int),
                    _ => row.get::<_, String>(i).map(Cell::Text),
                })
                .collect::<rusqlite::Result<Row>>()
        })?;
        Ok(mapped.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

pub(crate) fn run(conn: &Connection, q: &SearchQuery) -> Result<SearchHits> {
    search(q, &DIALECT, &mut Sqlite(conn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::property_statement;

    fn plan(conn: &Connection, sql: &str, args: &[&str]) -> Vec<String> {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        stmt.query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    /// The cost of the property probe must not depend on how many catalog rows a guest put in a run: each run is one
    /// seek into the `(run_id, prop_id)` index, read in index order, and stopped by its `LIMIT`. A full scan, a
    /// temporary sort, or a seek on the run id alone would read every row of the run.
    #[test]
    fn the_property_probe_is_one_range_of_the_run_and_prop_index_with_no_sort() {
        let conn = Connection::open_in_memory().unwrap();
        crate::sqlite_schema::migrate(&conn).unwrap();
        let sql = property_statement(&DIALECT, "db.", true, 6);
        let steps = plan(&conn, &sql, &["run_x", "db.", "db/"]);
        assert_eq!(steps.len(), 1, "{steps:?}");
        assert!(
            steps[0].starts_with("SEARCH run_catalog USING ")
                && steps[0].contains("INDEX ")
                && steps[0].ends_with("(run_id=? AND prop_id>? AND prop_id<?)"),
            "{steps:?}"
        );
        assert!(
            !steps.iter().any(|s| s.contains("TEMP B-TREE")),
            "{steps:?}"
        );
    }
}

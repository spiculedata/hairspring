//! Bounded search on SQLite. See [`crate::search`].
use crate::search::{search, Arg, Cell, Dialect, Execute, Row, SearchHits, SearchQuery, Shape};
use crate::types::Result;
use rusqlite::types::ToSql;
use rusqlite::Connection;

const DIALECT: Dialect = Dialect {
    marker: "?1",
    order: "rowid",
    lateral: false,
};

struct Sqlite<'a>(&'a Connection);

impl Execute for Sqlite<'_> {
    fn rows(&mut self, sql: &str, arg: &Arg, shape: Shape) -> Result<Vec<Row>> {
        let mut stmt = self.0.prepare(sql)?;
        let param: &dyn ToSql = match arg {
            Arg::Text(t) => t,
            Arg::Int(n) => n,
        };
        let mapped = stmt.query_map([param], |row| {
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

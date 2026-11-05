//! Bounded search on PostgreSQL. See [`crate::search`].
use crate::search::{search, Arg, Cell, Dialect, Execute, Row, SearchHits, SearchQuery, Shape};
use crate::types::Result;
use postgres::types::ToSql;
use postgres::Client;

const DIALECT: Dialect = Dialect {
    marker: "$1",
    order: "seq",
    lateral: true,
};

struct Pg<'a>(&'a mut Client);

impl Execute for Pg<'_> {
    fn rows(&mut self, sql: &str, arg: &Arg, shape: Shape) -> Result<Vec<Row>> {
        let param: &(dyn ToSql + Sync) = match arg {
            Arg::Text(t) => t,
            Arg::Int(n) => n,
        };
        Ok(self
            .0
            .query(sql, &[param])?
            .iter()
            .map(|row| {
                shape
                    .chars()
                    .enumerate()
                    .map(|(i, kind)| match kind {
                        'n' => Cell::Int(row.get::<_, i64>(i)),
                        _ => Cell::Text(row.get::<_, String>(i)),
                    })
                    .collect()
            })
            .collect())
    }
}

pub(crate) fn run(client: &mut Client, q: &SearchQuery) -> Result<SearchHits> {
    search(q, &DIALECT, &mut Pg(client))
}

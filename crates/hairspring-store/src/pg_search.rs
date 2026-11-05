//! Bounded search on PostgreSQL. See [`crate::search`].
use crate::search::{search, Arg, Cell, Dialect, Execute, Row, SearchHits, SearchQuery, Shape};
use crate::types::Result;
use postgres::types::ToSql;
use postgres::Client;

const DIALECT: Dialect = Dialect {
    marker: "$",
    order: "seq",
    lateral: true,
    bytewise: " COLLATE \"C\"",
};

struct Pg<'a>(&'a mut Client);

fn param(arg: &Arg) -> &(dyn ToSql + Sync) {
    match arg {
        Arg::Text(t) => t,
        Arg::Int(n) => n,
    }
}

impl Execute for Pg<'_> {
    fn rows(&mut self, sql: &str, args: &[Arg], shape: Shape) -> Result<Vec<Row>> {
        let params: Vec<&(dyn ToSql + Sync)> = args.iter().map(param).collect();
        Ok(self
            .0
            .query(sql, &params)?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::property_statement;

    /// A database whose default collation follows a locale would order `n.Z` after `n.a`, and could even leave a
    /// matching id outside the range; SQLite compares bytewise, so PostgreSQL must be told to as well.
    #[test]
    fn the_property_range_and_its_order_are_bytewise() {
        let sql = property_statement(&DIALECT, "db.", true, 6);
        assert!(
            sql.contains(r#"prop_id COLLATE "C" >= $2 AND prop_id COLLATE "C" < $3"#),
            "{sql}"
        );
        assert!(
            sql.ends_with(r#"ORDER BY prop_id COLLATE "C" LIMIT 6"#),
            "{sql}"
        );
        assert!(sql.contains("WHERE run_id = $1"), "{sql}");
    }
}

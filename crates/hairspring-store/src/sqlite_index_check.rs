//! Whether an index that already exists under a version 7 index's name is the index the migration would create.
//!
//! `CREATE INDEX IF NOT EXISTS` goes by name, so the check has to look at the table, uniqueness, the columns with their
//! collation and direction (all from SQLite's pragmas) and, for a partial index, the predicate (from the stored
//! statement, which SQLite offers no pragma for). A genuine SQLite error, such as a busy database, is an error, never a
//! "different definition".
use crate::types::Result;
use rusqlite::{Connection, OptionalExtension};

/// The collation an index column has when nothing says otherwise, and the one the version 7 indexes use.
const DEFAULT_COLLATION: &str = "BINARY";

/// A key column of an index: its name (`None` for an expression), whether it is descending, and its collation.
type KeyColumn = (Option<String>, bool, String);

pub(crate) fn sqlite_index_fits(
    conn: &Connection,
    index: &str,
    table: &str,
    columns: &[&str],
    predicate: Option<&str>,
) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT tbl_name, sql FROM sqlite_master WHERE type='index' AND name=?",
            [index],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((on, Some(sql))) = found else {
        return Ok(false);
    };
    if on != table {
        return Ok(false);
    }
    let unique = conn
        .query_row(
            "SELECT \"unique\" FROM pragma_index_list(?1) WHERE name=?2",
            [table, index],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    if unique != Some(0) {
        return Ok(false);
    }
    let wanted: Vec<KeyColumn> = columns
        .iter()
        .map(|c| (Some((*c).to_owned()), false, DEFAULT_COLLATION.to_owned()))
        .collect();
    if key_columns(conn, index)? != wanted {
        return Ok(false);
    }
    let normalised = normalise_outside_literals(&sql);
    let where_clause = normalised.split_once(")where").map(|(_, w)| w);
    Ok(where_clause == predicate)
}

/// The key columns of an index in order.
fn key_columns(conn: &Connection, index: &str) -> Result<Vec<KeyColumn>> {
    let mut stmt = conn.prepare(
        "SELECT name, \"desc\", coll FROM pragma_index_xinfo(?1) WHERE key=1 ORDER BY seqno",
    )?;
    let rows = stmt
        .query_map([index], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, i64>(1)? != 0,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// Lower-cases the statement and drops white space and comments, but only outside quotes: a string literal is kept
/// exactly (case and spaces count), and a quoted name loses its quotes (`"status"` is `status`).
fn normalise_outside_literals(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => copy_literal(&mut chars, &mut out),
            '"' | '`' => copy_name(&mut chars, c, &mut out),
            '[' => copy_name(&mut chars, ']', &mut out),
            '-' if chars.peek() == Some(&'-') => skip_line_comment(&mut chars),
            '/' if chars.peek() == Some(&'*') => skip_block_comment(&mut chars),
            c if c.is_whitespace() => {}
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

fn skip_line_comment(chars: &mut Chars<'_>) {
    for c in chars.by_ref() {
        if c == '\n' {
            break;
        }
    }
}

fn skip_block_comment(chars: &mut Chars<'_>) {
    chars.next();
    let mut previous = ' ';
    for c in chars.by_ref() {
        if previous == '*' && c == '/' {
            break;
        }
        previous = c;
    }
}

/// A string literal, copied exactly with its quotes (a doubled quote stays doubled).
fn copy_literal(chars: &mut Chars<'_>, out: &mut String) {
    out.push('\'');
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '\'' {
            if chars.peek() != Some(&'\'') {
                break;
            }
            out.extend(chars.next());
        }
    }
}

/// A quoted name, lower-cased without its quotes; a doubled closing quote (not in `[...]`) is one literal quote.
fn copy_name(chars: &mut Chars<'_>, close: char, out: &mut String) {
    while let Some(c) = chars.next() {
        if c == close {
            if close == ']' || chars.peek() != Some(&close) {
                break;
            }
            chars.next();
        }
        out.extend(c.to_lowercase());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn normalising_keeps_literals_exact_and_unquotes_names() {
        assert_eq!(
            normalise_outside_literals(
                "CREATE INDEX i ON t (\"A\", [b]) /* c */ WHERE Status = 'Vio  Lation' -- x\n"
            ),
            "createindexiont(a,b)wherestatus='Vio  Lation'"
        );
        assert_eq!(normalise_outside_literals("x = 'it''s'"), "x='it''s'");
    }

    #[test]
    fn a_busy_database_is_an_error_not_a_different_definition() {
        let dir = std::env::temp_dir().join(format!("hairspring-idx-busy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("db.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE t(a); CREATE INDEX i ON t(a);")
            .unwrap();
        let holder = Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(Duration::ZERO).unwrap();
        let result = sqlite_index_fits(&conn, "i", "t", &["a"], None);
        holder.execute_batch("ROLLBACK").unwrap();
        assert!(result.is_err(), "{result:?}");
        assert!(sqlite_index_fits(&conn, "i", "t", &["a"], None).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }
}

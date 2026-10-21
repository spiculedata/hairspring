//! SQLite side of the explorer's storage needs (schema version 4): the digest allowance counters, the stored coverage
//! series and the reporter lookup. Every statement binds its inputs as parameters.
use crate::digest_allowance::{DigestUse, MAX_COVERAGE_JSON_BYTES, MAX_REPORTER_SEEDS};
use crate::reporters::{FirstReporters, Reported};
use crate::trials::to_db;
use crate::types::{Result, StoreError};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::BTreeMap;

/// Seeds named in one statement. Far below SQLite's limit on bound parameters.
const SEEDS_PER_STATEMENT: usize = 500;

pub(crate) fn load_digest_use(conn: &Connection, run_id: &str) -> Result<DigestUse> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT bytes, capped_trials FROM run_digest_use WHERE run_id=?",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(
        row.map_or_else(DigestUse::default, |(bytes, capped_trials)| DigestUse {
            bytes: bytes as u64,
            capped_trials: capped_trials as u64,
        }),
    )
}

pub(crate) fn save_digest_use(conn: &Connection, run_id: &str, used: DigestUse) -> Result<()> {
    conn.execute(
        "INSERT INTO run_digest_use(run_id,bytes,capped_trials) VALUES (?,?,?) \
         ON CONFLICT(run_id) DO UPDATE SET bytes=excluded.bytes, capped_trials=excluded.capped_trials",
        params![run_id, to_db(used.bytes), to_db(used.capped_trials)],
    )?;
    Ok(())
}

/// Keeps `json` as the run's coverage series, or removes the run's series when there is none.
pub(crate) fn write_coverage(conn: &Connection, run_id: &str, json: Option<&str>) -> Result<()> {
    let Some(json) = json else {
        conn.execute("DELETE FROM run_coverage WHERE run_id=?", [run_id])?;
        return Ok(());
    };
    if json.len() > MAX_COVERAGE_JSON_BYTES {
        return Err(StoreError::LimitExceeded {
            what: "coverage series bytes",
            limit: MAX_COVERAGE_JSON_BYTES as u64,
        });
    }
    conn.execute(
        "INSERT INTO run_coverage(run_id,series_json) VALUES (?,?) \
         ON CONFLICT(run_id) DO UPDATE SET series_json=excluded.series_json",
        params![run_id, json],
    )?;
    Ok(())
}

pub(crate) fn run_coverage_json(conn: &Connection, run_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT series_json FROM run_coverage WHERE run_id=?",
            [run_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// The first reporter (lowest sequence number) of each seed. Reads two columns of the rows `trials_seed` finds.
pub(crate) fn reporters_of(
    conn: &Connection,
    run_id: &str,
    seeds: &[u64],
) -> Result<BTreeMap<u64, String>> {
    if seeds.len() > MAX_REPORTER_SEEDS {
        return Err(StoreError::LimitExceeded {
            what: "seeds in one reporter lookup",
            limit: MAX_REPORTER_SEEDS as u64,
        });
    }
    let mut first_reporters = FirstReporters::default();
    for chunk in seeds.chunks(SEEDS_PER_STATEMENT) {
        let mut stmt = conn.prepare(&format!(
            "SELECT seed, seq, worker_id FROM trials WHERE run_id=?1 AND seed IN ({})",
            seed_marks(chunk.len())
        ))?;
        let args = std::iter::once(rusqlite::types::Value::Text(run_id.to_owned())).chain(
            chunk
                .iter()
                .map(|s| rusqlite::types::Value::Integer(*s as i64)),
        );
        let rows = stmt.query_map(params_from_iter(args), |r| {
            Ok(Reported {
                seed: r.get::<_, i64>(0)? as u64,
                seq: r.get::<_, i64>(1)? as u64,
                worker: r.get::<_, String>(2)?,
            })
        })?;
        for row in rows {
            first_reporters.offer(row?);
        }
    }
    Ok(first_reporters.into_workers())
}

/// `?2,?3,...` for `n` seeds bound after the run id. Only placeholders are spliced into the SQL, never a value.
fn seed_marks(n: usize) -> String {
    (0..n)
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reporter_lookup_walks_the_seed_index_and_never_scans_the_trials_table() {
        let conn = Connection::open_in_memory().unwrap();
        crate::sqlite_schema::migrate(&conn).unwrap();
        let sql = format!(
            "EXPLAIN QUERY PLAN SELECT seed, seq, worker_id FROM trials WHERE run_id=?1 AND seed IN ({})",
            seed_marks(3)
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let plan: Vec<String> = stmt
            .query_map(
                rusqlite::params_from_iter([
                    rusqlite::types::Value::Text("r".into()),
                    rusqlite::types::Value::Integer(1),
                    rusqlite::types::Value::Integer(2),
                    rusqlite::types::Value::Integer(3),
                ]),
                |r| r.get::<_, String>(3),
            )
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            plan.iter().any(|p| p.contains("USING INDEX trials_seed")),
            "{plan:?}"
        );
        assert!(
            !plan.iter().any(|p| p.starts_with("SCAN trials")),
            "{plan:?}"
        );
    }

    #[test]
    fn seed_placeholders_are_numbered_after_the_run_id() {
        assert_eq!(seed_marks(0), "");
        assert_eq!(seed_marks(3), "?2,?3,?4");
    }
}

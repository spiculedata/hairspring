//! Run inputs on SQLite (schema version 8). The rules are in [`crate::run_inputs`]; this only fetches and writes rows.
use crate::run_inputs::{
    decide, parse_columns, prepare, Decision, NewRunInputs, RunInputsOutcome, RunInputsRecord,
    RunInputsRejection,
};
use crate::sqlite_tx::write_tx;
use crate::types::{Result, StoreError};
use rusqlite::{params, Connection, OptionalExtension};

/// scheme, fields, missing, reporter, trust, attempt, created_at
type Row = (String, String, String, String, bool, u32, i64);

fn stored_row(conn: &Connection, run_id: &str) -> Result<Option<Row>> {
    Ok(conn
        .query_row(
            "SELECT scheme, fields_json, missing_json, reporter_id, reporter_trusted, attempt, created_at \
             FROM run_inputs WHERE run_id=?",
            [run_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()?)
}

pub(crate) fn get(conn: &Connection, run_id: &str) -> Result<Option<RunInputsRecord>> {
    let Some((
        scheme,
        fields_json,
        missing_json,
        reporter_id,
        reporter_trusted,
        attempt,
        created_at,
    )) = stored_row(conn, run_id)?
    else {
        return Ok(None);
    };
    let (fields, missing) = parse_columns(&fields_json, &missing_json)
        .map_err(|why| StoreError::Backend(why.to_string()))?;
    Ok(Some(RunInputsRecord {
        run_id: run_id.to_string(),
        scheme,
        fields,
        missing,
        reporter_id,
        reporter_trusted,
        attempt,
        created_at,
    }))
}

pub(crate) fn put(conn: &mut Connection, new: &NewRunInputs, now: i64) -> Result<RunInputsOutcome> {
    let prepared = match prepare(new) {
        Ok(p) => p,
        Err(why) => return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Invalid(why))),
    };
    let tx = write_tx(conn)?;
    // `BEGIN IMMEDIATE` holds the write lock, so no trial batch starts a newer attempt between this read and the write.
    let ingest_attempt: Option<i64> = tx
        .query_row(
            "SELECT ingest_attempt FROM runs WHERE id=?",
            [&new.run_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(ingest_attempt) = ingest_attempt else {
        return Ok(RunInputsOutcome::Rejected(RunInputsRejection::RunNotFound));
    };
    // The run's trials are already from a later attempt: these inputs describe trials that are gone.
    if i64::from(new.attempt) < ingest_attempt {
        return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Stale));
    }
    let stored = stored_row(&tx, &new.run_id)?;
    let view = stored
        .as_ref()
        .map(|(scheme, fields, missing, _, _, attempt, _)| {
            (scheme.as_str(), fields.as_str(), missing.as_str(), *attempt)
        });
    let outcome = match decide(view, new, &prepared) {
        Decision::Duplicate => return Ok(RunInputsOutcome::Duplicate),
        Decision::Conflict => return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Conflict)),
        Decision::Stale => return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Stale)),
        Decision::Replace => {
            tx.execute("DELETE FROM run_inputs WHERE run_id=?", [&new.run_id])?;
            RunInputsOutcome::Replaced
        }
        Decision::Insert => RunInputsOutcome::Stored,
    };
    tx.execute(
        "INSERT INTO run_inputs(run_id,scheme,fields_json,missing_json,reporter_id,reporter_trusted,attempt,created_at) \
         VALUES (?,?,?,?,?,?,?,?)",
        params![
            new.run_id,
            new.scheme,
            prepared.fields_json,
            prepared.missing_json,
            new.reporter_id,
            new.reporter_trusted,
            new.attempt,
            now
        ],
    )?;
    tx.commit()?;
    Ok(outcome)
}

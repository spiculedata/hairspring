//! Run inputs on SQLite (schema version 8). The rules are in [`crate::run_inputs`]; this only fetches and writes rows.
use crate::run_inputs::{
    parse_columns, prepare, NewRunInputs, RunInputsOutcome, RunInputsRecord, RunInputsRejection,
};
use crate::sqlite_tx::write_tx;
use crate::types::{Result, StoreError};
use rusqlite::{params, Connection, OptionalExtension};

type Row = (String, String, String, String, bool, i64);

fn stored_row(conn: &Connection, run_id: &str) -> Result<Option<Row>> {
    Ok(conn
        .query_row(
            "SELECT scheme, fields_json, missing_json, reporter_id, reporter_trusted, created_at \
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
                ))
            },
        )
        .optional()?)
}

pub(crate) fn get(conn: &Connection, run_id: &str) -> Result<Option<RunInputsRecord>> {
    let Some((scheme, fields_json, missing_json, reporter_id, reporter_trusted, created_at)) =
        stored_row(conn, run_id)?
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
        created_at,
    }))
}

pub(crate) fn put(conn: &mut Connection, new: &NewRunInputs, now: i64) -> Result<RunInputsOutcome> {
    let prepared = match prepare(new) {
        Ok(p) => p,
        Err(why) => return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Invalid(why))),
    };
    let tx = write_tx(conn)?;
    let run_exists = tx
        .query_row("SELECT 1 FROM runs WHERE id=?", [&new.run_id], |_| Ok(()))
        .optional()?
        .is_some();
    if !run_exists {
        return Ok(RunInputsOutcome::Rejected(RunInputsRejection::RunNotFound));
    }
    if let Some((scheme, fields_json, missing_json, ..)) = stored_row(&tx, &new.run_id)? {
        let same = scheme == new.scheme
            && fields_json == prepared.fields_json
            && missing_json == prepared.missing_json;
        return Ok(if same {
            RunInputsOutcome::Duplicate
        } else {
            RunInputsOutcome::Rejected(RunInputsRejection::Conflict)
        });
    }
    tx.execute(
        "INSERT INTO run_inputs(run_id,scheme,fields_json,missing_json,reporter_id,reporter_trusted,created_at) \
         VALUES (?,?,?,?,?,?,?)",
        params![
            new.run_id,
            new.scheme,
            prepared.fields_json,
            prepared.missing_json,
            new.reporter_id,
            new.reporter_trusted,
            now
        ],
    )?;
    tx.commit()?;
    Ok(RunInputsOutcome::Stored)
}

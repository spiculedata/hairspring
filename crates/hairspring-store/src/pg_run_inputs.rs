//! Run inputs on PostgreSQL (schema version 8). The rules are in [`crate::run_inputs`]; this only fetches and writes
//! rows.
use crate::run_inputs::{
    parse_columns, prepare, NewRunInputs, RunInputsOutcome, RunInputsRecord, RunInputsRejection,
};
use crate::types::{Result, StoreError};
use postgres::GenericClient;

type Row = (String, String, String, String, bool, i64);

fn stored_row<C: GenericClient>(c: &mut C, run_id: &str) -> Result<Option<Row>> {
    Ok(c.query_opt(
        "SELECT scheme, fields_json, missing_json, reporter_id, reporter_trusted, created_at \
         FROM run_inputs WHERE run_id=$1",
        &[&run_id],
    )?
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5))))
}

pub(crate) fn get<C: GenericClient>(c: &mut C, run_id: &str) -> Result<Option<RunInputsRecord>> {
    let Some((scheme, fields_json, missing_json, reporter_id, reporter_trusted, created_at)) =
        stored_row(c, run_id)?
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

pub(crate) fn put(
    c: &mut postgres::Client,
    new: &NewRunInputs,
    now: i64,
) -> Result<RunInputsOutcome> {
    let prepared = match prepare(new) {
        Ok(p) => p,
        Err(why) => return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Invalid(why))),
    };
    let mut tx = c.transaction()?;
    let run_exists = tx
        .query_opt("SELECT 1 FROM runs WHERE id=$1", &[&new.run_id])?
        .is_some();
    if !run_exists {
        return Ok(RunInputsOutcome::Rejected(RunInputsRejection::RunNotFound));
    }
    let inserted = tx.execute(
        "INSERT INTO run_inputs(run_id,scheme,fields_json,missing_json,reporter_id,reporter_trusted,created_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (run_id) DO NOTHING",
        &[
            &new.run_id,
            &new.scheme,
            &prepared.fields_json,
            &prepared.missing_json,
            &new.reporter_id,
            &new.reporter_trusted,
            &now,
        ],
    )?;
    if inserted == 1 {
        tx.commit()?;
        return Ok(RunInputsOutcome::Stored);
    }
    let (scheme, fields_json, missing_json, ..) =
        stored_row(&mut tx, &new.run_id)?.ok_or_else(|| {
            StoreError::Backend("run inputs vanished inside their transaction".into())
        })?;
    let same = scheme == new.scheme
        && fields_json == prepared.fields_json
        && missing_json == prepared.missing_json;
    Ok(if same {
        RunInputsOutcome::Duplicate
    } else {
        RunInputsOutcome::Rejected(RunInputsRejection::Conflict)
    })
}

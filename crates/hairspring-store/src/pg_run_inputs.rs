//! Run inputs on PostgreSQL (schema version 8). The rules are in [`crate::run_inputs`]; this only fetches and writes
//! rows.
use crate::run_inputs::{
    decide, parse_columns, prepare, Decision, NewRunInputs, RunInputsOutcome, RunInputsRecord,
    RunInputsRejection,
};
use crate::types::{Result, StoreError};
use postgres::GenericClient;

/// scheme, fields, missing, reporter, trust, attempt, created_at
type Row = (String, String, String, String, bool, i32, i64);

fn stored_row<C: GenericClient>(c: &mut C, run_id: &str) -> Result<Option<Row>> {
    let sql = "SELECT scheme, fields_json, missing_json, reporter_id, reporter_trusted, attempt, created_at \
               FROM run_inputs WHERE run_id=$1";
    Ok(c.query_opt(sql, &[&run_id])?.map(|r| {
        (
            r.get(0),
            r.get(1),
            r.get(2),
            r.get(3),
            r.get(4),
            r.get(5),
            r.get(6),
        )
    }))
}

pub(crate) fn get<C: GenericClient>(c: &mut C, run_id: &str) -> Result<Option<RunInputsRecord>> {
    let Some((
        scheme,
        fields_json,
        missing_json,
        reporter_id,
        reporter_trusted,
        attempt,
        created_at,
    )) = stored_row(c, run_id)?
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
        attempt: attempt as u32,
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
    let attempt = new.attempt as i32;
    let mut tx = c.transaction()?;
    let run_exists = tx
        .query_opt("SELECT 1 FROM runs WHERE id=$1", &[&new.run_id])?
        .is_some();
    if !run_exists {
        return Ok(RunInputsOutcome::Rejected(RunInputsRejection::RunNotFound));
    }
    // Replacing is a delete and an insert, as on SQLite, so recording needs `SELECT`, `INSERT` and `DELETE` and no
    // `UPDATE`, and no row lock (which would need `UPDATE` too). Two writers can both see the same row: `ON CONFLICT DO
    // NOTHING` lets one insert, and the other reads what it lost to and decides again.
    let mut replaced = false;
    for _ in 0..3 {
        let inserted = tx.execute(
            "INSERT INTO run_inputs(run_id,scheme,fields_json,missing_json,reporter_id,reporter_trusted,attempt,created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (run_id) DO NOTHING",
            &[
                &new.run_id,
                &new.scheme,
                &prepared.fields_json,
                &prepared.missing_json,
                &new.reporter_id,
                &new.reporter_trusted,
                &attempt,
                &now,
            ],
        )?;
        if inserted == 1 {
            tx.commit()?;
            return Ok(if replaced {
                RunInputsOutcome::Replaced
            } else {
                RunInputsOutcome::Stored
            });
        }
        let stored = stored_row(&mut tx, &new.run_id)?.ok_or_else(|| {
            StoreError::Backend("run inputs vanished inside their transaction".into())
        })?;
        let view = (
            stored.0.as_str(),
            stored.1.as_str(),
            stored.2.as_str(),
            stored.5 as u32,
        );
        match decide(Some(view), new, &prepared) {
            Decision::Duplicate => return Ok(RunInputsOutcome::Duplicate),
            Decision::Conflict => {
                return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Conflict))
            }
            Decision::Stale => return Ok(RunInputsOutcome::Rejected(RunInputsRejection::Stale)),
            Decision::Insert | Decision::Replace => {
                tx.execute(
                    "DELETE FROM run_inputs WHERE run_id=$1 AND attempt<$2",
                    &[&new.run_id, &attempt],
                )?;
                replaced = true;
            }
        }
    }
    Err(StoreError::Backend(
        "run inputs kept changing while they were being recorded".into(),
    ))
}

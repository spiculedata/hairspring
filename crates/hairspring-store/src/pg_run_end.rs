//! Ending a run together with its terminal event, on PostgreSQL. The state and the event are written in one
//! transaction, so a reader that sees the run ended also sees the event.
use crate::types::{NewEvent, Result, StoreError};
use postgres::{Client, GenericClient};

/// Cancels a queued or running run, fails its queued jobs and appends `event`, when given, in one transaction.
pub(crate) fn cancel(
    c: &mut Client,
    id: &str,
    event: Option<&NewEvent<'_>>,
    now: i64,
) -> Result<bool> {
    let mut tx = c.transaction()?;
    // Same order as finish_job (jobs, then runs), so the two can never deadlock.
    tx.execute("SELECT id FROM jobs WHERE run_id=$1 FOR UPDATE", &[&id])?;
    let n = tx.execute(
        "UPDATE runs SET state='cancelled', finished_at=$1 WHERE id=$2 AND state IN ('queued','running')",
        &[&now, &id],
    )?;
    if n > 0 {
        tx.execute(
            "UPDATE jobs SET status='failed', last_error='cancelled' WHERE run_id=$1 AND status='queued'",
            &[&id],
        )?;
        if let Some(event) = event {
            insert_event(&mut tx, id, event, now)?;
        }
    }
    tx.commit()?;
    Ok(n > 0)
}

/// Moves a queued or running run to the error state and appends `event`, in one transaction.
pub(crate) fn finish_as_error(
    c: &mut Client,
    id: &str,
    reason: &str,
    event: &NewEvent<'_>,
    now: i64,
) -> Result<bool> {
    let mut tx = c.transaction()?;
    let n = tx.execute(
        "UPDATE runs SET state='error', steps=0, trials=0, error=$1, finished_at=$2 WHERE id=$3 AND state IN ('queued','running')",
        &[&reason, &now, &id],
    )?;
    if n > 0 {
        insert_event(&mut tx, id, event, now)?;
    } else if tx
        .query_opt("SELECT 1 FROM runs WHERE id=$1", &[&id])?
        .is_none()
    {
        return Err(StoreError::NotFound(format!("run {id}")));
    }
    tx.commit()?;
    Ok(n > 0)
}

fn insert_event<C: GenericClient>(
    c: &mut C,
    run_id: &str,
    event: &NewEvent<'_>,
    now: i64,
) -> Result<()> {
    c.execute(
        "INSERT INTO events(run_id,ts,kind,payload_json) VALUES ($1,$2,$3,$4)",
        &[&run_id, &now, &event.kind, &event.payload_json],
    )?;
    Ok(())
}

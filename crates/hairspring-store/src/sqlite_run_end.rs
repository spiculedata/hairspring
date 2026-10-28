//! Ending a run together with its terminal event, on SQLite. The state and the event are written in one transaction,
//! so a reader that sees the run ended also sees the event.
use crate::types::{NewEvent, Result, StoreError};
use rusqlite::{params, Connection, OptionalExtension};

/// Cancels a queued or running run, fails its queued jobs and appends `event`, when given, in one transaction.
pub(crate) fn cancel(
    conn: &mut Connection,
    id: &str,
    event: Option<&NewEvent<'_>>,
    now: i64,
) -> Result<bool> {
    let tx = crate::sqlite_tx::write_tx(conn)?;
    let n = tx.execute(
        "UPDATE runs SET state='cancelled', finished_at=? WHERE id=? AND state IN ('queued','running')",
        params![now, id],
    )?;
    if n > 0 {
        tx.execute(
            "UPDATE jobs SET status='failed', last_error='cancelled' WHERE run_id=? AND status='queued'",
            [id],
        )?;
        if let Some(event) = event {
            insert_event(&tx, id, event, now)?;
        }
    }
    tx.commit()?;
    Ok(n > 0)
}

/// Moves a queued or running run to the error state and appends `event`, in one transaction.
pub(crate) fn finish_as_error(
    conn: &mut Connection,
    id: &str,
    reason: &str,
    event: &NewEvent<'_>,
    now: i64,
) -> Result<bool> {
    let tx = crate::sqlite_tx::write_tx(conn)?;
    let n = tx.execute(
        "UPDATE runs SET state='error', steps=0, trials=0, error=?, finished_at=? WHERE id=? AND state IN ('queued','running')",
        params![reason, now, id],
    )?;
    if n > 0 {
        insert_event(&tx, id, event, now)?;
    } else if !run_exists(&tx, id)? {
        return Err(StoreError::NotFound(format!("run {id}")));
    }
    tx.commit()?;
    Ok(n > 0)
}

fn insert_event(conn: &Connection, run_id: &str, event: &NewEvent<'_>, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO events(run_id,ts,kind,payload_json) VALUES (?,?,?,?)",
        params![run_id, now, event.kind, event.payload_json],
    )?;
    Ok(())
}

fn run_exists(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM runs WHERE id=?", [id], |_| Ok(()))
        .optional()?
        .is_some())
}

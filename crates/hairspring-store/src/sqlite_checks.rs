//! Stored confirmation checks on SQLite. A confirmation is written in one transaction, and the connection's mutex plus
//! the write transaction serialise writers, so two identical confirmations store once. Rows list by creation time and
//! then by the order they were written (`rowid`), never by id.
use crate::checks::*;
use crate::new_id;
use crate::types::{Result, StoreError};
use rusqlite::{params, Connection, OptionalExtension, Row};

const COLUMNS: &str = "id, failure_id, confirmation_id, ordinal, kind, status, passed, failed, inconclusive, \
    command, detail_json, artefacts_json, environment_json, started_at, finished_at, runner, runner_trusted, created_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<CheckRecord> {
    Ok(CheckRecord {
        id: r.get(0)?,
        failure_id: r.get(1)?,
        confirmation_id: r.get(2)?,
        ordinal: r.get(3)?,
        kind: r.get(4)?,
        status: r.get(5)?,
        passed: r.get(6)?,
        failed: r.get(7)?,
        inconclusive: r.get(8)?,
        command: r.get(9)?,
        detail_json: r.get(10)?,
        artefacts_json: r.get(11)?,
        environment_json: r.get(12)?,
        started_at: r.get(13)?,
        finished_at: r.get(14)?,
        runner: r.get(15)?,
        runner_trusted: r.get::<_, i64>(16)? != 0,
        created_at: r.get(17)?,
    })
}

pub(crate) fn list(conn: &Connection, failure_id: &str) -> Result<Vec<CheckRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM failure_checks c WHERE failure_id=? ORDER BY created_at, \
         (SELECT MIN(rowid) FROM failure_checks m WHERE m.confirmation_id=c.confirmation_id), ordinal"
    ))?;
    let rows = stmt
        .query_map([failure_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn of_confirmation(conn: &Connection, id: &str) -> Result<Vec<CheckRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM failure_checks WHERE confirmation_id=? ORDER BY ordinal"
    ))?;
    let rows = stmt
        .query_map([id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether the rows already stored under the confirmation's id are exactly this confirmation.
fn same_confirmation(stored: &[CheckRecord], new: &NewConfirmation) -> bool {
    stored.len() == new.checks.len()
        && stored[0].failure_id == new.failure_id
        && stored
            .iter()
            .zip(&new.checks)
            .enumerate()
            .all(|(i, (row, check))| row.same_as(check, i as u32))
}

pub(crate) fn add(
    conn: &mut Connection,
    new: &NewConfirmation,
    now: i64,
) -> Result<ConfirmationOutcome> {
    validate(new)?;
    let tx = conn.transaction()?;
    let stored = of_confirmation(&tx, &new.id)?;
    if !stored.is_empty() {
        return if same_confirmation(&stored, new) {
            Ok(ConfirmationOutcome::Duplicate)
        } else {
            Err(StoreError::Backend(format!(
                "confirmation {} is already stored with other content",
                new.id
            )))
        };
    }
    let exists = tx
        .query_row(
            "SELECT 1 FROM failures WHERE id=?",
            [&new.failure_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(StoreError::NotFound(format!("failure {}", new.failure_id)));
    }
    let held = {
        let mut stmt = tx.prepare(
            "SELECT confirmation_id, COUNT(*) FROM failure_checks WHERE failure_id=? \
             GROUP BY confirmation_id ORDER BY MIN(created_at), MIN(rowid)",
        )?;
        let rows = stmt
            .query_map([&new.failure_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for id in to_drop(&held, new.checks.len()) {
        tx.execute("DELETE FROM failure_checks WHERE confirmation_id=?", [id])?;
    }
    for (ordinal, c) in new.checks.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO failure_checks({COLUMNS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
            ),
            params![
                new_id("chk"),
                new.failure_id,
                new.id,
                ordinal as i64,
                c.kind,
                c.status,
                c.passed,
                c.failed,
                c.inconclusive,
                c.command,
                c.detail_json,
                c.artefacts_json,
                c.environment_json,
                c.started_at,
                c.finished_at,
                new.runner,
                i64::from(new.runner_trusted),
                now
            ],
        )?;
    }
    tx.commit()?;
    Ok(ConfirmationOutcome::Stored)
}

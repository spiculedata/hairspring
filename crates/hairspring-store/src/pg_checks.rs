//! Stored confirmation checks on PostgreSQL. A confirmation is written in one transaction that first locks the failure's
//! row, so confirmations of one failure are written one at a time and two identical ones store once.
use crate::checks::*;
use crate::new_id;
use crate::types::{Result, StoreError};
use postgres::{Client, GenericClient, Row};

const COLUMNS: &str = "id, failure_id, confirmation_id, ordinal, kind, status, passed, failed, inconclusive, \
    command, detail_json, artefacts_json, environment_json, started_at, finished_at, runner, runner_trusted, created_at";

fn from_row(r: &Row) -> CheckRecord {
    CheckRecord {
        id: r.get(0),
        failure_id: r.get(1),
        confirmation_id: r.get(2),
        ordinal: r.get::<_, i32>(3) as u32,
        kind: r.get(4),
        status: r.get(5),
        passed: r.get::<_, i64>(6) as u32,
        failed: r.get::<_, i64>(7) as u32,
        inconclusive: r.get::<_, i64>(8) as u32,
        command: r.get(9),
        detail_json: r.get(10),
        artefacts_json: r.get(11),
        environment_json: r.get(12),
        started_at: r.get(13),
        finished_at: r.get(14),
        runner: r.get(15),
        runner_trusted: r.get(16),
        created_at: r.get(17),
    }
}

pub(crate) fn list(c: &mut Client, failure_id: &str) -> Result<Vec<CheckRecord>> {
    let rows = c.query(
        &format!(
            "SELECT {COLUMNS} FROM failure_checks WHERE failure_id=$1 ORDER BY created_at, confirmation_id, ordinal"
        ),
        &[&failure_id],
    )?;
    Ok(rows.iter().map(from_row).collect())
}

fn of_confirmation<C: GenericClient>(c: &mut C, id: &str) -> Result<Vec<CheckRecord>> {
    let rows = c.query(
        &format!("SELECT {COLUMNS} FROM failure_checks WHERE confirmation_id=$1 ORDER BY ordinal"),
        &[&id],
    )?;
    Ok(rows.iter().map(from_row).collect())
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

pub(crate) fn add(c: &mut Client, new: &NewConfirmation, now: i64) -> Result<ConfirmationOutcome> {
    validate(new)?;
    let mut tx = c.transaction()?;
    // Locks the failure, so a concurrent confirmation of it waits here and then sees this one.
    let locked = tx.query_opt(
        "SELECT 1 FROM failures WHERE id=$1 FOR UPDATE",
        &[&new.failure_id],
    )?;
    if locked.is_none() {
        return Err(StoreError::NotFound(format!("failure {}", new.failure_id)));
    }
    let stored = of_confirmation(&mut tx, &new.id)?;
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
    let held: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM failure_checks WHERE failure_id=$1",
            &[&new.failure_id],
        )?
        .get(0);
    if over_cap(held as u64, new.checks.len()) {
        return Err(StoreError::LimitExceeded {
            what: "checks held for one failure",
            limit: MAX_CHECKS_PER_FAILURE,
        });
    }
    for (ordinal, ch) in new.checks.iter().enumerate() {
        tx.execute(
            &format!(
                "INSERT INTO failure_checks({COLUMNS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"
            ),
            &[
                &new_id("chk"),
                &new.failure_id,
                &new.id,
                &(ordinal as i32),
                &ch.kind,
                &ch.status,
                &i64::from(ch.passed),
                &i64::from(ch.failed),
                &i64::from(ch.inconclusive),
                &ch.command,
                &ch.detail_json,
                &ch.artefacts_json,
                &ch.environment_json,
                &ch.started_at,
                &ch.finished_at,
                &new.runner,
                &new.runner_trusted,
                &now,
            ],
        )?;
    }
    tx.commit()?;
    Ok(ConfirmationOutcome::Stored)
}

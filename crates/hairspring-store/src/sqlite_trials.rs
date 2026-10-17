//! SQLite side of per-trial storage (schema version 2). Every statement binds its inputs as parameters; nothing
//! guest-influenced is ever spliced into SQL text.
use crate::sha256_hex;
use crate::sqlite_lineage as lineage;
use crate::trials::{to_db, *};
use crate::types::{Result, RunState, StoreError};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

const TRIAL_COLUMNS: &str = "run_id, seq, seed, verdict, reason, exit_code, signal, timed_out, degraded, perturbing, \
     report_missing, rated, claimable, behaviour_hash, schedule_fp, syscalls, work_ops, elapsed_ms, worker_id, \
     worker_trusted, attempt, kept, detail_ref, output_ref, digest";

fn trial_from_row(r: &Row<'_>) -> rusqlite::Result<TrialRecord> {
    let verdict: String = r.get("verdict")?;
    let kept: String = r.get("kept")?;
    Ok(TrialRecord {
        run_id: r.get("run_id")?,
        seq: r.get::<_, i64>("seq")? as u64,
        seed: r.get::<_, i64>("seed")? as u64,
        verdict: TrialVerdict::parse(&verdict).unwrap_or(TrialVerdict::Infra),
        reason: r.get("reason")?,
        exit_code: r.get("exit_code")?,
        signal: r.get("signal")?,
        timed_out: r.get("timed_out")?,
        degraded: r.get("degraded")?,
        perturbing: r.get("perturbing")?,
        report_missing: r.get("report_missing")?,
        rated: r.get("rated")?,
        claimable: r.get("claimable")?,
        behaviour_hash: r.get("behaviour_hash")?,
        schedule_fp: r.get("schedule_fp")?,
        syscalls: r.get::<_, i64>("syscalls")? as u64,
        work_ops: r.get::<_, i64>("work_ops")? as u64,
        elapsed_ms: r.get::<_, i64>("elapsed_ms")? as u64,
        worker_id: r.get("worker_id")?,
        worker_trusted: r.get("worker_trusted")?,
        attempt: r.get::<_, i64>("attempt")? as u32,
        kept: KeptState::parse(&kept).unwrap_or(KeptState::NotKept),
        detail_ref: r.get("detail_ref")?,
        output_ref: r.get("output_ref")?,
        digest: r.get("digest")?,
    })
}

fn catalog_from_row(r: &Row<'_>) -> rusqlite::Result<CatalogRowRecord> {
    Ok(CatalogRowRecord {
        idx: r.get::<_, i64>("idx")? as u32,
        prop_id: r.get("prop_id")?,
        kind: r.get("kind")?,
        must_hit: r.get("must_hit")?,
        msg: r.get("msg")?,
        loc_file: r.get("loc_file")?,
        loc_line: r.get::<_, Option<i64>>("loc_line")?.map(|l| l as u64),
        loc_func: r.get("loc_func")?,
        decided: r.get("decided")?,
    })
}

fn property_from_row(r: &Row<'_>) -> rusqlite::Result<RunPropertyRow> {
    let n = |name: &str| r.get::<_, i64>(name).map(|v| v as u64);
    let opt = |name: &str| r.get::<_, Option<i64>>(name).map(|v| v.map(|v| v as u64));
    Ok(RunPropertyRow {
        idx: r.get::<_, i64>("idx")? as u32,
        status: r.get("status")?,
        trials: n("trials")?,
        trials_rated: n("trials_rated")?,
        trials_unreported: n("trials_unreported")?,
        trials_degraded: n("trials_degraded")?,
        trials_perturbing: n("trials_perturbing")?,
        trials_conflicted: n("trials_conflicted")?,
        trials_inconclusive_with_failures: n("trials_inconclusive_with_failures")?,
        trials_claimable: n("trials_claimable")?,
        trials_hit: n("trials_hit")?,
        trials_true: n("trials_true")?,
        trials_false: n("trials_false")?,
        evals: n("evals")?,
        evals_true: n("evals_true")?,
        evals_false: n("evals_false")?,
        rate_basis: r.get("rate_basis")?,
        rate_k: opt("rate_k")?,
        rate_n: opt("rate_n")?,
        first_failure_seed: opt("first_failure_seed")?,
    })
}

/// A lease check and the run state, read under the write lock this transaction already holds.
struct Target {
    run_id: String,
    attempt: i64,
}

fn lease_target(tx: &Transaction, job_id: &str, worker_id: &str) -> Result<Option<Target>> {
    let job: Option<(String, String, Option<String>, i64)> = tx
        .query_row(
            "SELECT run_id, status, lease_owner, attempts FROM jobs WHERE id=?",
            [job_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(job.and_then(|(run_id, status, owner, attempt)| {
        (status == "running" && owner.as_deref() == Some(worker_id))
            .then_some(Target { run_id, attempt })
    }))
}

struct IngestRun {
    open: bool,
    ingest_attempt: i64,
    catalog_size: i64,
    kept_bytes: i64,
}

fn run_state(tx: &Transaction, run_id: &str) -> Result<Option<IngestRun>> {
    let row: Option<(String, i64, i64, i64)> = tx
        .query_row(
            "SELECT state, ingest_attempt, catalog_size, kept_bytes FROM runs WHERE id=?",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(row.map(
        |(state, ingest_attempt, catalog_size, kept_bytes)| IngestRun {
            open: RunState::parse(&state).is_some_and(|s| !s.is_terminal()),
            ingest_attempt,
            catalog_size,
            kept_bytes,
        },
    ))
}

/// A new attempt of the job starts the run's trials over; the earlier attempt's rows are removed. The run's tree nodes
/// refer to its trial rows, so they go first. Returns the reason, with nothing changed, when other rows depend on this
/// run's nodes (branch runs of the tree it roots, nodes built on its nodes).
fn start_attempt(
    tx: &Transaction,
    run_id: &str,
    attempt: i64,
) -> Result<std::result::Result<(), String>> {
    if let Some(why) = lineage::attempt_blocked(tx, run_id)? {
        return Ok(Err(why));
    }
    let mut blobs = kept_blobs(tx, run_id)?;
    blobs.extend(lineage::drop_run_lineage(tx, run_id)?);
    for table in ["trials", "run_catalog", "run_properties"] {
        tx.execute(&format!("DELETE FROM {table} WHERE run_id=?"), [run_id])?;
    }
    tx.execute(
        "UPDATE runs SET ingest_attempt=?, catalog_size=0, kept_bytes=0 WHERE id=?",
        params![attempt, run_id],
    )?;
    collect_orphans(tx, &blobs)?;
    Ok(Ok(()))
}

/// The blobs the run's trial rows point at.
fn kept_blobs(tx: &Transaction, run_id: &str) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(
        "SELECT detail_ref FROM trials WHERE run_id=?1 AND detail_ref IS NOT NULL UNION SELECT output_ref FROM trials WHERE run_id=?1 AND output_ref IS NOT NULL",
    )?;
    let hashes = stmt
        .query_map([run_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(hashes)
}

/// Deletes the blobs no remaining trial row or failure refers to.
fn collect_orphans(tx: &Transaction, hashes: &[String]) -> Result<()> {
    for h in hashes {
        tx.execute(
            "DELETE FROM blobs WHERE hash=?1 AND NOT EXISTS (SELECT 1 FROM trials WHERE detail_ref=?1 OR output_ref=?1) AND NOT EXISTS (SELECT 1 FROM failures WHERE bundle_hash=?1) AND NOT EXISTS (SELECT 1 FROM trial_lineage WHERE edge_ref=?1)",
            [h],
        )?;
    }
    Ok(())
}

/// Applies one catalog row: the next index, or the one-time decision of an undecided row.
fn apply_catalog_row(
    tx: &Transaction,
    run_id: &str,
    size: i64,
    row: &CatalogRowRecord,
) -> Result<std::result::Result<i64, String>> {
    let idx = i64::from(row.idx);
    if idx == size {
        if size as usize >= MAX_CATALOG_ROWS {
            return Ok(Err(format!(
                "a run may hold at most {MAX_CATALOG_ROWS} catalog rows"
            )));
        }
        let taken: Option<i64> = tx
            .query_row(
                "SELECT idx FROM run_catalog WHERE run_id=? AND prop_id=?",
                params![run_id, row.prop_id],
                |r| r.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Ok(Err("a catalog id was added twice".to_string()));
        }
        tx.execute(
            "INSERT INTO run_catalog(run_id,idx,prop_id,kind,must_hit,msg,loc_file,loc_line,loc_func,decided) VALUES (?,?,?,?,?,?,?,?,?,?)",
            params![run_id, idx, row.prop_id, row.kind, row.must_hit, row.msg, row.loc_file, row.loc_line.map(to_db), row.loc_func, row.decided],
        )?;
        return Ok(Ok(size + 1));
    }
    if idx > size {
        return Ok(Err("a catalog row skips an index".to_string()));
    }
    let existing: Option<CatalogRowRecord> = tx
        .query_row(
            "SELECT * FROM run_catalog WHERE run_id=? AND idx=?",
            params![run_id, idx],
            catalog_from_row,
        )
        .optional()?;
    let Some(existing) = existing else {
        return Ok(Err("a catalog row is missing".to_string()));
    };
    if existing == *row {
        // A batch sent again after a lost reply repeats its catalog rows.
        return Ok(Ok(size));
    }
    if existing.decided {
        return Ok(Err("a decided catalog row cannot change".to_string()));
    }
    if existing.prop_id != row.prop_id {
        return Ok(Err("a decision cannot rename a catalog id".to_string()));
    }
    if !row.decided {
        return Ok(Err(
            "an update to an existing catalog row must decide it".to_string()
        ));
    }
    tx.execute(
        "UPDATE run_catalog SET kind=?, must_hit=?, msg=?, loc_file=?, loc_line=?, loc_func=?, decided=? WHERE run_id=? AND idx=?",
        params![row.kind, row.must_hit, row.msg, row.loc_file, row.loc_line.map(to_db), row.loc_func, row.decided, run_id, idx],
    )?;
    Ok(Ok(size))
}

fn put_blob(tx: &Transaction, data: &[u8]) -> Result<String> {
    let hash = sha256_hex(data);
    tx.execute(
        "INSERT OR IGNORE INTO blobs(hash,data) VALUES (?,?)",
        params![hash, data],
    )?;
    Ok(hash)
}

struct Kept {
    state: KeptState,
    detail_ref: Option<String>,
    output_ref: Option<String>,
    bytes: i64,
}

/// Stores a trial's kept detail and output as blobs, unless the run's allowance is used up; then the trial says so.
fn keep(tx: &Transaction, t: &NewTrial, used: i64, cap: u64) -> Result<Kept> {
    let size =
        (t.detail.as_ref().map_or(0, Vec::len) + t.output.as_ref().map_or(0, Vec::len)) as i64;
    if t.detail.is_none() && t.output.is_none() {
        return Ok(Kept {
            state: KeptState::NotKept,
            detail_ref: None,
            output_ref: None,
            bytes: 0,
        });
    }
    if (used.saturating_add(size)) as u64 > cap {
        return Ok(Kept {
            state: KeptState::DroppedCap,
            detail_ref: None,
            output_ref: None,
            bytes: 0,
        });
    }
    let detail_ref = t.detail.as_deref().map(|d| put_blob(tx, d)).transpose()?;
    let output_ref = t.output.as_deref().map(|o| put_blob(tx, o)).transpose()?;
    Ok(Kept {
        state: KeptState::Kept,
        detail_ref,
        output_ref,
        bytes: size,
    })
}

fn existing_trial(tx: &Transaction, run_id: &str, seq: u64) -> Result<Option<TrialRecord>> {
    Ok(tx
        .query_row(
            &format!("SELECT {TRIAL_COLUMNS} FROM trials WHERE run_id=? AND seq=?"),
            params![run_id, to_db(seq)],
            trial_from_row,
        )
        .optional()?)
}

struct Who<'a> {
    worker_id: &'a str,
    trusted: bool,
    attempt: i64,
}

fn insert_trial(
    tx: &Transaction,
    run_id: &str,
    who: &Who,
    t: &NewTrial,
    kept: &Kept,
) -> Result<()> {
    tx.execute(
        &format!("INSERT INTO trials({TRIAL_COLUMNS}) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"),
        params![
            run_id, to_db(t.seq), t.seed as i64, t.verdict.as_str(), t.reason, t.exit_code, t.signal,
            t.timed_out, t.degraded, t.perturbing, t.report_missing, t.rated, t.claimable,
            t.behaviour_hash, t.schedule_fp, to_db(t.syscalls), to_db(t.work_ops), to_db(t.elapsed_ms),
            who.worker_id, who.trusted, who.attempt, kept.state.as_str(), kept.detail_ref, kept.output_ref, t.digest,
        ],
    )?;
    Ok(())
}

pub(crate) fn ingest(
    conn: &mut Connection,
    job_id: &str,
    worker_id: &str,
    worker_trusted: bool,
    batch: &TrialBatch,
) -> Result<IngestOutcome> {
    if let Err(why) = batch.check() {
        return Ok(IngestOutcome::Rejected(why));
    }
    // The write lock is taken up front, so concurrent batches for one run are applied one after the other.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(target) = lease_target(&tx, job_id, worker_id)? else {
        return Ok(IngestOutcome::NotLeased);
    };
    let Some(mut run) = run_state(&tx, &target.run_id)? else {
        return Ok(IngestOutcome::NotLeased);
    };
    if !run.open {
        return Ok(IngestOutcome::RunNotOpen);
    }
    if target.attempt < run.ingest_attempt {
        return Ok(IngestOutcome::Rejected(
            "this attempt of the job is no longer current".to_string(),
        ));
    }
    if target.attempt > run.ingest_attempt {
        if let Err(why) = start_attempt(&tx, &target.run_id, target.attempt)? {
            return Ok(IngestOutcome::Rejected(why));
        }
        run.catalog_size = 0;
        run.kept_bytes = 0;
    }
    for row in &batch.catalog {
        match apply_catalog_row(&tx, &target.run_id, run.catalog_size, row)? {
            Ok(size) => run.catalog_size = size,
            Err(why) => return Ok(IngestOutcome::Rejected(why)),
        }
    }
    let who = Who {
        worker_id,
        trusted: worker_trusted,
        attempt: target.attempt,
    };
    let (mut inserted, mut duplicates) = (0u32, 0u32);
    for t in &batch.trials {
        if t.max_idx.is_some_and(|m| i64::from(m) >= run.catalog_size) {
            return Ok(IngestOutcome::Rejected(format!(
                "trial {} refers to an id the catalog does not hold",
                t.seq
            )));
        }
        if let Some(stored) = existing_trial(&tx, &target.run_id, t.seq)? {
            if !stored.same_content(t) {
                return Ok(IngestOutcome::Rejected(format!(
                    "trial {} was already stored with different content",
                    t.seq
                )));
            }
            duplicates += 1;
            continue;
        }
        let kept = keep(&tx, t, run.kept_bytes, batch.kept_bytes_cap)?;
        run.kept_bytes += kept.bytes;
        insert_trial(&tx, &target.run_id, &who, t, &kept)?;
        inserted += 1;
    }
    tx.execute(
        "UPDATE runs SET catalog_size=?, kept_bytes=? WHERE id=?",
        params![run.catalog_size, run.kept_bytes, target.run_id],
    )?;
    tx.commit()?;
    Ok(IngestOutcome::Applied {
        inserted,
        duplicates,
    })
}

/// Replaces a run's property rows. Called inside the transaction that finishes the job.
pub(crate) fn replace_run_properties(
    tx: &Transaction,
    run_id: &str,
    rows: &[RunPropertyRow],
) -> Result<()> {
    tx.execute("DELETE FROM run_properties WHERE run_id=?", [run_id])?;
    for p in rows {
        tx.execute(
            "INSERT INTO run_properties(run_id,idx,status,trials,trials_rated,trials_unreported,trials_degraded,trials_perturbing,trials_conflicted,trials_inconclusive_with_failures,trials_claimable,trials_hit,trials_true,trials_false,evals,evals_true,evals_false,rate_basis,rate_k,rate_n,first_failure_seed) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                run_id, p.idx, p.status, to_db(p.trials), to_db(p.trials_rated), to_db(p.trials_unreported),
                to_db(p.trials_degraded), to_db(p.trials_perturbing), to_db(p.trials_conflicted),
                to_db(p.trials_inconclusive_with_failures), to_db(p.trials_claimable), to_db(p.trials_hit),
                to_db(p.trials_true), to_db(p.trials_false), to_db(p.evals), to_db(p.evals_true), to_db(p.evals_false),
                p.rate_basis, p.rate_k.map(to_db), p.rate_n.map(to_db), p.first_failure_seed.map(|s| s as i64),
            ],
        )?;
    }
    Ok(())
}

/// Writes the run-level figures of a finished job.
pub(crate) fn write_run_stats(tx: &Transaction, run_id: &str, s: &RunStats) -> Result<()> {
    tx.execute(
        "UPDATE runs SET inconclusive=?, distinct_schedules=?, work=?, stopped_at_first_failure=?, ingest_incomplete=?, property_summary_json=? WHERE id=?",
        params![
            to_db(s.inconclusive), to_db(s.distinct_schedules), to_db(s.work), s.stopped_at_first_failure,
            s.ingest_incomplete, s.property_summary_json, run_id
        ],
    )?;
    replace_run_properties(tx, run_id, &s.properties)
}

pub(crate) fn count_trials(conn: &Connection, run_id: &str) -> Result<u64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM trials WHERE run_id=?",
        [run_id],
        |r| r.get::<_, i64>(0),
    )? as u64)
}

pub(crate) fn get_trial(conn: &Connection, run_id: &str, seq: u64) -> Result<Option<TrialRecord>> {
    Ok(conn
        .query_row(
            &format!("SELECT {TRIAL_COLUMNS} FROM trials WHERE run_id=? AND seq=?"),
            params![run_id, to_db(seq)],
            trial_from_row,
        )
        .optional()?)
}

/// `after` is exclusive; `None` starts from the first row.
fn after_bound(after: Option<u64>) -> i64 {
    after.map_or(-1, to_db)
}

pub(crate) fn list_trials(
    conn: &Connection,
    run_id: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<Vec<TrialRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {TRIAL_COLUMNS} FROM trials WHERE run_id=? AND seq>? ORDER BY seq LIMIT ?"
    ))?;
    let rows = stmt
        .query_map(params![run_id, after_bound(after), limit], trial_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn list_trial_digests(
    conn: &Connection,
    run_id: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<Vec<TrialDigestRow>> {
    let mut stmt = conn.prepare(
        "SELECT seq, seed, digest FROM trials WHERE run_id=? AND seq>? ORDER BY seq LIMIT ?",
    )?;
    let rows = stmt
        .query_map(params![run_id, after_bound(after), limit], |r| {
            Ok(TrialDigestRow {
                seq: r.get::<_, i64>(0)? as u64,
                seed: r.get::<_, i64>(1)? as u64,
                digest: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn list_trial_strip_rows(
    conn: &Connection,
    run_id: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<Vec<TrialStripRow>> {
    let mut stmt = conn.prepare(
        "SELECT seq, verdict, rated, degraded, report_missing FROM trials \
         WHERE run_id=? AND seq>? ORDER BY seq LIMIT ?",
    )?;
    let rows = stmt
        .query_map(params![run_id, after_bound(after), limit], |r| {
            let verdict: String = r.get(1)?;
            Ok(TrialStripRow {
                seq: r.get::<_, i64>(0)? as u64,
                verdict: TrialVerdict::parse(&verdict).unwrap_or(TrialVerdict::Infra),
                rated: r.get(2)?,
                degraded: r.get(3)?,
                report_missing: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn get_catalog(conn: &Connection, run_id: &str) -> Result<Vec<CatalogRowRecord>> {
    let mut stmt = conn.prepare("SELECT * FROM run_catalog WHERE run_id=? ORDER BY idx")?;
    let rows = stmt
        .query_map([run_id], catalog_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn list_run_properties(conn: &Connection, run_id: &str) -> Result<Vec<RunPropertyRow>> {
    let mut stmt = conn.prepare("SELECT * FROM run_properties WHERE run_id=? ORDER BY idx")?;
    let rows = stmt
        .query_map([run_id], property_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn run_contributors(conn: &Connection, run_id: &str) -> Result<Vec<Contributor>> {
    let mut stmt = conn.prepare(
        "SELECT worker_id, MIN(worker_trusted), COUNT(*) FROM trials WHERE run_id=? GROUP BY worker_id ORDER BY MIN(seq)",
    )?;
    let rows = stmt
        .query_map([run_id], |r| {
            Ok(Contributor {
                worker_id: r.get(0)?,
                trusted: r.get::<_, i64>(1)? != 0,
                trials: r.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn set_worker_secret_hash(conn: &Connection, id: &str, hash: &str) -> Result<()> {
    let n = conn.execute(
        "UPDATE workers SET secret_hash=? WHERE id=?",
        params![hash, id],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("worker {id}")));
    }
    Ok(())
}

pub(crate) fn worker_secret_hash(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT secret_hash FROM workers WHERE id=?", [id], |r| {
            r.get(0)
        })
        .optional()?
        .flatten())
}

pub(crate) fn set_worker_trusted(conn: &Connection, id: &str, trusted: bool) -> Result<()> {
    let n = conn.execute(
        "UPDATE workers SET trusted=? WHERE id=?",
        params![trusted, id],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("worker {id}")));
    }
    Ok(())
}

pub(crate) fn worker_trusted(conn: &Connection, id: &str) -> Result<Option<bool>> {
    Ok(conn
        .query_row("SELECT trusted FROM workers WHERE id=?", [id], |r| r.get(0))
        .optional()?)
}

/// Finishing a job makes the finishing attempt's rows the run's rows: anything an earlier attempt left is removed.
pub(crate) fn discard_earlier_attempts(tx: &Transaction, run_id: &str, attempt: i64) -> Result<()> {
    let held: i64 = tx.query_row(
        "SELECT ingest_attempt FROM runs WHERE id=?",
        [run_id],
        |r| r.get(0),
    )?;
    if held != 0 && held < attempt {
        // Finishing cannot be refused halfway, so the caller's transaction ends in an error and nothing is applied.
        start_attempt(tx, run_id, attempt)?.map_err(StoreError::Backend)?;
    }
    Ok(())
}

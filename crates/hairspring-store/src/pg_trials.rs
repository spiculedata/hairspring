//! PostgreSQL side of per-trial storage (schema version 2). Every statement binds its inputs as parameters; nothing
//! guest-influenced is ever spliced into SQL text. Concurrent batches for one run are serialised by a row lock.
use crate::digest_allowance::{digest_form, DigestForm, DigestUse};
use crate::pg_explorer as explorer;
use crate::pg_lineage as lineage;
use crate::sha256_hex;
use crate::trials::{to_db, *};
use crate::types::{Result, RunState, StoreError};
use postgres::{Client, Row, Transaction};

const TRIAL_COLUMNS: &str = "run_id, seq, seed, verdict, reason, exit_code, signal, timed_out, degraded, perturbing, \
     report_missing, rated, claimable, behaviour_hash, schedule_fp, syscalls, work_ops, elapsed_ms, worker_id, \
     worker_trusted, attempt, kept, detail_ref, output_ref, digest";

fn trial_from_row(r: &Row) -> TrialRecord {
    let verdict: String = r.get("verdict");
    let kept: String = r.get("kept");
    TrialRecord {
        run_id: r.get("run_id"),
        seq: r.get::<_, i64>("seq") as u64,
        seed: r.get::<_, i64>("seed") as u64,
        verdict: TrialVerdict::parse(&verdict).unwrap_or(TrialVerdict::Infra),
        reason: r.get("reason"),
        exit_code: r.get("exit_code"),
        signal: r.get("signal"),
        timed_out: r.get("timed_out"),
        degraded: r.get("degraded"),
        perturbing: r.get("perturbing"),
        report_missing: r.get("report_missing"),
        rated: r.get("rated"),
        claimable: r.get("claimable"),
        behaviour_hash: r.get("behaviour_hash"),
        schedule_fp: r.get("schedule_fp"),
        syscalls: r.get::<_, i64>("syscalls") as u64,
        work_ops: r.get::<_, i64>("work_ops") as u64,
        elapsed_ms: r.get::<_, i64>("elapsed_ms") as u64,
        worker_id: r.get("worker_id"),
        worker_trusted: r.get("worker_trusted"),
        attempt: r.get::<_, i64>("attempt") as u32,
        kept: KeptState::parse(&kept).unwrap_or(KeptState::NotKept),
        detail_ref: r.get("detail_ref"),
        output_ref: r.get("output_ref"),
        digest: r.get("digest"),
    }
}

fn catalog_from_row(r: &Row) -> CatalogRowRecord {
    CatalogRowRecord {
        idx: r.get::<_, i32>("idx") as u32,
        prop_id: r.get("prop_id"),
        kind: r.get("kind"),
        must_hit: r.get("must_hit"),
        msg: r.get("msg"),
        loc_file: r.get("loc_file"),
        loc_line: r.get::<_, Option<i64>>("loc_line").map(|l| l as u64),
        loc_func: r.get("loc_func"),
        decided: r.get("decided"),
    }
}

fn property_from_row(r: &Row) -> RunPropertyRow {
    let n = |name: &str| r.get::<_, i64>(name) as u64;
    let opt = |name: &str| r.get::<_, Option<i64>>(name).map(|v| v as u64);
    RunPropertyRow {
        idx: r.get::<_, i32>("idx") as u32,
        status: r.get("status"),
        trials: n("trials"),
        trials_rated: n("trials_rated"),
        trials_unreported: n("trials_unreported"),
        trials_degraded: n("trials_degraded"),
        trials_perturbing: n("trials_perturbing"),
        trials_conflicted: n("trials_conflicted"),
        trials_inconclusive_with_failures: n("trials_inconclusive_with_failures"),
        trials_claimable: n("trials_claimable"),
        trials_hit: n("trials_hit"),
        trials_true: n("trials_true"),
        trials_false: n("trials_false"),
        evals: n("evals"),
        evals_true: n("evals_true"),
        evals_false: n("evals_false"),
        rate_basis: r.get("rate_basis"),
        rate_k: opt("rate_k"),
        rate_n: opt("rate_n"),
        first_failure_seed: opt("first_failure_seed"),
    }
}

struct IngestRun {
    open: bool,
    ingest_attempt: i64,
    catalog_size: i64,
    kept_bytes: i64,
    digest: DigestUse,
}

/// A new attempt of the job starts the run's trials over; the earlier attempt's rows are removed. The run's tree nodes
/// refer to its trial rows, so they go first. Returns the reason, with nothing changed, when other rows depend on this
/// run's nodes (branch runs of the tree it roots, nodes built on its nodes).
fn start_attempt(
    tx: &mut Transaction,
    run_id: &str,
    attempt: i64,
) -> Result<std::result::Result<(), String>> {
    if let Some(why) = lineage::attempt_blocked(tx, run_id)? {
        return Ok(Err(why));
    }
    let mut blobs: Vec<String> = tx
        .query(
            "SELECT detail_ref FROM trials WHERE run_id=$1 AND detail_ref IS NOT NULL UNION SELECT output_ref FROM trials WHERE run_id=$1 AND output_ref IS NOT NULL",
            &[&run_id],
        )?
        .iter()
        .map(|r| r.get(0))
        .collect();
    blobs.extend(lineage::drop_run_lineage(tx, run_id)?);
    for table in [
        "trials",
        "run_catalog",
        "run_properties",
        "run_digest_use",
        "run_coverage",
    ] {
        tx.execute(&format!("DELETE FROM {table} WHERE run_id=$1"), &[&run_id])?;
    }
    tx.execute(
        "UPDATE runs SET ingest_attempt=$1, catalog_size=0, kept_bytes=0 WHERE id=$2",
        &[&attempt, &run_id],
    )?;
    // Delete the blobs no remaining trial row or failure refers to.
    for h in &blobs {
        tx.execute(
            "DELETE FROM blobs WHERE hash=$1 AND NOT EXISTS (SELECT 1 FROM trials WHERE detail_ref=$1 OR output_ref=$1) AND NOT EXISTS (SELECT 1 FROM failures WHERE bundle_hash=$1) AND NOT EXISTS (SELECT 1 FROM trial_lineage WHERE edge_ref=$1)",
            &[h],
        )?;
    }
    Ok(Ok(()))
}

type Applied = std::result::Result<i64, String>;

/// Applies one catalog row: the next index, or the one-time decision of an undecided row.
fn apply_catalog_row(
    tx: &mut Transaction,
    run_id: &str,
    size: i64,
    row: &CatalogRowRecord,
) -> Result<Applied> {
    let idx = row.idx as i32;
    let line = row.loc_line.map(to_db);
    if i64::from(row.idx) == size {
        if size as usize >= MAX_CATALOG_ROWS {
            return Ok(Err(format!(
                "a run may hold at most {MAX_CATALOG_ROWS} catalog rows"
            )));
        }
        let taken = tx.query_opt(
            "SELECT idx FROM run_catalog WHERE run_id=$1 AND prop_id=$2",
            &[&run_id, &row.prop_id],
        )?;
        if taken.is_some() {
            return Ok(Err("a catalog id was added twice".to_string()));
        }
        tx.execute(
            "INSERT INTO run_catalog(run_id,idx,prop_id,kind,must_hit,msg,loc_file,loc_line,loc_func,decided) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[&run_id, &idx, &row.prop_id, &row.kind, &row.must_hit, &row.msg, &row.loc_file, &line, &row.loc_func, &row.decided],
        )?;
        return Ok(Ok(size + 1));
    }
    if i64::from(row.idx) > size {
        return Ok(Err("a catalog row skips an index".to_string()));
    }
    let existing = tx.query_opt(
        "SELECT * FROM run_catalog WHERE run_id=$1 AND idx=$2",
        &[&run_id, &idx],
    )?;
    let Some(existing) = existing.map(|r| catalog_from_row(&r)) else {
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
        "UPDATE run_catalog SET kind=$1, must_hit=$2, msg=$3, loc_file=$4, loc_line=$5, loc_func=$6, decided=$7 WHERE run_id=$8 AND idx=$9",
        &[&row.kind, &row.must_hit, &row.msg, &row.loc_file, &line, &row.loc_func, &row.decided, &run_id, &idx],
    )?;
    Ok(Ok(size))
}

fn put_blob(tx: &mut Transaction, data: &[u8]) -> Result<String> {
    let hash = sha256_hex(data);
    tx.execute(
        "INSERT INTO blobs(hash,data) VALUES ($1,$2) ON CONFLICT DO NOTHING",
        &[&hash, &data],
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
fn keep(tx: &mut Transaction, t: &NewTrial, used: i64, cap: u64) -> Result<Kept> {
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
    let detail_ref = match t.detail.as_deref() {
        Some(d) => Some(put_blob(tx, d)?),
        None => None,
    };
    let output_ref = match t.output.as_deref() {
        Some(o) => Some(put_blob(tx, o)?),
        None => None,
    };
    Ok(Kept {
        state: KeptState::Kept,
        detail_ref,
        output_ref,
        bytes: size,
    })
}

struct Who<'a> {
    worker_id: &'a str,
    trusted: bool,
    attempt: i64,
}

fn insert_trial(
    tx: &mut Transaction,
    run_id: &str,
    who: &Who,
    t: &NewTrial,
    kept: &Kept,
) -> Result<()> {
    let (seq, seed) = (to_db(t.seq), t.seed as i64);
    let (syscalls, work_ops, elapsed) = (to_db(t.syscalls), to_db(t.work_ops), to_db(t.elapsed_ms));
    let verdict = t.verdict.as_str();
    let kept_state = kept.state.as_str();
    tx.execute(
        &format!("INSERT INTO trials({TRIAL_COLUMNS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25)"),
        &[
            &run_id, &seq, &seed, &verdict, &t.reason, &t.exit_code, &t.signal, &t.timed_out, &t.degraded,
            &t.perturbing, &t.report_missing, &t.rated, &t.claimable, &t.behaviour_hash, &t.schedule_fp,
            &syscalls, &work_ops, &elapsed, &who.worker_id, &who.trusted, &who.attempt, &kept_state,
            &kept.detail_ref, &kept.output_ref, &t.digest,
        ],
    )?;
    Ok(())
}

fn existing_trial(tx: &mut Transaction, run_id: &str, seq: u64) -> Result<Option<TrialRecord>> {
    let seq = to_db(seq);
    Ok(tx
        .query_opt(
            &format!("SELECT {TRIAL_COLUMNS} FROM trials WHERE run_id=$1 AND seq=$2"),
            &[&run_id, &seq],
        )?
        .map(|r| trial_from_row(&r)))
}

fn load_run(tx: &mut Transaction, run_id: &str) -> Result<Option<IngestRun>> {
    let row = tx.query_opt(
        "SELECT state, ingest_attempt, catalog_size, kept_bytes FROM runs WHERE id=$1 FOR UPDATE",
        &[&run_id],
    )?;
    let Some(r) = row else { return Ok(None) };
    let state: String = r.get(0);
    Ok(Some(IngestRun {
        open: RunState::parse(&state).is_some_and(|s| !s.is_terminal()),
        ingest_attempt: r.get(1),
        catalog_size: r.get(2),
        kept_bytes: r.get(3),
        digest: explorer::load_digest_use(tx, run_id)?,
    }))
}

enum Placed {
    Inserted,
    Duplicate,
    Rejected(String),
}

/// Stores one trial of a batch, or recognises it as one already stored. Charges the digest it stores to `run`.
fn place_trial(
    tx: &mut Transaction,
    run_id: &str,
    run: &mut IngestRun,
    who: &Who,
    batch: &TrialBatch,
    t: &NewTrial,
) -> Result<Placed> {
    if t.max_idx.is_some_and(|m| i64::from(m) >= run.catalog_size) {
        return Ok(Placed::Rejected(format!(
            "trial {} refers to an id the catalog does not hold",
            t.seq
        )));
    }
    if let Some(stored) = existing_trial(tx, run_id, t.seq)? {
        if !stored.same_as_stored_form_of(t) {
            return Ok(Placed::Rejected(format!(
                "trial {} was already stored with different content",
                t.seq
            )));
        }
        return Ok(Placed::Duplicate);
    }
    let (form, replaced) = match digest_form(t, run.digest.bytes, batch.digest_bytes_cap) {
        DigestForm::Original(t) => (t, false),
        DigestForm::Replaced(stand_in) => (stand_in, true),
        DigestForm::Refused => {
            return Ok(Placed::Rejected(format!(
                "trial {} would pass the run's digest allowance and offers no stand-in",
                t.seq
            )))
        }
    };
    let kept = keep(tx, form, run.kept_bytes, batch.kept_bytes_cap)?;
    run.kept_bytes += kept.bytes;
    run.digest = run.digest.after(form, replaced);
    insert_trial(tx, run_id, who, form, &kept)?;
    Ok(Placed::Inserted)
}

pub(crate) fn ingest(
    c: &mut Client,
    job_id: &str,
    worker_id: &str,
    worker_trusted: bool,
    batch: &TrialBatch,
) -> Result<IngestOutcome> {
    if let Err(why) = batch.check() {
        return Ok(IngestOutcome::Rejected(why));
    }
    let mut tx = c.transaction()?;
    let job = tx.query_opt(
        "SELECT run_id, status, lease_owner, attempts FROM jobs WHERE id=$1 FOR UPDATE",
        &[&job_id],
    )?;
    let Some(job) = job else {
        return Ok(IngestOutcome::NotLeased);
    };
    let (run_id, status, owner, attempts): (String, String, Option<String>, i32) =
        (job.get(0), job.get(1), job.get(2), job.get(3));
    if status != "running" || owner.as_deref() != Some(worker_id) {
        return Ok(IngestOutcome::NotLeased);
    }
    let attempt = i64::from(attempts);
    // Locking the run row serialises concurrent batches for one run.
    let Some(mut run) = load_run(&mut tx, &run_id)? else {
        return Ok(IngestOutcome::NotLeased);
    };
    if !run.open {
        return Ok(IngestOutcome::RunNotOpen);
    }
    if attempt < run.ingest_attempt {
        return Ok(IngestOutcome::Rejected(
            "this attempt of the job is no longer current".to_string(),
        ));
    }
    if attempt > run.ingest_attempt {
        if let Err(why) = start_attempt(&mut tx, &run_id, attempt)? {
            return Ok(IngestOutcome::Rejected(why));
        }
        run.catalog_size = 0;
        run.kept_bytes = 0;
        run.digest = DigestUse::default();
    }
    for row in &batch.catalog {
        match apply_catalog_row(&mut tx, &run_id, run.catalog_size, row)? {
            Ok(size) => run.catalog_size = size,
            Err(why) => return Ok(IngestOutcome::Rejected(why)),
        }
    }
    let who = Who {
        worker_id,
        trusted: worker_trusted,
        attempt,
    };
    let (mut inserted, mut duplicates) = (0u32, 0u32);
    for t in &batch.trials {
        match place_trial(&mut tx, &run_id, &mut run, &who, batch, t)? {
            Placed::Inserted => inserted += 1,
            Placed::Duplicate => duplicates += 1,
            Placed::Rejected(why) => return Ok(IngestOutcome::Rejected(why)),
        }
    }
    tx.execute(
        "UPDATE runs SET catalog_size=$1, kept_bytes=$2 WHERE id=$3",
        &[&run.catalog_size, &run.kept_bytes, &run_id],
    )?;
    explorer::save_digest_use(&mut tx, &run_id, run.digest)?;
    tx.commit()?;
    Ok(IngestOutcome::Applied {
        inserted,
        duplicates,
    })
}

/// Replaces a run's property rows and writes the run-level figures. Called inside the transaction that finishes the job.
pub(crate) fn write_run_stats(tx: &mut Transaction, run_id: &str, s: &RunStats) -> Result<()> {
    tx.execute(
        "UPDATE runs SET inconclusive=$1, distinct_schedules=$2, work=$3, stopped_at_first_failure=$4, ingest_incomplete=$5, property_summary_json=$6 WHERE id=$7",
        &[
            &to_db(s.inconclusive), &to_db(s.distinct_schedules), &to_db(s.work), &s.stopped_at_first_failure,
            &s.ingest_incomplete, &s.property_summary_json, &run_id,
        ],
    )?;
    tx.execute("DELETE FROM run_properties WHERE run_id=$1", &[&run_id])?;
    for p in &s.properties {
        let idx = p.idx as i32;
        let n = [
            to_db(p.trials),
            to_db(p.trials_rated),
            to_db(p.trials_unreported),
            to_db(p.trials_degraded),
            to_db(p.trials_perturbing),
            to_db(p.trials_conflicted),
            to_db(p.trials_inconclusive_with_failures),
            to_db(p.trials_claimable),
            to_db(p.trials_hit),
            to_db(p.trials_true),
            to_db(p.trials_false),
            to_db(p.evals),
            to_db(p.evals_true),
            to_db(p.evals_false),
        ];
        let (rate_k, rate_n) = (p.rate_k.map(to_db), p.rate_n.map(to_db));
        let first = p.first_failure_seed.map(|s| s as i64);
        tx.execute(
            "INSERT INTO run_properties(run_id,idx,status,trials,trials_rated,trials_unreported,trials_degraded,trials_perturbing,trials_conflicted,trials_inconclusive_with_failures,trials_claimable,trials_hit,trials_true,trials_false,evals,evals_true,evals_false,rate_basis,rate_k,rate_n,first_failure_seed) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)",
            &[
                &run_id, &idx, &p.status, &n[0], &n[1], &n[2], &n[3], &n[4], &n[5], &n[6], &n[7], &n[8], &n[9],
                &n[10], &n[11], &n[12], &n[13], &p.rate_basis, &rate_k, &rate_n, &first,
            ],
        )?;
    }
    explorer::write_coverage(tx, run_id, s.coverage_json.as_deref())
}

pub(crate) fn count_trials(c: &mut Client, run_id: &str) -> Result<u64> {
    Ok(
        c.query_one("SELECT COUNT(*) FROM trials WHERE run_id=$1", &[&run_id])?
            .get::<_, i64>(0) as u64,
    )
}

pub(crate) fn get_trial(c: &mut Client, run_id: &str, seq: u64) -> Result<Option<TrialRecord>> {
    let seq = to_db(seq);
    Ok(c.query_opt(
        &format!("SELECT {TRIAL_COLUMNS} FROM trials WHERE run_id=$1 AND seq=$2"),
        &[&run_id, &seq],
    )?
    .map(|r| trial_from_row(&r)))
}

/// `after` is exclusive; `None` starts from the first row.
fn after_bound(after: Option<u64>) -> i64 {
    after.map_or(-1, to_db)
}

pub(crate) fn list_trials(
    c: &mut Client,
    run_id: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<Vec<TrialRecord>> {
    let (after, limit) = (after_bound(after), i64::from(limit));
    Ok(c.query(
        &format!(
            "SELECT {TRIAL_COLUMNS} FROM trials WHERE run_id=$1 AND seq>$2 ORDER BY seq LIMIT $3"
        ),
        &[&run_id, &after, &limit],
    )?
    .iter()
    .map(trial_from_row)
    .collect())
}

pub(crate) fn list_trial_digests(
    c: &mut Client,
    run_id: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<Vec<TrialDigestRow>> {
    let (after, limit) = (after_bound(after), i64::from(limit));
    Ok(c.query(
        "SELECT seq, seed, digest FROM trials WHERE run_id=$1 AND seq>$2 ORDER BY seq LIMIT $3",
        &[&run_id, &after, &limit],
    )?
    .iter()
    .map(|r| TrialDigestRow {
        seq: r.get::<_, i64>(0) as u64,
        seed: r.get::<_, i64>(1) as u64,
        digest: r.get(2),
    })
    .collect())
}

pub(crate) fn list_trial_strip_rows(
    c: &mut Client,
    run_id: &str,
    after: Option<u64>,
    limit: u32,
) -> Result<Vec<TrialStripRow>> {
    let (after, limit) = (after_bound(after), i64::from(limit));
    Ok(c.query(
        "SELECT seq, verdict, rated, degraded, report_missing FROM trials \
         WHERE run_id=$1 AND seq>$2 ORDER BY seq LIMIT $3",
        &[&run_id, &after, &limit],
    )?
    .iter()
    .map(|r| {
        let verdict: String = r.get(1);
        TrialStripRow {
            seq: r.get::<_, i64>(0) as u64,
            verdict: TrialVerdict::parse(&verdict).unwrap_or(TrialVerdict::Infra),
            rated: r.get(2),
            degraded: r.get(3),
            report_missing: r.get(4),
        }
    })
    .collect())
}

pub(crate) fn get_catalog(c: &mut Client, run_id: &str) -> Result<Vec<CatalogRowRecord>> {
    Ok(c.query(
        "SELECT * FROM run_catalog WHERE run_id=$1 ORDER BY idx",
        &[&run_id],
    )?
    .iter()
    .map(catalog_from_row)
    .collect())
}

pub(crate) fn list_run_properties(c: &mut Client, run_id: &str) -> Result<Vec<RunPropertyRow>> {
    Ok(c.query(
        "SELECT * FROM run_properties WHERE run_id=$1 ORDER BY idx",
        &[&run_id],
    )?
    .iter()
    .map(property_from_row)
    .collect())
}

pub(crate) fn run_contributors(c: &mut Client, run_id: &str) -> Result<Vec<Contributor>> {
    Ok(c
        .query(
            "SELECT worker_id, BOOL_AND(worker_trusted), COUNT(*) FROM trials WHERE run_id=$1 GROUP BY worker_id ORDER BY MIN(seq)",
            &[&run_id],
        )?
        .iter()
        .map(|r| Contributor {
            worker_id: r.get(0),
            trusted: r.get(1),
            trials: r.get::<_, i64>(2) as u64,
        })
        .collect())
}

pub(crate) fn set_worker_secret_hash(c: &mut Client, id: &str, hash: &str) -> Result<()> {
    let n = c.execute(
        "UPDATE workers SET secret_hash=$1 WHERE id=$2",
        &[&hash, &id],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("worker {id}")));
    }
    Ok(())
}

pub(crate) fn worker_secret_hash(c: &mut Client, id: &str) -> Result<Option<String>> {
    Ok(
        c.query_opt("SELECT secret_hash FROM workers WHERE id=$1", &[&id])?
            .and_then(|r| r.get::<_, Option<String>>(0)),
    )
}

pub(crate) fn set_worker_trusted(c: &mut Client, id: &str, trusted: bool) -> Result<()> {
    let n = c.execute(
        "UPDATE workers SET trusted=$1 WHERE id=$2",
        &[&trusted, &id],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("worker {id}")));
    }
    Ok(())
}

pub(crate) fn worker_trusted(c: &mut Client, id: &str) -> Result<Option<bool>> {
    Ok(
        c.query_opt("SELECT trusted FROM workers WHERE id=$1", &[&id])?
            .map(|r| r.get(0)),
    )
}

/// Finishing a job makes the finishing attempt's rows the run's rows: anything an earlier attempt left is removed.
pub(crate) fn discard_earlier_attempts(
    tx: &mut Transaction,
    run_id: &str,
    attempt: i64,
) -> Result<()> {
    let held: i64 = tx
        .query_one("SELECT ingest_attempt FROM runs WHERE id=$1", &[&run_id])?
        .get(0);
    if held != 0 && held < attempt {
        // Finishing cannot be refused halfway, so the caller's transaction ends in an error and nothing is applied.
        start_attempt(tx, run_id, attempt)?.map_err(StoreError::Backend)?;
    }
    Ok(())
}

//! Taking per-trial records from a worker.
//!
//! A worker is a separate process, possibly remote and possibly compromised, and the text in its records is later shown
//! to people and to AI agents. Every field is therefore re-checked here against the protocol's own limits, the digest
//! is decoded and checked against the catalog the server holds, and a batch is stored all or nothing.
use crate::digest_cap::{is_capped_reason, stand_in};
use crate::query::is_schedule_fingerprint;
use crate::types::IngestReceipt;
use crate::{Result, Service, ServiceError};
use hairspring_engine::config::MAX_TRIALS_CAP;
use hairspring_engine::trial_log::{ReportedVerdict, TrialReport};
use hairspring_props::limits::MAX_IDS_PER_RUN;
use hairspring_props::record::{TrialRecord, MAX_DETAIL_BYTES};
use hairspring_props::trial::failure_basis;
use hairspring_props::valid::is_plain_text;
use hairspring_props::{CatalogRow, CatalogUpdate, RunCatalog, TrialDigest};
use hairspring_store::{
    CatalogRowRecord, IngestOutcome, Job, NewTrial, RunRecord, TrialBatch, TrialVerdict,
    MAX_BATCH_BYTES, MAX_KEPT_OUTPUT_BYTES, MAX_TRIALS_PER_BATCH, MAX_TRIAL_REASON_CHARS,
};

/// Longest behaviour hash or schedule fingerprint a worker may send, in characters.
const MAX_HASH_CHARS: usize = 64;
/// Prefix of the worker ids the process itself uses; remote workers get `wrk_` ids from the server.
const LOCAL_WORKER_PREFIX: &str = "local-";

/// Equality that does not stop at the first difference.
fn same_text(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn invalid(why: impl Into<String>) -> ServiceError {
    ServiceError::InvalidRequest(why.into())
}

fn verdict(v: ReportedVerdict) -> TrialVerdict {
    match v {
        ReportedVerdict::Pass => TrialVerdict::Pass,
        ReportedVerdict::Violation => TrialVerdict::Violation,
        ReportedVerdict::Inconclusive => TrialVerdict::Inconclusive,
        ReportedVerdict::Unusable => TrialVerdict::Unusable,
        ReportedVerdict::Infra => TrialVerdict::Infra,
    }
}

fn hash_field(value: &Option<String>, what: &str) -> Result<()> {
    match value {
        Some(h)
            if h.is_empty()
                || h.len() > MAX_HASH_CHARS
                || !h.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            Err(invalid(format!(
                "{what} must be 1-{MAX_HASH_CHARS} letters and digits"
            )))
        }
        _ => Ok(()),
    }
}

/// A schedule fingerprint must be one the `fp` filter can name: 16 to 64 lower case hex digits.
fn fingerprint_field(value: &Option<String>) -> Result<()> {
    match value {
        Some(f) if !is_schedule_fingerprint(f) => Err(invalid(
            "schedule fingerprint must be 16-64 lower case hex digits",
        )),
        _ => Ok(()),
    }
}

/// The kept output must be `{"stdout": text, "stderr": text}`, plus, for a stream the worker had to cut, the number of
/// bytes it left out (`stdout_omitted_bytes`, `stderr_omitted_bytes`: non-negative integers), and an optional
/// `schedule` object. Nothing else.
fn check_output(text: &str) -> Result<()> {
    if text.len() > MAX_KEPT_OUTPUT_BYTES {
        return Err(invalid("kept output is too large"));
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| invalid("kept output is not valid JSON"))?;
    let Some(object) = value.as_object() else {
        return Err(invalid("kept output must be a JSON object"));
    };
    let shaped = object.iter().all(|(k, v)| match k.as_str() {
        "stdout" | "stderr" => v.is_string(),
        "stdout_omitted_bytes" | "stderr_omitted_bytes" => v.is_u64(),
        // The scheduler summary. Its fields are checked when it is served, so a bad one costs the summary and not
        // the trial.
        "schedule" => v.is_object(),
        _ => false,
    });
    if shaped {
        Ok(())
    } else {
        Err(invalid(
            "kept output may hold only stdout and stderr text and their omitted byte counts",
        ))
    }
}

fn check_detail(text: &str) -> Result<()> {
    if text.len() > MAX_DETAIL_BYTES {
        return Err(invalid("kept detail is too large"));
    }
    TrialRecord::decode(text.as_bytes()).map_err(|e| invalid(e.to_string()))?;
    Ok(())
}

fn row_record(u: &CatalogUpdate) -> CatalogRowRecord {
    CatalogRowRecord {
        idx: u.idx,
        prop_id: u.row.id.clone(),
        kind: u.row.kind.as_str().to_owned(),
        must_hit: u.row.kind.must_hit(),
        msg: u.row.msg.clone(),
        loc_file: u.row.loc.as_ref().map(|l| l.file.clone()),
        loc_line: u.row.loc.as_ref().map(|l| l.line),
        loc_func: u.row.loc.as_ref().and_then(|l| l.func.clone()),
        decided: u.row.decided,
    }
}

/// Applies a report's catalog updates to `catalog`. An update that repeats a row the server already holds is a
/// resend, not a change.
fn apply_updates(mut catalog: RunCatalog, updates: &[CatalogUpdate]) -> Result<RunCatalog> {
    for u in updates {
        if catalog.get(u.idx) == Some(&u.row) {
            continue;
        }
        catalog = catalog
            .apply(u, MAX_IDS_PER_RUN)
            .map_err(|e| invalid(e.to_string()))?;
    }
    Ok(catalog)
}

/// A digest that says a property failed cannot belong to a pass, and a pass cannot have timed out. A trial that
/// sent no events (an excluded digest) can be a pass: there is nothing in it to contradict.
fn consistent(v: ReportedVerdict, d: &TrialDigest) -> bool {
    if v != ReportedVerdict::Pass {
        return true;
    }
    let failed = d
        .entries
        .iter()
        .any(|e| e.catalogued && failure_basis(e.kind, e.hits, e.falses).is_some());
    !d.flags.inconclusive && !failed
}

struct Checked {
    trial: NewTrial,
    rows: Vec<CatalogRowRecord>,
}

/// `limit` is the number of trials the run was started with: a record numbered at or past it is not one of its trials.
fn check_report(r: &TrialReport, catalog: RunCatalog, limit: u64) -> Result<(Checked, RunCatalog)> {
    let label = |what: &str| invalid(format!("trial {}: {what}", r.seq));
    if r.seq >= limit.min(MAX_TRIALS_CAP) {
        return Err(label(
            "sequence number is past the trials this run was started with",
        ));
    }
    if r.reason.chars().count() > MAX_TRIAL_REASON_CHARS || !is_plain_text(&r.reason, usize::MAX) {
        return Err(label(
            "reason is too long or has control or format characters",
        ));
    }
    if is_capped_reason(&r.reason) {
        return Err(label("reason starts with text only the server writes"));
    }
    hash_field(&r.behaviour_hash, "behaviour hash").map_err(|e| label(&e.to_string()))?;
    fingerprint_field(&r.schedule_fp).map_err(|e| label(&e.to_string()))?;
    let limit = i64::MAX as u64;
    if r.syscalls > limit || r.work_ops > limit || r.elapsed_ms > limit {
        return Err(label("a counter is out of range"));
    }
    let digest = TrialDigest::decode(&r.digest).map_err(|e| label(&e.to_string()))?;
    if digest.flags.excluded && (!digest.entries.is_empty() || !r.catalog.is_empty()) {
        return Err(label("an excluded trial carries no ids"));
    }
    if !consistent(r.verdict, &digest) {
        return Err(label("the verdict contradicts the digest"));
    }
    let catalog = apply_updates(catalog, &r.catalog).map_err(|e| label(&e.to_string()))?;
    let max_idx = digest.entries.iter().map(|e| e.idx).max();
    if max_idx.is_some_and(|m| m as usize >= catalog.len()) {
        return Err(label(
            "the digest refers to an id the catalog does not hold",
        ));
    }
    r.detail
        .as_deref()
        .map(check_detail)
        .transpose()
        .map_err(|e| label(&e.to_string()))?;
    r.output
        .as_deref()
        .map(check_output)
        .transpose()
        .map_err(|e| label(&e.to_string()))?;
    let f = digest.flags;
    let rated = !f.excluded && !f.inconclusive && !f.degraded && !f.report_missing;
    let trial = NewTrial {
        seq: r.seq,
        seed: r.seed,
        verdict: verdict(r.verdict),
        reason: r.reason.clone(),
        exit_code: r.exit_code,
        signal: r.signal,
        timed_out: f.inconclusive,
        degraded: f.degraded,
        perturbing: f.perturbing,
        report_missing: f.report_missing,
        rated,
        claimable: rated && !f.perturbing,
        behaviour_hash: r.behaviour_hash.clone(),
        schedule_fp: r.schedule_fp.clone(),
        syscalls: r.syscalls,
        work_ops: r.work_ops,
        elapsed_ms: r.elapsed_ms,
        max_idx,
        digest: r.digest.clone(),
        detail: r.detail.as_ref().map(|d| d.clone().into_bytes()),
        output: r.output.as_ref().map(|o| o.clone().into_bytes()),
        on_digest_cap: None,
    };
    let rows = r.catalog.iter().map(row_record).collect();
    Ok((Checked { trial, rows }, catalog))
}

fn size_of(r: &TrialReport) -> usize {
    r.digest.len()
        + r.detail.as_ref().map_or(0, String::len)
        + r.output.as_ref().map_or(0, String::len)
}

/// Splits records into batches the store will take: at most [`MAX_TRIALS_PER_BATCH`] records and about
/// [`MAX_BATCH_BYTES`] bytes each.
pub(crate) fn batches(reports: &[TrialReport]) -> Vec<&[TrialReport]> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (i, r) in reports.iter().enumerate() {
        let size = size_of(r);
        if i > start && (i - start >= MAX_TRIALS_PER_BATCH || bytes + size > MAX_BATCH_BYTES) {
            out.push(&reports[start..i]);
            (start, bytes) = (i, 0);
        }
        bytes += size;
    }
    if start < reports.len() {
        out.push(&reports[start..]);
    }
    out
}

impl Service {
    /// Whether rows from this worker may back a claim. The process's own workers are trusted; a remote worker is
    /// trusted only when an administrator said so.
    pub(crate) fn worker_is_trusted(&self, worker_id: &str) -> Result<bool> {
        if worker_id.starts_with(LOCAL_WORKER_PREFIX) {
            return Ok(true);
        }
        Ok(self.store.worker_trusted(worker_id)?.unwrap_or(false))
    }

    /// The run's catalog as the server holds it, or an empty one when the job is on a newer attempt than the rows.
    fn catalog_for(&self, run: &RunRecord, job: &Job) -> Result<RunCatalog> {
        if u64::from(job.attempts) > run.ingest_attempt {
            return Ok(RunCatalog::new());
        }
        let rows = self
            .store
            .get_catalog(&run.id)?
            .into_iter()
            .map(|r| stored_row(&r))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| ServiceError::Internal("a stored catalog row is unreadable".into()))?;
        RunCatalog::from_rows(rows).map_err(|e| ServiceError::Internal(e.to_string()))
    }

    /// Stores a batch of trial records from the worker holding the job's lease, all or nothing. Sending a batch
    /// again changes nothing.
    ///
    /// This form is for code running inside the process (the embedded workers and the in-process path of
    /// [`Service::complete`]), which holds the job's lease directly and has no network caller to authenticate. It
    /// skips the credential check, so no network handler may call it: a remote worker uses
    /// [`Service::ingest_trials_with_secret`].
    pub fn ingest_trials(
        &self,
        job_id: &str,
        worker_id: &str,
        reports: &[TrialReport],
    ) -> Result<IngestReceipt> {
        self.ingest_authenticated(job_id, worker_id, reports)
    }

    /// Checks that the caller really is the worker it names. A worker id is not a credential (anyone with a `run`
    /// token can see ids), so a remote worker proves itself with the secret it was given at registration. There is
    /// no exemption by id: the process's own workers never come through here. Trust is only ever read for an
    /// authenticated worker.
    pub fn authenticate_worker(&self, worker_id: &str, secret: Option<&str>) -> Result<()> {
        let denied = || invalid("invalid worker credential");
        let given = secret.filter(|s| !s.is_empty()).ok_or_else(denied)?;
        let stored = self
            .store
            .worker_secret_hash(worker_id)?
            .ok_or_else(denied)?;
        if same_text(&hairspring_store::sha256_hex(given.as_bytes()), &stored) {
            Ok(())
        } else {
            Err(denied())
        }
    }

    /// Stores a batch of trial records from a remote worker, after checking its secret. See
    /// [`Service::ingest_trials`].
    pub fn ingest_trials_with_secret(
        &self,
        job_id: &str,
        worker_id: &str,
        secret: Option<&str>,
        reports: &[TrialReport],
    ) -> Result<IngestReceipt> {
        self.authenticate_worker(worker_id, secret)?;
        self.ingest_authenticated(job_id, worker_id, reports)
    }

    /// Everything after authentication: the caller has already shown it is `worker_id`.
    fn ingest_authenticated(
        &self,
        job_id: &str,
        worker_id: &str,
        reports: &[TrialReport],
    ) -> Result<IngestReceipt> {
        if reports.len() > MAX_TRIALS_PER_BATCH {
            return Err(invalid(format!(
                "a batch may hold at most {MAX_TRIALS_PER_BATCH} trials"
            )));
        }
        let job = self.owned_running_job(job_id, worker_id)?;
        let run = self
            .store
            .get_run(&job.run_id)?
            .ok_or_else(|| ServiceError::NotFound(format!("run {}", job.run_id)))?;
        if run.state.is_terminal() {
            return Err(invalid("the run is already finished"));
        }
        let mut catalog = self.catalog_for(&run, &job)?;
        let limit = crate::sweep::trial_limit(&job.payload_json);
        let (mut trials, mut rows) = (Vec::with_capacity(reports.len()), Vec::new());
        for r in reports {
            let (checked, next) = check_report(r, catalog, limit)?;
            catalog = next;
            rows.extend(checked.rows);
            trials.push(checked.trial);
        }
        // Stand-ins are built up front only when this batch could pass the run's digest allowance by what is known
        // now. Otherwise the store, which decides under the run's lock, asks for them (`NeedsStandIn`) and the batch is
        // sent once more with them.
        let incoming: u64 = trials.iter().map(|t| t.digest.len() as u64).sum();
        let used = self.store.run_digest_use(&run.id)?.bytes;
        let eager = used.saturating_add(incoming) > self.cfg.digest_bytes_per_run;
        let mut batch = TrialBatch {
            catalog: rows,
            trials: if eager {
                with_stand_ins(trials)
            } else {
                trials
            },
            kept_bytes_cap: self.cfg.kept_bytes_per_run,
            digest_bytes_cap: self.cfg.digest_bytes_per_run,
        };
        let trusted = self.worker_is_trusted(worker_id)?;
        let mut outcome = self.send_batch(job_id, worker_id, trusted, &batch)?;
        if outcome == IngestOutcome::NeedsStandIn {
            batch.trials = with_stand_ins(std::mem::take(&mut batch.trials));
            outcome = self.send_batch(job_id, worker_id, trusted, &batch)?;
        }
        match outcome {
            IngestOutcome::Applied {
                inserted,
                duplicates,
            } => {
                self.heartbeat(job_id, worker_id)?;
                Ok(IngestReceipt {
                    inserted,
                    duplicates,
                })
            }
            IngestOutcome::NotLeased => Err(invalid("job is not leased by this worker")),
            IngestOutcome::RunNotOpen => Err(invalid("the run is already finished")),
            IngestOutcome::Rejected(why) => Err(invalid(why)),
            // Every trial offered its stand-in, so the store cannot ask again.
            IngestOutcome::NeedsStandIn => Err(ServiceError::Internal(
                "the store asked for digest stand-ins that were already offered".into(),
            )),
        }
    }

    fn send_batch(
        &self,
        job_id: &str,
        worker_id: &str,
        trusted: bool,
        batch: &TrialBatch,
    ) -> Result<IngestOutcome> {
        Ok(self.store.ingest_trials(
            job_id,
            worker_id,
            trusted,
            batch,
            hairspring_store::now_ms(),
        )?)
    }
}

/// Each trial with the stand-in the store keeps if its digest does not fit.
fn with_stand_ins(trials: Vec<NewTrial>) -> Vec<NewTrial> {
    trials
        .into_iter()
        .map(|t| NewTrial {
            on_digest_cap: Some(Box::new(stand_in(&t))),
            ..t
        })
        .collect()
}

/// A stored catalog row as a protocol row, or `None` if its kind is not one this build knows.
pub(crate) fn stored_row(r: &CatalogRowRecord) -> Option<CatalogRow> {
    Some(CatalogRow {
        id: r.prop_id.clone(),
        kind: hairspring_props::AssertKind::from_wire(&r.kind)?,
        msg: r.msg.clone(),
        loc: r.loc_file.as_ref().map(|file| hairspring_props::Loc {
            file: file.clone(),
            line: r.loc_line.unwrap_or(0),
            func: r.loc_func.clone(),
        }),
        decided: r.decided,
    })
}

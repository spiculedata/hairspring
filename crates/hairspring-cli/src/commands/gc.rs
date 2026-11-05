//! `hairspring gc`: retention. It reports what it would remove and removes nothing unless `--apply` is given together
//! with the report's `--cutoff`, `--expect-runs` and `--expect-digest`: the same cutoff makes the same runs old, and
//! the count and digest prove the runs found are the runs the report showed.
//!
//! What is removed is decided by the store (`Store::retention_delete`), which re-checks every run inside the
//! transaction that deletes it. This command adds the age and state selection, the guard, the report, and the removal
//! of bundle files from the bundle directory. The order is crash safe: the store commits first, then the files go. A
//! crash between the two leaves bundle files nothing refers to (the stored copy of the bundle is gone with its run, so
//! a later `gc` cannot find them: delete them by hand); it never leaves a run, failure or check that points at
//! something missing.
use super::gc_files::{self, BundleFiles, Removal};
use super::gc_report::{self, Header};
use crate::cli::{GcArgs, GcState};
use crate::ctx::{CmdResult, Ctx};
use hairspring_store::{now_ms, sha256_hex, RetentionReport, RunState, Store};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

const MS_PER_HOUR: i64 = 3_600_000;
/// Hex digits of the run-set digest shown in the report.
const DIGEST_LEN: usize = 16;
/// Said when another `gc` holds the store's retention lock.
pub const ANOTHER_GC: &str =
    "another hairspring gc is running against this database; nothing was looked at or removed. Run it again when that one has finished";

/// Parses `30d`, `12h` or `2w` into milliseconds. A whole number above zero and one of those three units.
pub fn parse_age(text: &str) -> Result<i64, String> {
    let bad = || {
        format!(
            "`{text}` is not an age: use a whole number above zero and h, d or w (for example 30d)"
        )
    };
    let unit = text.chars().last().ok_or_else(bad)?;
    let hours_per_unit: i64 = match unit {
        'h' => 1,
        'd' => 24,
        'w' => 24 * 7,
        _ => return Err(bad()),
    };
    let digits = &text[..text.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let n: i64 = digits.parse().map_err(|_| bad())?;
    if n == 0 {
        return Err(bad());
    }
    n.checked_mul(hours_per_unit)
        .and_then(|h| h.checked_mul(MS_PER_HOUR))
        .ok_or_else(|| format!("`{text}` is too long an age"))
}

/// The cutoff to use: the one given (from an earlier report), or `--older-than` before `now`. A given cutoff may not
/// be later than `--older-than` allows, so it can never make younger runs old.
pub fn cutoff_for(given: Option<i64>, older_than: &str, now: i64) -> Result<i64, String> {
    let age_ms = parse_age(older_than)?;
    let latest = now.saturating_sub(age_ms).max(0);
    match given {
        None => Ok(latest),
        Some(c) if c < 0 => Err(format!(
            "--cutoff {c} is not a time: use the cutoff a report printed (milliseconds since 1970)"
        )),
        Some(c) if c > latest => Err(format!(
            "--cutoff {c} is later than --older-than {older_than} allows now ({latest}); nothing was removed"
        )),
        Some(c) => Ok(c),
    }
}

/// "1 run", "2 runs".
pub fn runs_text(n: u64) -> String {
    if n == 1 {
        "1 run".to_owned()
    } else {
        format!("{n} runs")
    }
}

/// A short digest of the set of run ids (order does not matter), so an apply can check it removes the runs the report
/// showed and not merely as many.
pub fn runs_digest(run_ids: &[String]) -> String {
    let mut ids: Vec<&str> = run_ids.iter().map(String::as_str).collect();
    ids.sort_unstable();
    sha256_hex(ids.join("\n").as_bytes())[..DIGEST_LEN].to_owned()
}

fn selected_states(args: &GcArgs) -> Vec<RunState> {
    let chosen: Vec<GcState> = if args.states.is_empty() {
        vec![GcState::Passed, GcState::Error, GcState::Cancelled]
    } else {
        args.states.clone()
    };
    let mut states: Vec<RunState> = chosen.iter().map(|s| s.run_state()).collect();
    states.sort_by_key(|s| s.as_str());
    states.dedup();
    states
}

/// What a pass removes (or would), with the bundle files. For an apply, `removal` says what happened to the files and
/// `file_error` why they could not be looked for.
pub struct Section {
    pub report: RetentionReport,
    pub files: BundleFiles,
    pub removal: Option<Removal>,
    pub file_error: Option<String>,
}

impl Section {
    pub fn bundle_files(&self) -> (u64, u64) {
        match &self.removal {
            Some(r) => (r.removed, r.bytes),
            None => (self.files.files.len() as u64, self.files.bytes),
        }
    }
}

/// The arguments that remove exactly what a report shows.
pub fn apply_args(cutoff: i64, runs: u64, digest: &str) -> String {
    format!("--apply --cutoff {cutoff} --expect-runs {runs} --expect-digest {digest}")
}

pub fn run(ctx: &Ctx, args: &GcArgs, out: &mut dyn Write, err: &mut dyn Write) -> CmdResult<i32> {
    let cutoff = cutoff_for(args.cutoff, &args.older_than, now_ms())?;
    let states = selected_states(args);
    let store = ctx.store()?;
    // One gc at a time per database: held until this function returns.
    let Some(_gc_lock) = store.retention_lock()? else {
        writeln!(err, "hairspring: {ANOTHER_GC}")?;
        return Ok(crate::EXIT_ERROR);
    };
    let (ids, skipped) = select(&*store, cutoff, &states)?;

    let plan = store.retention_delete(&ids, cutoff, true)?;
    let plan_files = gc_files::find(&args.bundle_dir, &plan.bundle_hashes)?;
    let planned = Section {
        report: plan,
        files: plan_files,
        removal: None,
        file_error: None,
    };
    let digest = runs_digest(&planned.report.run_ids);

    let mut applied: Option<Section> = None;
    let mut refusal: Option<String> = None;
    if args.apply {
        refusal = apply_refusal(args, cutoff, planned.report.runs_deleted(), &digest);
        if refusal.is_none() {
            applied = Some(apply(&*store, &planned, cutoff, &args.bundle_dir)?);
        }
    }

    let header = Header {
        older_than: &args.older_than,
        cutoff,
        digest: &digest,
        states: &states,
        skipped: &skipped,
    };
    let problems = problems_of(&planned, applied.as_ref());
    if args.json {
        gc_report::write_json(
            out,
            &header,
            &planned,
            applied.as_ref(),
            refusal.as_deref(),
            &problems,
        )?;
    } else {
        gc_report::write_text(out, &header, &planned, applied.as_ref(), args.apply)?;
    }
    if let Some(why) = &refusal {
        writeln!(err, "hairspring: {why}")?;
    }
    for p in &problems {
        writeln!(err, "hairspring: {p}")?;
    }
    let failed = refusal.is_some() || !problems.is_empty();
    Ok(if failed {
        crate::EXIT_ERROR
    } else {
        crate::EXIT_OK
    })
}

/// Why `--apply` may not go ahead, or `None`. It needs the report's cutoff, run count and digest, and the runs found
/// now must match the count and the digest.
fn apply_refusal(args: &GcArgs, cutoff: i64, found: u64, digest: &str) -> Option<String> {
    let (Some(_), Some(n), Some(d)) =
        (args.cutoff, args.expect_runs, args.expect_digest.as_deref())
    else {
        return Some(format!(
            "refusing to remove anything without {} (from the report); nothing was removed",
            apply_args(cutoff, found, digest).trim_start_matches("--apply ")
        ));
    };
    if n != found {
        return Some(format!(
            "refusing to remove anything: --expect-runs is {n} but the report found {}; nothing was removed",
            runs_text(found)
        ));
    }
    if d != digest {
        return Some(format!(
            "refusing to remove anything: these are not the runs the report showed (--expect-digest is {d}, the runs found give {digest}); nothing was removed"
        ));
    }
    None
}

/// The runs to name to the store, and how many old runs were left out of the selection (by reason).
fn select(
    store: &dyn Store,
    cutoff: i64,
    states: &[RunState],
) -> CmdResult<(Vec<String>, BTreeMap<String, u64>)> {
    let mut ids = Vec::new();
    let mut skipped: BTreeMap<String, u64> = BTreeMap::new();
    for c in store.retention_scan(cutoff)? {
        if !c.state.is_terminal() {
            *skipped.entry("active".to_owned()).or_insert(0) += 1;
        } else if states.contains(&c.state) {
            ids.push(c.id);
        } else {
            *skipped
                .entry(format!("not_selected_{}", c.state.as_str()))
                .or_insert(0) += 1;
        }
    }
    Ok((ids, skipped))
}

/// Removes exactly the runs the report listed, then the bundle files of the bundles that were deleted. A run that stopped
/// qualifying in between is kept by the store, with its reason in the apply's own `kept`. Once the store has
/// committed, nothing here fails the call: a bundle directory that cannot be read is a problem in the result, so what
/// was removed is still reported.
fn apply(
    store: &dyn Store,
    planned: &Section,
    cutoff: i64,
    bundle_dir: &Path,
) -> CmdResult<Section> {
    let report = store.retention_delete(&planned.report.run_ids, cutoff, false)?;
    let (files, file_error) = match gc_files::find(bundle_dir, &report.bundle_hashes) {
        Ok(files) => (files, None),
        Err(e) => (
            BundleFiles::default(),
            Some(format!(
                "could not read the bundle directory {}: {e}; no bundle files were removed",
                bundle_dir.display()
            )),
        ),
    };
    let removal = gc_files::remove(bundle_dir, &files);
    Ok(Section {
        report,
        files,
        removal: Some(removal),
        file_error,
    })
}

/// What went wrong in an apply, one line each. Any line makes the exit status 2.
fn problems_of(planned: &Section, applied: Option<&Section>) -> Vec<String> {
    let Some(a) = applied else { return Vec::new() };
    let mut problems = Vec::new();
    if let Some(e) = &a.report.error {
        problems.push(format!(
            "stopped part-way: {e}; the runs counted under \"removed\" are removed, the others are untouched"
        ));
    } else if a.report.runs_deleted() < planned.report.runs_deleted() {
        problems.push(format!(
            "removed {} of the {} in the report; the others no longer qualified and were kept (see \"kept when removing\")",
            a.report.runs_deleted(),
            runs_text(planned.report.runs_deleted())
        ));
    }
    if let Some(e) = &a.file_error {
        problems.push(e.clone());
    }
    if let Some(r) = &a.removal {
        problems.extend(
            r.failed
                .iter()
                .map(|f| format!("bundle file not removed: {f}")),
        );
    }
    problems
}

#[cfg(test)]
#[path = "gc_tests.rs"]
mod tests;

//! `hairspring gc`: retention. It reports what it would remove and removes nothing unless `--apply` is given together
//! with `--expect-runs N`, the exact number of runs the report just showed.
//!
//! What is removed is decided by the store (`Store::retention_delete`), which re-checks every run inside the
//! transaction that deletes it. This command adds three things: the age and state selection, the report, and the
//! removal of bundle files from the bundle directory. The order is crash safe: the store commits first, then the files
//! go. A crash between the two leaves bundle files nothing refers to (the stored copy of the bundle is gone with its
//! run, so a later `gc` cannot find them: delete them by hand); it never leaves a run, failure or check that points at
//! something missing.
use super::gc_files::{self, BundleFiles, Removal};
use crate::cli::{GcArgs, GcState};
use crate::ctx::{CmdResult, Ctx};
use crate::timefmt::format_utc;
use hairspring_store::{now_ms, RetentionReport, RunState, Store};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;

const MS_PER_HOUR: i64 = 3_600_000;

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

/// What a pass removes (or would), with the bundle files. For an apply, `removal` says what happened to the files.
struct Section {
    report: RetentionReport,
    files: BundleFiles,
    removal: Option<Removal>,
}

impl Section {
    fn bundle_files(&self) -> (u64, u64) {
        match &self.removal {
            Some(r) => (r.removed, r.bytes),
            None => (self.files.files.len() as u64, self.files.bytes),
        }
    }
}

pub fn run(ctx: &Ctx, args: &GcArgs, out: &mut dyn Write, err: &mut dyn Write) -> CmdResult<i32> {
    let age_ms = parse_age(&args.older_than)?;
    let states = selected_states(args);
    let cutoff = now_ms().saturating_sub(age_ms).max(0);
    let store = ctx.store()?;
    let (ids, skipped) = select(&*store, cutoff, &states)?;

    let plan = store.retention_delete(&ids, cutoff, true)?;
    let plan_files = gc_files::find(&args.bundle_dir, &plan.bundle_hashes)?;
    let planned = Section {
        report: plan,
        files: plan_files,
        removal: None,
    };

    let wanted = planned.report.runs_deleted();
    let mut applied: Option<Section> = None;
    let mut refusal: Option<String> = None;
    if args.apply {
        match args.expect_runs {
            Some(n) if n == wanted => {
                applied = Some(apply(&*store, &planned, cutoff, &args.bundle_dir)?);
            }
            Some(n) => {
                refusal = Some(format!(
                    "refusing to remove anything: --expect-runs is {n} but the report found {wanted} runs; nothing was removed"
                ));
            }
            None => {
                refusal = Some(format!(
                    "refusing to remove anything without --expect-runs {wanted} (the number of runs in the report); nothing was removed"
                ));
            }
        }
    }

    let header = Header {
        older_than: &args.older_than,
        cutoff,
        states: &states,
        skipped: &skipped,
    };
    let problems = problems_of(&applied);
    if args.json {
        write_json(
            out,
            &header,
            &planned,
            applied.as_ref(),
            refusal.as_deref(),
            &problems,
        )?;
    } else {
        write_text(out, &header, &planned, applied.as_ref(), args)?;
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
/// qualifying in between is kept by the store and shows in the counts.
fn apply(
    store: &dyn Store,
    planned: &Section,
    cutoff: i64,
    bundle_dir: &std::path::Path,
) -> CmdResult<Section> {
    let report = store.retention_delete(&planned.report.run_ids, cutoff, false)?;
    let files = gc_files::find(bundle_dir, &report.bundle_hashes)?;
    let removal = gc_files::remove(&files.files);
    Ok(Section {
        report,
        files,
        removal: Some(removal),
    })
}

fn problems_of(applied: &Option<Section>) -> Vec<String> {
    let Some(a) = applied else { return Vec::new() };
    let mut problems = Vec::new();
    if let Some(e) = &a.report.error {
        problems.push(format!(
            "stopped part-way: {e}; the runs counted above are removed, the others are untouched"
        ));
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

struct Header<'a> {
    older_than: &'a str,
    cutoff: i64,
    states: &'a [RunState],
    skipped: &'a BTreeMap<String, u64>,
}

fn counts(s: &Section) -> Value {
    let r = &s.report;
    json!({
        "runs": r.runs_deleted(),
        "runs_by_state": r.runs_by_state,
        "failures": r.failures,
        "checks": r.checks,
        "trials": r.trials,
        "events": r.events,
        "jobs": r.jobs,
        "trees": r.trees_deleted.len(),
        "tree_nodes": r.nodes,
        "stored_blobs": r.blobs,
        "stored_blob_bytes": r.blob_bytes,
        "bundle_blobs": r.bundle_blobs,
        "bundle_blob_bytes": r.bundle_bytes,
        "bundle_files": s.bundle_files().0,
        "bundle_file_bytes": s.bundle_files().1,
    })
}

fn kept(header: &Header, s: &Section) -> Value {
    let mut m: Map<String, Value> = Map::new();
    for (k, v) in header.skipped {
        m.insert(k.clone(), json!(v));
    }
    for (k, v) in &s.report.kept {
        m.insert((*k).to_owned(), json!(v));
    }
    Value::Object(m)
}

fn write_json(
    out: &mut dyn Write,
    header: &Header,
    planned: &Section,
    applied: Option<&Section>,
    refusal: Option<&str>,
    problems: &[String],
) -> CmdResult<()> {
    let removed = applied.map(counts);
    let doc = json!({
        "dry_run": applied.is_none(),
        "older_than": header.older_than,
        "cutoff": header.cutoff,
        "states": header.states.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "would_remove": counts(planned),
        "kept": kept(header, planned),
        "removed": removed,
        "refused": refusal,
        "problems": problems,
        "not_deleted": NOT_DELETED,
    });
    writeln!(out, "{}", serde_json::to_string(&doc)?)?;
    Ok(())
}

/// Said in every report, so nobody reads "removed" as more than it is.
const NOT_DELETED: [&str; 3] = [
    "the database file does not shrink: SQLite reuses the freed space (run VACUUM to give it back); PostgreSQL reclaims it by vacuuming",
    "bundle files outside the bundle directory, and exported or copied bundles, are not touched",
    "workers, tokens, and runs not selected or not old enough are not touched",
];

fn write_text(
    out: &mut dyn Write,
    header: &Header,
    planned: &Section,
    applied: Option<&Section>,
    args: &GcArgs,
) -> CmdResult<()> {
    writeln!(
        out,
        "runs older than {} (finished before {} UTC), states: {}",
        header.older_than,
        format_utc(header.cutoff),
        header
            .states
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )?;
    let title = match (applied.is_some(), args.apply) {
        (true, _) => "to be removed:",
        (false, true) => "would remove (--apply was refused, so nothing has been changed):",
        (false, false) => "would remove (dry run, nothing has been changed):",
    };
    write_section(out, title, planned)?;
    write_kept(out, header, planned)?;
    if let Some(a) = applied {
        write_section(out, "removed:", a)?;
        for note in &a.files.skipped {
            writeln!(out, "  note: {note}")?;
        }
    } else {
        for note in &planned.files.skipped {
            writeln!(out, "  note: {note}")?;
        }
    }
    writeln!(out, "not deleted:")?;
    for line in NOT_DELETED {
        writeln!(out, "  - {line}")?;
    }
    if applied.is_none() && !args.apply {
        let n = planned.report.runs_deleted();
        if n == 0 {
            writeln!(out, "nothing to remove.")?;
        } else {
            writeln!(
                out,
                "nothing was removed. To remove exactly these {n} runs, run again with --apply --expect-runs {n}."
            )?;
        }
    }
    Ok(())
}

fn write_section(out: &mut dyn Write, title: &str, s: &Section) -> CmdResult<()> {
    let r = &s.report;
    writeln!(out, "{title}")?;
    let by_state = r
        .runs_by_state
        .iter()
        .map(|(k, v)| format!("{k} {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    let by_state = if by_state.is_empty() {
        String::new()
    } else {
        format!("  ({by_state})")
    };
    writeln!(out, "  runs          {:>8}{by_state}", r.runs_deleted())?;
    writeln!(out, "  failures      {:>8}", r.failures)?;
    writeln!(out, "  checks        {:>8}", r.checks)?;
    writeln!(out, "  trials        {:>8}", r.trials)?;
    writeln!(out, "  events        {:>8}", r.events)?;
    writeln!(out, "  jobs          {:>8}", r.jobs)?;
    writeln!(
        out,
        "  trial trees   {:>8}  ({} nodes)",
        r.trees_deleted.len(),
        r.nodes
    )?;
    writeln!(
        out,
        "  stored blobs  {:>8}  ({}), of which failure bundles {} ({})",
        r.blobs,
        human_bytes(r.blob_bytes),
        r.bundle_blobs,
        human_bytes(r.bundle_bytes)
    )?;
    writeln!(
        out,
        "  bundle files  {:>8}  ({})",
        s.bundle_files().0,
        human_bytes(s.bundle_files().1)
    )?;
    Ok(())
}

fn write_kept(out: &mut dyn Write, header: &Header, s: &Section) -> CmdResult<()> {
    let mut lines = Vec::new();
    for (k, v) in header.skipped {
        let text = match k.strip_prefix("not_selected_") {
            Some(state) => {
                format!("{v} {state} runs not selected (add --state {state} to include them)")
            }
            None => format!("{v} runs still queued or running"),
        };
        lines.push(text);
    }
    for (k, v) in &s.report.kept {
        let text = match *k {
            "active_job" => format!("{v} finished runs with a queued or running job"),
            "tree_kept" => format!("{v} runs whose trial tree has a run that is kept"),
            "not_found" => format!("{v} runs that no longer exist"),
            other => format!("{v} runs kept ({other})"),
        };
        lines.push(text);
    }
    if !lines.is_empty() {
        writeln!(out, "kept:")?;
        for l in lines {
            writeln!(out, "  {l}")?;
        }
    }
    Ok(())
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_are_a_whole_number_and_a_unit() {
        assert_eq!(parse_age("1h"), Ok(3_600_000));
        assert_eq!(parse_age("30d"), Ok(30 * 24 * 3_600_000));
        assert_eq!(parse_age("2w"), Ok(14 * 24 * 3_600_000));
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "",
            "d",
            "0d",
            "30",
            "30m",
            "-5d",
            "1.5d",
            "+3d",
            "3 d",
            "d3",
            "٣d",
            "99999999999999999999d",
        ] {
            assert!(parse_age(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(
            parse_age(&format!("{}w", i64::MAX / 1000)).is_err(),
            "overflow is refused, not wrapped"
        );
    }

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
    }
}

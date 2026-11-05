//! The text and JSON reports of `hairspring gc`.
use super::gc::{apply_args, runs_text, Section};
use crate::ctx::CmdResult;
use crate::timefmt::format_utc;
use hairspring_store::RunState;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;

pub struct Header<'a> {
    pub older_than: &'a str,
    pub cutoff: i64,
    pub digest: &'a str,
    pub states: &'a [RunState],
    pub skipped: &'a BTreeMap<String, u64>,
}

/// Said in every report, so nobody reads "removed" as more than it is.
const NOT_DELETED: [&str; 3] = [
    "the database file does not shrink: SQLite reuses the freed space (run VACUUM to give it back); PostgreSQL reclaims it by vacuuming",
    "bundle files outside the bundle directory, and exported or copied bundles, are not touched",
    "workers, tokens, and runs not selected or not old enough are not touched",
];

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

/// The JSON key of a reason the store kept a run for. The store's `active` (found queued or running when it checked)
/// gets its own key, because `active` in `kept` already counts the runs the selection skipped as still queued or
/// running.
fn kept_key(reason: &str) -> String {
    match reason {
        "active" => "active_when_checked".to_owned(),
        other => other.to_owned(),
    }
}

fn kept_reasons(s: &Section) -> Map<String, Value> {
    s.report
        .kept
        .iter()
        .map(|(k, v)| (kept_key(k), json!(v)))
        .collect()
}

fn kept(header: &Header, s: &Section) -> Value {
    let mut m: Map<String, Value> = header
        .skipped
        .iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect();
    m.extend(kept_reasons(s));
    Value::Object(m)
}

pub fn write_json(
    out: &mut dyn Write,
    header: &Header,
    planned: &Section,
    applied: Option<&Section>,
    refusal: Option<&str>,
    problems: &[String],
) -> CmdResult<()> {
    let runs = planned.report.runs_deleted();
    let doc = json!({
        "dry_run": applied.is_none(),
        "older_than": header.older_than,
        "cutoff": header.cutoff,
        "runs_digest": header.digest,
        "apply_args": apply_args(header.cutoff, runs, header.digest).split(' ').collect::<Vec<_>>(),
        "states": header.states.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "would_remove": counts(planned),
        "kept": kept(header, planned),
        "removed": applied.map(counts),
        "kept_when_removing": applied.map(|a| Value::Object(kept_reasons(a))),
        "refused": refusal,
        "problems": problems,
        "not_deleted": NOT_DELETED,
    });
    writeln!(out, "{}", serde_json::to_string(&doc)?)?;
    Ok(())
}

pub fn write_text(
    out: &mut dyn Write,
    header: &Header,
    planned: &Section,
    applied: Option<&Section>,
    asked_to_apply: bool,
) -> CmdResult<()> {
    writeln!(
        out,
        "runs older than {} (finished before {} UTC by this computer's clock; cutoff {}), states: {}",
        header.older_than,
        format_utc(header.cutoff),
        header.cutoff,
        header
            .states
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )?;
    let title = match (applied.is_some(), asked_to_apply) {
        (true, _) => "to be removed:",
        (false, true) => "would remove (--apply was refused, so nothing has been changed):",
        (false, false) => "would remove (dry run, nothing has been changed):",
    };
    write_section(out, title, planned)?;
    let mut kept_lines = skipped_lines(header);
    kept_lines.extend(kept_lines_of(planned));
    write_lines(out, "kept:", &kept_lines)?;
    let notes = match applied {
        Some(a) => {
            write_section(out, "removed:", a)?;
            write_lines(out, "kept when removing:", &kept_lines_of(a))?;
            &a.files.skipped
        }
        None => &planned.files.skipped,
    };
    for note in notes {
        writeln!(out, "  note: {note}")?;
    }
    writeln!(out, "not deleted:")?;
    for line in NOT_DELETED {
        writeln!(out, "  - {line}")?;
    }
    if applied.is_none() && !asked_to_apply {
        let n = planned.report.runs_deleted();
        if n == 0 {
            writeln!(out, "nothing to remove.")?;
        } else {
            let these = if n == 1 {
                "this run".to_owned()
            } else {
                format!("these {n} runs")
            };
            writeln!(
                out,
                "nothing was removed. To remove exactly {these}, run again with {}.",
                apply_args(header.cutoff, n, header.digest)
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

fn skipped_lines(header: &Header) -> Vec<String> {
    header
        .skipped
        .iter()
        .map(|(k, v)| match k.strip_prefix("not_selected_") {
            Some(state) => format!(
                "{} {state} not selected (add --state {state} to include them)",
                runs_text(*v)
            ),
            None => format!("{} still queued or running", runs_text(*v)),
        })
        .collect()
}

fn kept_lines_of(s: &Section) -> Vec<String> {
    s.report
        .kept
        .iter()
        .map(|(k, v)| {
            let n = runs_text(*v);
            match *k {
                "active" => format!("{n} no longer finished (queued or running again)"),
                "active_job" => format!("{n} with a queued or running job"),
                "too_new" => format!("{n} not older than the cutoff"),
                "tree_kept" => format!("{n} whose trial tree has a run that is kept"),
                "not_found" => format!("{n} that no longer exist"),
                other => format!("{n} kept ({other})"),
            }
        })
        .collect()
}

fn write_lines(out: &mut dyn Write, title: &str, lines: &[String]) -> CmdResult<()> {
    if lines.is_empty() {
        return Ok(());
    }
    writeln!(out, "{title}")?;
    for l in lines {
        writeln!(out, "  {l}")?;
    }
    Ok(())
}

pub fn human_bytes(n: u64) -> String {
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

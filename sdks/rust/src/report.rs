//! Writing the lines: the startup catalog and the report at exit. Everything here runs at startup or at exit, never
//! at a call site.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{AcqRel, Relaxed};

use crate::catalog::{self, Totals};
use crate::fmt::Buf;
use crate::lines::{
    self, AssertRow, Built, CatalogRow, EndReason, Overflow, ThreadRow, WitnessOut,
};
use crate::output;
use crate::runtime::{self, Runtime};
use crate::site::WitnessSnap;
use crate::slots::{table, MAX_THREADS};

/// Set by the first report; later calls do nothing.
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Send a built line; one that did not fit, or could not be written, is counted, never truncated.
fn send(built: Built, buf: &Buf) {
    match built {
        Ok(()) => {
            if output::write_line(buf.as_bytes()).is_err() {
                runtime::note_dropped(1);
            }
        }
        Err(Overflow) => runtime::note_dropped(1),
    }
}

/// `hello`, then one `catalog` line per distinct (id, kind) pair, so a program that crashes, hangs or is killed
/// still has a catalog.
pub fn emit_startup(rt: &Runtime) {
    let p = runtime::proc_info(rt);
    let mut b = Buf::new();
    send(lines::hello(&mut b, &p), &b);
    for (site, entry) in rt.sites.iter().zip(&rt.index.entries) {
        if !site.is_live() || entry.canon.is_none() || !entry.emit {
            continue;
        }
        b.clear();
        let row = CatalogRow {
            id: site.id,
            kind: site.kind,
            msg: site.msg,
            file: site.file,
            line: site.line,
        };
        send(lines::catalog(&mut b, &p, &row), &b);
    }
}

/// A thread's totals, read once per report so the thread lines and the end line agree.
struct ThreadSnap {
    key: Vec<u8>,
    evals: u64,
    work: u64,
    tape_consumed: u64,
    is_stable: bool,
}

/// A slot counts only once its key is published: claims are lock-free, so a slot can be claimed a moment before its
/// key is written.
fn snapshot_threads() -> Vec<ThreadSnap> {
    table()
        .slots()
        .iter()
        .filter_map(|slot| {
            Some(ThreadSnap {
                key: slot.key()?,
                evals: slot.evals.load(Relaxed),
                work: slot.work.load(Relaxed),
                tape_consumed: slot.tape_consumed.load(Relaxed),
                is_stable: slot.is_stable.load(Relaxed) != 0,
            })
        })
        .collect()
}

/// A witness with its thread's key looked up.
struct Resolved {
    key: Vec<u8>,
    snap: WitnessSnap,
}

fn resolve(snap: Option<WitnessSnap>) -> Option<Resolved> {
    let snap = snap?;
    let slot = usize::try_from(snap.slot)
        .ok()
        .filter(|&i| i < MAX_THREADS)?;
    Some(Resolved {
        key: table().slot(slot).key().unwrap_or_default(),
        snap,
    })
}

fn witness_out(r: &Option<Resolved>) -> Option<WitnessOut<'_>> {
    r.as_ref().map(|r| WitnessOut {
        key: &r.key,
        ord: r.snap.ord,
        vals: r.snap.vals,
    })
}

fn emit_asserts(rt: &Runtime, totals: &[Totals]) {
    let p = runtime::proc_info(rt);
    let mut b = Buf::new();
    for (i, (site, entry)) in rt.sites.iter().zip(&rt.index.entries).enumerate() {
        if entry.canon != Some(i) {
            continue;
        }
        let t = &totals[i];
        let (first_true, first_false) = (resolve(t.pick_true), resolve(t.pick_false));
        let row = AssertRow {
            id: site.id,
            kind: site.kind,
            trues: t.trues,
            falses: t.falses,
            first_true: witness_out(&first_true),
            first_false: witness_out(&first_false),
        };
        b.clear();
        send(lines::assert_line(&mut b, &p, &row), &b);
    }
}

/// Do the records in the catalog account for every evaluation the threads counted? A record the linker kept out of
/// the section still counts its evaluations in the threads' `evals`, so the totals disagree and the shortfall says
/// how many evaluations were never attributed. See the tolerance in `catalog::in_flight_bound`.
fn audit(rt: &Runtime, reason: EndReason, threads: u64) {
    // With more threads than slots the last slot is shared and its count is racy, so the comparison means nothing.
    if table().used() >= MAX_THREADS {
        return;
    }
    let evals: u64 = table().slots().iter().map(|s| s.evals.load(Relaxed)).sum();
    // Read after `evals`: a surplus here can only hide a loss, never invent one.
    let counted = catalog::counted(rt.sites);
    let missing = catalog::audit_missing(evals, counted, catalog::in_flight_bound(reason, threads));
    if missing == 0 {
        return;
    }
    runtime::note_dropped(1);
    let note = format!(
        "hairspring: incomplete catalog: {missing} evaluation(s) ran at call sites that are missing from the catalog section\n"
    );
    if output::write_line(note.as_bytes()).is_err() {
        runtime::note_dropped(1);
    }
}

fn report_body(rt: &Runtime, reason: EndReason, status: i64) {
    if output::verify() {
        runtime::note_dropped(1); // nothing is written: the descriptor leads elsewhere
    }
    emit_asserts(rt, &catalog::sum(rt.sites, &rt.index));
    let threads = snapshot_threads();
    let evals: u64 = threads.iter().map(|t| t.evals).sum();
    let work: u64 = threads.iter().map(|t| t.work).sum();
    let count = threads.len() as u64;
    audit(rt, reason, count); // before `end`, whose `dropped` it may raise
    let p = runtime::proc_info(rt);
    let mut b = Buf::new();
    send(lines::work(&mut b, &p, work), &b);
    for t in &threads {
        b.clear();
        let row = ThreadRow {
            key: &t.key,
            evals: t.evals,
            tape_consumed: t.tape_consumed,
            is_stable: t.is_stable,
        };
        send(lines::thread(&mut b, &p, &row), &b);
    }
    b.clear();
    let p = runtime::proc_info(rt);
    send(lines::end(&mut b, &p, reason, status, evals, count), &b);
}

/// Write the report: one `assert` per distinct id, `work`, one `thread` per thread, then `end`. At most once per
/// process; the first call wins.
pub fn report_now(reason: EndReason, status: i64) {
    if REPORTED.swap(true, AcqRel) {
        return;
    }
    report_body(runtime::ensure_started(), reason, status);
}

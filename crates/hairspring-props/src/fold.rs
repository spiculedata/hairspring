//! Folding a trial's output lines into [`TrialProps`].
//!
//! Declarations (`hello`, `catalog`, `thread`, `work`, `end`) are applied as they stream past; `assert` and `eval`
//! lines are held until the stream ends and applied after them, so the result does not depend on the order of lines
//! between those two groups. Held events are bounded by the per-trial event byte budget.

use crate::event::*;
use crate::limits::*;
use crate::line::{is_event_candidate, parse_line, IgnoreReason, LineOutcome, RejectReason};
use crate::trial::{
    CatalogConflict, EndReport, EventHealth, ProcReport, TrialProperty, TrialProps, Witness,
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone)]
enum Source<'a> {
    Output(&'a [u8]),
    Lines(Vec<&'a [u8]>),
}

/// The output of one trial (stdout and stderr, in any order the caller chooses).
#[derive(Debug, Clone)]
pub struct TrialEvents<'a> {
    source: Source<'a>,
}

impl Default for TrialEvents<'_> {
    fn default() -> Self {
        TrialEvents {
            source: Source::Output(&[]),
        }
    }
}

impl<'a> TrialEvents<'a> {
    /// Wrap raw output; lines are found lazily when folding, so nothing is allocated per line.
    pub fn from_output(output: &'a [u8]) -> Self {
        TrialEvents {
            source: Source::Output(output),
        }
    }

    pub fn from_lines<I: IntoIterator<Item = &'a [u8]>>(lines: I) -> Self {
        TrialEvents {
            source: Source::Lines(lines.into_iter().collect()),
        }
    }

    /// Fold every line; never panics, never fails. Anything dropped is counted in `health`.
    pub fn fold(&self) -> TrialProps {
        match &self.source {
            Source::Output(out) => Self::fold_stream(out.split(|b| *b == b'\n')),
            Source::Lines(lines) => Self::fold_stream(lines.iter().copied()),
        }
    }

    /// Fold lines as they are produced, without collecting them first.
    pub fn fold_stream<'l, I: IntoIterator<Item = &'l [u8]>>(lines: I) -> TrialProps {
        let mut acc = Accumulator::default();
        for line in lines {
            acc.feed(line);
        }
        acc.finish()
    }
}

type ProcId = (String, String);

#[derive(Default)]
struct Accumulator {
    procs: Vec<ProcReport>,
    props: BTreeMap<String, TrialProperty>,
    declared: HashSet<ProcId>,
    declared_per_proc: HashMap<String, usize>,
    asserted: HashSet<ProcId>,
    conflicted: HashSet<ProcId>,
    conflicts: Vec<CatalogConflict>,
    /// `assert` and `eval` events, applied after all declarations.
    deferred: Vec<Event>,
    perturbing: bool,
    health: EventHealth,
}

impl Accumulator {
    fn finish(mut self) -> TrialProps {
        for event in std::mem::take(&mut self.deferred) {
            match event {
                Event::Assert(a) => self.apply_assert(a),
                Event::Eval(e) => self.apply_eval(e),
                _ => {}
            }
        }
        self.check_reports();
        TrialProps {
            procs: self.procs,
            properties: self.props.into_values().collect(),
            conflicts: self.conflicts,
            perturbing: self.perturbing,
            health: self.health,
        }
    }

    /// Cross-checks between what `end` promised and what arrived.
    fn check_reports(&mut self) {
        for p in &self.procs {
            let Some(end) = &p.end else { continue };
            let declared = self.declared_per_proc.get(&p.tag).copied().unwrap_or(0);
            if end.sites != declared as u64 {
                self.health.sites_mismatch += 1;
            }
            if end.threads != p.threads.len() as u64 {
                self.health.thread_count_mismatch += 1;
            }
            let unreported = self
                .declared
                .iter()
                .filter(|key| key.0 == p.tag && !self.asserted.contains(*key))
                .count();
            self.health.unreported_ids += unreported as u64;
        }
    }

    fn feed(&mut self, line: &[u8]) {
        if !is_event_candidate(line) {
            return;
        }
        let cost = (line.len().min(MAX_LINE_BYTES) + 1) as u64;
        if self.health.event_bytes.saturating_add(cost) > MAX_EVENT_BYTES_PER_TRIAL as u64 {
            self.health.over_limit += 1;
            return;
        }
        self.health.event_bytes += cost;
        match parse_line(line) {
            LineOutcome::NotEvent => {}
            LineOutcome::Rejected(reason) => self.count_rejection(reason),
            LineOutcome::Ignored(IgnoreReason::UnknownKind) => self.health.unknown_kind += 1,
            LineOutcome::Ignored(IgnoreReason::UnsupportedVersion) => {
                self.health.unsupported_version += 1
            }
            LineOutcome::Event {
                event,
                unknown_fields,
            } => {
                self.health.accepted += 1;
                self.health.unknown_fields += u64::from(unknown_fields);
                self.apply(event);
            }
        }
    }

    fn count_rejection(&mut self, reason: RejectReason) {
        match reason {
            RejectReason::TooLong => self.health.too_long += 1,
            RejectReason::InvalidUtf8 => self.health.invalid_utf8 += 1,
            RejectReason::OverLimit(_) => self.health.over_limit += 1,
            _ => self.health.malformed += 1,
        }
    }

    /// Index of the process with this tag, creating it unless the per-trial cap is reached.
    fn proc_slot(&mut self, tag: &str) -> Option<usize> {
        if let Some(i) = self.procs.iter().position(|p| p.tag == tag) {
            return Some(i);
        }
        if self.procs.len() >= MAX_PROCS_PER_TRIAL {
            self.health.over_limit += 1;
            return None;
        }
        self.procs.push(ProcReport {
            tag: tag.to_owned(),
            sdk: None,
            sdk_version: None,
            catalog_hash: None,
            signal_handlers: None,
            work_ops: None,
            end: None,
            threads: Vec::new(),
        });
        Some(self.procs.len() - 1)
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Hello(h) => self.apply_hello(h),
            Event::Work(w) => self.apply_work(w),
            Event::Thread(t) => self.apply_thread(t),
            Event::End(e) => self.apply_end(e),
            Event::Catalog(c) => {
                if self.proc_slot(&c.proc).is_some() {
                    self.declare(&c.proc, &c.id, c.kind, &c.msg, c.loc.as_ref());
                }
            }
            Event::Assert(_) | Event::Eval(_) => {
                if matches!(event, Event::Eval(_)) {
                    self.perturbing = true;
                }
                self.deferred.push(event);
            }
        }
    }

    fn apply_hello(&mut self, h: Hello) {
        let Some(i) = self.proc_slot(&h.proc) else {
            return;
        };
        let p = &mut self.procs[i];
        if p.signal_handlers.is_some() {
            self.health.duplicates += 1;
            return;
        }
        p.sdk = Some(h.sdk);
        p.sdk_version = Some(h.sdk_version);
        p.catalog_hash = Some(h.catalog_hash);
        p.signal_handlers = Some(h.signal_handlers);
    }

    fn apply_work(&mut self, w: Work) {
        let Some(i) = self.proc_slot(&w.proc) else {
            return;
        };
        if self.procs[i].work_ops.is_some() {
            self.health.duplicates += 1;
            return;
        }
        self.procs[i].work_ops = Some(w.ops);
    }

    fn apply_thread(&mut self, t: ThreadLine) {
        let Some(i) = self.proc_slot(&t.proc) else {
            return;
        };
        let threads = &mut self.procs[i].threads;
        if threads.iter().any(|x| x.thread == t.thread.thread) {
            self.health.duplicates += 1;
        } else if threads.len() >= MAX_THREADS_PER_PROC {
            self.health.over_limit += 1;
        } else {
            threads.push(t.thread);
        }
    }

    fn apply_end(&mut self, e: End) {
        let Some(i) = self.proc_slot(&e.proc) else {
            return;
        };
        if self.procs[i].end.is_some() {
            self.health.duplicates += 1;
            return;
        }
        self.procs[i].end = Some(EndReport {
            reason: e.reason,
            status: e.status,
            sites: e.sites,
            evals: e.evals,
            dropped: e.dropped,
            threads: e.threads,
        });
    }

    /// Register `id` as catalogued by `proc`. Returns false when the declaration was refused.
    fn declare(
        &mut self,
        proc: &str,
        id: &str,
        kind: AssertKind,
        msg: &str,
        loc: Option<&Loc>,
    ) -> bool {
        let key = (proc.to_owned(), id.to_owned());
        if let Some(existing) = self.props.get(id) {
            if existing.kind != kind {
                self.record_conflict(&key, existing.kind, kind);
                return false;
            }
        }
        if !self.declared.contains(&key) {
            let count = self.declared_per_proc.entry(proc.to_owned()).or_insert(0);
            if *count >= MAX_IDS_PER_PROC {
                self.health.over_limit += 1;
                return false;
            }
            *count += 1;
            self.declared.insert(key);
        }
        self.props
            .entry(id.to_owned())
            .or_insert_with(|| TrialProperty {
                id: id.to_owned(),
                kind,
                must_hit: kind.must_hit(),
                msg: msg.to_owned(),
                loc: loc.cloned(),
                hits: 0,
                trues: 0,
                falses: 0,
                first_false: None,
                first_true: None,
            });
        true
    }

    fn record_conflict(&mut self, key: &ProcId, kept: AssertKind, rejected: AssertKind) {
        if self.conflicted.insert(key.clone()) {
            self.health.conflicts += 1;
            self.conflicts.push(CatalogConflict {
                id: key.1.clone(),
                kept,
                rejected,
                proc: key.0.clone(),
            });
        }
    }

    fn apply_assert(&mut self, a: AssertReport) {
        let key = (a.proc.clone(), a.id.clone());
        if self.conflicted.contains(&key) {
            self.health.dropped_asserts += 1;
            return;
        }
        if !self.declared.contains(&key) {
            self.health.uncatalogued += 1;
            return;
        }
        if !self.asserted.insert(key) {
            // A second report for the same (proc, id): its counts are still evidence, so they are added.
            self.health.duplicates += 1;
        }
        let Some(p) = self.props.get_mut(&a.id) else {
            return;
        };
        p.hits = p.hits.saturating_add(a.hits);
        p.trues = p.trues.saturating_add(a.trues);
        p.falses = p.falses.saturating_add(a.falses);
        if p.first_false.is_none() {
            p.first_false = a.first_false.map(|w| located(&a.proc, w));
        }
        if p.first_true.is_none() {
            p.first_true = a.first_true.map(|w| located(&a.proc, w));
        }
    }

    /// Eval lines come from scripts that never send hello or end, so they do not create a process entry (which
    /// would then read as a lost report).
    fn apply_eval(&mut self, e: Eval) {
        if !self.declare(&e.proc, &e.id, e.kind, "", None) {
            return;
        }
        // Reachable and unreachable sites carry no condition: any evaluation is a hit.
        let ok = e.ok || !e.kind.has_condition();
        if let Some(p) = self.props.get_mut(&e.id) {
            p.hits = p.hits.saturating_add(1);
            if ok {
                p.trues = p.trues.saturating_add(1);
            } else {
                p.falses = p.falses.saturating_add(1);
            }
        }
    }
}

fn located(proc: &str, w: WireWitness) -> Witness {
    Witness {
        proc: proc.to_owned(),
        thread: w.thread,
        ord: w.ord,
        vals: w.vals,
    }
}

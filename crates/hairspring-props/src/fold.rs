//! Folding a trial's output lines into [`TrialProps`].

use crate::event::*;
use crate::limits::*;
use crate::line::{parse_line, IgnoreReason, LineOutcome, RejectReason};
use crate::trial::{
    CatalogConflict, EndReport, EventHealth, ProcReport, TrialProperty, TrialProps, Witness,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The lines of one trial's output (stdout and stderr, in any order the caller chooses).
#[derive(Debug, Clone, Default)]
pub struct TrialEvents<'a> {
    lines: Vec<&'a [u8]>,
}

impl<'a> TrialEvents<'a> {
    /// Split raw output on `\n`.
    pub fn from_output(output: &'a [u8]) -> Self {
        TrialEvents {
            lines: output.split(|b| *b == b'\n').collect(),
        }
    }

    pub fn from_lines<I: IntoIterator<Item = &'a [u8]>>(lines: I) -> Self {
        TrialEvents {
            lines: lines.into_iter().collect(),
        }
    }

    /// Fold every line; never panics, never fails. Anything dropped is counted in `health`.
    pub fn fold(&self) -> TrialProps {
        let mut acc = Accumulator::default();
        for line in &self.lines {
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
    perturbing: bool,
    health: EventHealth,
}

impl Accumulator {
    fn finish(self) -> TrialProps {
        TrialProps {
            procs: self.procs,
            properties: self.props.into_values().collect(),
            conflicts: self.conflicts,
            perturbing: self.perturbing,
            health: self.health,
        }
    }

    fn feed(&mut self, line: &[u8]) {
        let is_candidate = line.starts_with(EVENT_PREFIX_BARE.as_bytes());
        let cost = if is_candidate {
            (line.len().min(MAX_LINE_BYTES) + 1) as u64
        } else {
            0
        };
        if cost > 0
            && self.health.event_bytes.saturating_add(cost) > MAX_EVENT_BYTES_PER_TRIAL as u64
        {
            self.health.over_limit += 1;
            return;
        }
        match parse_line(line) {
            LineOutcome::NotEvent => {}
            LineOutcome::Rejected(reason) => {
                self.health.event_bytes += cost;
                self.count_rejection(reason);
            }
            LineOutcome::Ignored(reason) => {
                self.health.event_bytes += cost;
                match reason {
                    IgnoreReason::UnknownKind => self.health.unknown_kind += 1,
                    IgnoreReason::UnsupportedVersion => self.health.unsupported_version += 1,
                }
            }
            LineOutcome::Event {
                event,
                unknown_fields,
            } => {
                self.health.event_bytes += cost;
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
        });
        Some(self.procs.len() - 1)
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Hello(h) => self.apply_hello(h),
            Event::Work(w) => self.apply_work(w),
            Event::End(e) => self.apply_end(e),
            Event::Catalog(c) => {
                if self.proc_slot(&c.proc).is_some() {
                    self.declare(&c.proc, &c.id, c.kind, &c.msg, c.loc.as_ref());
                }
            }
            Event::Assert(a) => self.apply_assert(a),
            Event::Eval(e) => self.apply_eval(e),
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
            self.conflicts.push(CatalogConflict {
                id: key.1.clone(),
                kept,
                rejected,
                proc: key.0.clone(),
            });
        }
    }

    fn apply_assert(&mut self, a: AssertReport) {
        if self.proc_slot(&a.proc).is_none() {
            return;
        }
        let key = (a.proc.clone(), a.id.clone());
        if self.conflicted.contains(&key) {
            return;
        }
        if !self.declared.contains(&key) {
            self.health.uncatalogued += 1;
            return;
        }
        if !self.asserted.insert(key) {
            self.health.duplicates += 1;
            return;
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

    fn apply_eval(&mut self, e: Eval) {
        // Eval lines come from scripts that never send hello or end, so they do not create a process entry (which
        // would then read as a lost report).
        self.perturbing = true;
        if !self.declare(&e.proc, &e.id, e.kind, "", None) {
            return;
        }
        if let Some(p) = self.props.get_mut(&e.id) {
            p.hits = p.hits.saturating_add(1);
            if e.ok {
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

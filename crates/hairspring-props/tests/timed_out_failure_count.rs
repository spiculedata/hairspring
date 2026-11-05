//! The count of timed-out trials that showed a failure, checked three ways on random small runs: the fold, the
//! independent test oracle, and a brute-force reading of the definition (a trial counts when an id failed in it under
//! the kind the id had at that point and finally kept). Seeds may repeat: trials are told apart by their position.
mod common;

use common::legacy_run::LegacyRunProps;
use hairspring_props::limits::{MAX_IDS_PER_RUN, MAX_PROVISIONAL_REFS};
use hairspring_props::*;
use std::collections::BTreeMap;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, p: u64) -> bool {
        self.below(100) < p
    }
}

const K: [AssertKind; 4] = [
    AssertKind::Always,
    AssertKind::Unreachable,
    AssertKind::Sometimes,
    AssertKind::AlwaysOrUnreachable,
];

fn prop(rng: &mut Rng, id: String, kind: AssertKind) -> TrialProperty {
    let trues = rng.below(3);
    let falses = if rng.chance(50) { rng.below(2) + 1 } else { 0 };
    TrialProperty {
        id,
        kind,
        must_hit: kind.must_hit(),
        msg: "m".into(),
        loc: None,
        hits: trues + falses,
        trues,
        falses,
        first_false: None,
        first_true: None,
    }
}

fn trial(rng: &mut Rng, pool: u64, nkinds: u64) -> TrialProps {
    let mut properties = Vec::new();
    for i in 0..pool {
        if rng.chance(60) {
            let kind = K[rng.below(nkinds) as usize];
            properties.push(prop(rng, format!("id{i}"), kind));
        }
    }
    with_properties(properties)
}

fn with_properties(properties: Vec<TrialProperty>) -> TrialProps {
    TrialProps {
        procs: vec![ProcReport {
            tag: "p1".into(),
            sdk: None,
            sdk_version: None,
            catalog_hash: None,
            signal_handlers: None,
            work_ops: Some(1),
            end: Some(EndReport {
                reason: EndReason::Exit,
                status: 0,
                sites: 1,
                evals: 1,
                dropped: 0,
                threads: 0,
            }),
            threads: Vec::new(),
        }],
        properties,
        conflicts: Vec::new(),
        perturbing: false,
        health: EventHealth::default(),
    }
}

type Trials = Vec<(u64, TrialProps, bool)>;

fn run(seed: u64, dup_seeds: bool) -> (Trials, usize) {
    let mut rng = Rng(seed | 1);
    let n = 1 + rng.below(14) as usize;
    let pool = 1 + rng.below(5);
    let nkinds = 2 + rng.below(3);
    let cap = [1usize, 2, 3, MAX_IDS_PER_RUN][rng.below(4) as usize];
    let trials = (0..n)
        .map(|i| {
            let s = if dup_seeds {
                rng.below(3)
            } else {
                i as u64 * 3 + 100
            };
            (s, trial(&mut rng, pool, nkinds), rng.chance(55))
        })
        .collect();
    (trials, cap)
}

fn fold(t: &Trials, cap: usize) -> (RunSummary, Vec<PropertyStat>) {
    let mut r = RunProps::with_id_cap(cap);
    for (s, p, inc) in t {
        r = if *inc {
            r.add_inconclusive(*s, p)
        } else {
            r.add(*s, p)
        };
    }
    (r.summary(), r.finish())
}

fn legacy(t: &Trials, cap: usize) -> (RunSummary, Vec<PropertyStat>) {
    let mut r = LegacyRunProps::with_id_cap(cap);
    for (s, p, inc) in t {
        r = if *inc {
            r.add_inconclusive(*s, p)
        } else {
            r.add(*s, p)
        };
    }
    (r.summary(), r.finish())
}

/// B1: the literal definition. B2: plus the gate that the id's kind at that moment equalled the declared kind.
/// Also returns per-id counts under B2.
fn brute(t: &Trials, finals: &BTreeMap<String, AssertKind>) -> (u64, u64, BTreeMap<String, u64>) {
    let mut now: BTreeMap<String, (AssertKind, bool)> = BTreeMap::new();
    let (mut b1, mut b2) = (0u64, 0u64);
    let mut per = BTreeMap::new();
    for (_, p, inc) in t {
        let (mut hit1, mut hit2) = (false, false);
        for q in &p.properties {
            let Some(fk) = finals.get(&q.id) else {
                continue;
            };
            let e = now.entry(q.id.clone()).or_insert((q.kind, !*inc));
            if !*inc && !e.1 {
                *e = (q.kind, true);
            }
            if *inc && q.failure().is_some() && q.kind == *fk {
                hit1 = true;
                if e.0 == q.kind {
                    hit2 = true;
                    *per.entry(q.id.clone()).or_insert(0) += 1;
                }
            }
        }
        b1 += u64::from(hit1);
        b2 += u64::from(hit2);
    }
    (b1, b2, per)
}

#[test]
fn the_fold_the_oracle_and_the_definition_agree_on_random_runs() {
    for seed in 1..=20_000u64 {
        let (t, cap) = run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), false);
        let (fs, fstats) = fold(&t, cap);
        let (ls, lstats) = legacy(&t, cap);
        let finals: BTreeMap<String, AssertKind> =
            fstats.iter().map(|s| (s.id.clone(), s.kind)).collect();
        let (b1, b2, per) = brute(&t, &finals);
        assert_eq!(
            serde_json::to_string(&(&fs, &fstats)).unwrap(),
            serde_json::to_string(&(&ls, &lstats)).unwrap(),
            "fold vs oracle seed {seed}"
        );
        assert_eq!(
            fs.inconclusive_with_failure_trials, b2,
            "fold vs B2 seed {seed}"
        );
        for s in &fstats {
            assert_eq!(
                s.trials_inconclusive_with_failures,
                per.get(&s.id).copied().unwrap_or(0),
                "per-id seed {seed} id {}",
                s.id
            );
        }
        assert!(b1 >= b2);
    }
}

#[test]
fn trials_that_repeat_a_seed_are_counted_by_position_not_by_seed() {
    for seed in 1..=10_000u64 {
        let (t, cap) = run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15), true);
        let (fs, fstats) = fold(&t, cap);
        let (ls, _) = legacy(&t, cap);
        let finals: BTreeMap<String, AssertKind> =
            fstats.iter().map(|s| (s.id.clone(), s.kind)).collect();
        let (_, counted, _) = brute(&t, &finals);
        assert_eq!(
            fs.inconclusive_with_failure_trials, counted,
            "fold, seed {seed}"
        );
        assert_eq!(
            ls.inconclusive_with_failure_trials, counted,
            "oracle, seed {seed}"
        );
    }
}

fn failing_always(id: String) -> TrialProperty {
    TrialProperty {
        id,
        kind: AssertKind::Always,
        must_hit: true,
        msg: "m".into(),
        loc: None,
        hits: 1,
        trues: 0,
        falses: 1,
        first_false: None,
        first_true: None,
    }
}

fn many_failing_ids(count: usize) -> TrialProps {
    with_properties(
        (0..count)
            .map(|i| failing_always(format!("id{i}")))
            .collect(),
    )
}

#[test]
fn the_provisional_attributions_stay_within_the_limit_and_the_count_stays_a_lower_bound() {
    let ids = 50;
    let trials = MAX_PROVISIONAL_REFS / ids * 2;
    let timed_out = many_failing_ids(ids);
    let mut run = RunProps::new();
    for i in 0..trials {
        run = run.add_inconclusive(i as u64, &timed_out);
        assert!(run.provisional_refs() <= MAX_PROVISIONAL_REFS);
    }
    let held = run.provisional_refs();
    let counted = run.summary().inconclusive_with_failure_trials;
    assert!(
        held > MAX_PROVISIONAL_REFS - ids,
        "the limit was reached: {held}"
    );
    assert!(
        counted < trials as u64,
        "past the limit trials are left out: {counted}"
    );
    assert_eq!(counted as usize, MAX_PROVISIONAL_REFS / ids);
    // The ids keep their kind, so by the definition every one of these trials counts: the figure is a lower bound.
    run = settle_as_always(run, ids);
    assert_eq!(
        run.provisional_refs(),
        0,
        "settled ids release what they held"
    );
    assert!(run.summary().inconclusive_with_failure_trials <= trials as u64);
    assert_eq!(run.summary().inconclusive_with_failure_trials, counted);
}

/// Settles every id as `always` with a clean conclusive trial.
fn settle_as_always(run: RunProps, ids: usize) -> RunProps {
    let clean = with_properties(
        (0..ids)
            .map(|i| TrialProperty {
                falses: 0,
                hits: 1,
                trues: 1,
                ..failing_always(format!("id{i}"))
            })
            .collect(),
    );
    run.add(u64::MAX, &clean)
}

#[test]
fn a_trial_that_a_settled_id_also_attributes_is_never_held() {
    let settled = with_properties(vec![TrialProperty {
        falses: 0,
        hits: 1,
        trues: 1,
        ..failing_always("a".into())
    }]);
    let timed_out = with_properties(vec![failing_always("a".into()), failing_always("b".into())]);
    let run = RunProps::new()
        .add(1, &settled)
        .add_inconclusive(2, &timed_out);
    assert_eq!(run.provisional_refs(), 0);
    assert_eq!(run.summary().inconclusive_with_failure_trials, 1);
}

#[test]
fn the_provisional_limit_is_reached_exactly_and_the_trial_that_fills_it_is_counted() {
    // 64 undecided ids a trial divide the limit exactly, so the 512th trial brings the held total to the limit and
    // is still counted; only the 513th would pass it.
    let ids = 64;
    assert_eq!(MAX_PROVISIONAL_REFS % ids, 0);
    let fills = MAX_PROVISIONAL_REFS / ids;
    let timed_out = many_failing_ids(ids);
    let mut run = RunProps::new();
    for i in 0..=fills {
        run = run.add_inconclusive(i as u64, &timed_out);
    }
    assert_eq!(run.provisional_refs(), MAX_PROVISIONAL_REFS);
    assert_eq!(
        run.summary().inconclusive_with_failure_trials,
        fills as u64,
        "the trial that reaches the limit counts, the one past it does not"
    );
}

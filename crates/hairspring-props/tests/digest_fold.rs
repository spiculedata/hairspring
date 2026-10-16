//! The digest fold must produce exactly the statistics the previous fold produced, for every input, and a digest that
//! travelled through bytes and a separately rebuilt catalog must fold the same as one folded in process.
mod common;

use common::legacy_run::LegacyRunProps;
use hairspring_props::limits::MAX_IDS_PER_RUN;
use hairspring_props::*;

/// A small deterministic generator, so a failing case can be replayed from its seed.
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
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const KINDS: [AssertKind; 5] = AssertKind::ALL;

fn witness(rng: &mut Rng) -> Witness {
    Witness {
        proc: format!("p{}", rng.below(3) + 1),
        thread: format!("w-{}", rng.below(4)),
        ord: rng.below(1000),
        vals: [rng.below(9) as i64 - 4, 1, -2, rng.below(1 << 40) as i64],
    }
}

fn property(rng: &mut Rng, id: String, kind: AssertKind) -> TrialProperty {
    let trues = rng.below(4);
    let falses = if rng.chance(30) { rng.below(3) + 1 } else { 0 };
    let n = rng.below(5);
    TrialProperty {
        id,
        kind,
        must_hit: kind.must_hit(),
        msg: format!("message {n}"),
        loc: rng.chance(60).then(|| Loc {
            file: format!("src/f{n}.c"),
            line: n + 1,
            func: rng.chance(50).then(|| format!("fn{n}")),
        }),
        hits: trues + falses,
        trues,
        falses,
        first_false: (falses > 0).then(|| witness(rng)),
        first_true: (trues > 0).then(|| witness(rng)),
    }
}

fn proc_report(tag: &str, ended: bool) -> ProcReport {
    ProcReport {
        tag: tag.to_string(),
        sdk: None,
        sdk_version: None,
        catalog_hash: None,
        signal_handlers: None,
        work_ops: Some(3),
        end: ended.then_some(EndReport {
            reason: EndReason::Exit,
            status: 0,
            sites: 1,
            evals: 1,
            dropped: 0,
            threads: 0,
        }),
        threads: Vec::new(),
    }
}

fn natural_kind(id: &str) -> AssertKind {
    let last = u64::from(id.as_bytes()[id.len() - 1]);
    KINDS[(id.len() as u64 + last) as usize % 5]
}

fn random_trial(rng: &mut Rng, id_pool: u64) -> TrialProps {
    let mut ids: Vec<String> = Vec::new();
    for i in 0..id_pool {
        if rng.chance(55) {
            ids.push(format!("mod.id{i:02}"));
        }
    }
    ids.sort();
    let properties: Vec<TrialProperty> = ids
        .into_iter()
        .map(|id| {
            // Mostly the id's natural kind, sometimes another one, to exercise kind mismatches.
            let kind = if rng.chance(12) {
                KINDS[rng.below(5) as usize]
            } else {
                natural_kind(&id)
            };
            property(rng, id, kind)
        })
        .collect();
    let mut conflicts = Vec::new();
    for p in &properties {
        if rng.chance(6) {
            conflicts.push(CatalogConflict {
                dropped: DroppedCounts {
                    hits: 1,
                    trues: 1,
                    falses: 0,
                },
                id: p.id.clone(),
                kept: p.kind,
                rejected: KINDS[rng.below(5) as usize],
                proc: "p2".to_string(),
            });
        }
    }
    TrialProps {
        procs: vec![proc_report("p1", !rng.chance(10))],
        properties,
        conflicts,
        perturbing: rng.chance(10),
        health: EventHealth {
            malformed: u64::from(rng.chance(12)),
            ..EventHealth::default()
        },
    }
}

type Trials = Vec<(u64, TrialProps, bool)>;

fn random_run(seed: u64, trials: usize, id_pool: u64) -> Trials {
    let mut rng = Rng(seed | 1);
    (0..trials)
        .map(|i| {
            (
                (i as u64) * 3 + 100,
                random_trial(&mut rng, id_pool),
                rng.chance(15),
            )
        })
        .collect()
}

type Folded = (Vec<PropertyStat>, RunSummary);

fn legacy(trials: &Trials, cap: usize) -> Folded {
    let mut run = LegacyRunProps::with_id_cap(cap);
    for (seed, t, inconclusive) in trials {
        run = if *inconclusive {
            run.add_inconclusive(*seed, t)
        } else {
            run.add(*seed, t)
        };
    }
    let summary = run.summary();
    (run.finish(), summary)
}

fn current(trials: &Trials, cap: usize) -> Folded {
    let mut run = RunProps::with_id_cap(cap);
    for (seed, t, inconclusive) in trials {
        run = if *inconclusive {
            run.add_inconclusive(*seed, t)
        } else {
            run.add(*seed, t)
        };
    }
    let summary = run.summary();
    (run.finish(), summary)
}

/// What a server does: it receives encoded digests and catalog updates, rebuilds the catalog, and folds at the end.
fn via_bytes(trials: &Trials, cap: usize) -> Folded {
    let mut sender = RunCatalog::new();
    let mut wire = Vec::new();
    for (seed, t, inconclusive) in trials {
        let rec = sender.record(t, *inconclusive, cap);
        sender = rec.catalog;
        wire.push((*seed, rec.digest.encode(), rec.updates));
    }
    let mut catalog = RunCatalog::new();
    let mut digests = Vec::new();
    for (seed, bytes, updates) in wire {
        for u in &updates {
            catalog = catalog.apply(u, cap).expect("sender updates are valid");
        }
        digests.push((seed, TrialDigest::decode(&bytes).expect("round trip")));
    }
    let mut run = RunProps::with_id_cap(cap);
    for (seed, d) in &digests {
        run = run.add_digest(*seed, d, &catalog);
    }
    let summary = run.summary();
    (run.finish(), summary)
}

fn as_json(r: &Folded) -> String {
    serde_json::to_string(r).unwrap()
}

#[test]
fn the_digest_fold_is_byte_identical_to_the_previous_fold_on_random_runs() {
    for seed in 1..=300u64 {
        let trials = random_run(seed * 7919, 40, 10);
        let old = legacy(&trials, MAX_IDS_PER_RUN);
        let new = current(&trials, MAX_IDS_PER_RUN);
        assert_eq!(as_json(&old), as_json(&new), "seed {seed}");
    }
}

#[test]
fn the_digest_fold_matches_when_the_id_cap_drops_ids() {
    let mut dropped_somewhere = false;
    for seed in 1..=200u64 {
        let trials = random_run(seed * 104_729, 30, 12);
        let (old, new) = (legacy(&trials, 5), current(&trials, 5));
        assert_eq!(as_json(&old), as_json(&new), "seed {seed}");
        dropped_somewhere |= new.1.ids_dropped > 0;
    }
    assert!(dropped_somewhere, "the generator must exercise the cap");
}

#[test]
fn the_generator_exercises_mismatches_conflicts_and_inconclusive_trials() {
    let mut mismatches = 0;
    let mut conflicts = 0;
    let mut inconclusive = 0;
    for seed in 1..=50u64 {
        let (_, s) = current(&random_run(seed * 31, 40, 10), MAX_IDS_PER_RUN);
        mismatches += s.kind_mismatches;
        conflicts += s.conflict_trials;
        inconclusive += s.inconclusive_trials;
    }
    assert!(mismatches > 0 && conflicts > 0 && inconclusive > 0);
}

#[test]
fn a_digest_that_travelled_as_bytes_folds_like_one_folded_in_process() {
    for seed in 1..=200u64 {
        let trials = random_run(seed * 6151, 35, 10);
        for cap in [MAX_IDS_PER_RUN, 6] {
            let (direct, wire) = (current(&trials, cap), via_bytes(&trials, cap));
            assert_eq!(as_json(&direct), as_json(&wire), "seed {seed} cap {cap}");
        }
    }
}

fn goldens() -> Vec<(String, TrialProps)> {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdks/conformance/goldens");
    let mut found: Vec<_> = std::fs::read_dir(dir)
        .expect("goldens directory")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "events"))
        .collect();
    found.sort();
    found
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                TrialEvents::from_output(&bytes).fold(),
            )
        })
        .collect()
}

#[test]
fn every_conformance_golden_folds_identically_alone_and_repeated() {
    let found = goldens();
    assert!(found.len() >= 5, "the goldens must be found");
    for (name, props) in found {
        let trials: Trials = (0..4).map(|i| (i, props.clone(), i == 2)).collect();
        let old = legacy(&trials, MAX_IDS_PER_RUN);
        let new = current(&trials, MAX_IDS_PER_RUN);
        assert_eq!(as_json(&old), as_json(&new), "{name}");
        assert_eq!(
            as_json(&old),
            as_json(&via_bytes(&trials, MAX_IDS_PER_RUN)),
            "{name} via bytes"
        );
    }
}

#[test]
fn every_props_test_fixture_shape_folds_identically() {
    use common::*;
    let outputs = [
        single(AssertKind::Always, 5, 0),
        single(AssertKind::Always, 0, 2),
        single(AssertKind::Sometimes, 0, 4),
        single(AssertKind::Reachable, 0, 0),
        single(AssertKind::Unreachable, 1, 0),
        single(AssertKind::AlwaysOrUnreachable, 0, 3),
    ];
    let trials: Trials = outputs
        .iter()
        .enumerate()
        .map(|(i, o)| (i as u64, fold(o), i == 3))
        .collect();
    assert_eq!(
        as_json(&legacy(&trials, MAX_IDS_PER_RUN)),
        as_json(&current(&trials, MAX_IDS_PER_RUN))
    );
    assert_eq!(
        as_json(&legacy(&trials, MAX_IDS_PER_RUN)),
        as_json(&via_bytes(&trials, MAX_IDS_PER_RUN))
    );
}

//! Deterministic generators for the subtree fold tests: catalogs over one id universe, in different orders and with
//! different members, and digests built from logical trials so the same trial can be written against any catalog.

use hairspring_props::*;

/// A small deterministic generator, so a failing case can be replayed from its seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    pub fn shuffled<T: Clone>(&mut self, items: &[T]) -> Vec<T> {
        let mut out = items.to_vec();
        for i in (1..out.len()).rev() {
            out.swap(i, self.below(i as u64 + 1) as usize);
        }
        out
    }
}

/// Ids in the universe. Each id has one kind, message and location in every catalog.
pub const UNIVERSE: usize = 24;

pub fn id_name(n: usize) -> String {
    format!("prop{n:02}")
}

pub fn row(n: usize) -> CatalogRow {
    CatalogRow {
        id: id_name(n),
        kind: AssertKind::ALL[n % AssertKind::ALL.len()],
        msg: format!("message {n}"),
        loc: Some(Loc {
            file: format!("src/f{n}.c"),
            line: n as u64 + 1,
            func: None,
        }),
        decided: true,
    }
}

/// A catalog holding exactly these universe members, in this order.
pub fn catalog_of(members: &[usize]) -> RunCatalog {
    RunCatalog::from_rows(members.iter().map(|n| row(*n)).collect()).expect("valid rows")
}

/// A random subset of the universe in random order.
pub fn random_members(rng: &mut Rng, percent: u64) -> Vec<usize> {
    let all: Vec<usize> = (0..UNIVERSE).collect();
    let picked: Vec<usize> = all.into_iter().filter(|_| rng.chance(percent)).collect();
    rng.shuffled(&picked)
}

/// One id in one trial, by universe number.
#[derive(Debug, Clone)]
pub struct LogicalEntry {
    pub id: usize,
    pub trues: u64,
    pub falses: u64,
    pub conflicted: bool,
}

#[derive(Debug, Clone)]
pub struct LogicalTrial {
    pub flags: DigestFlags,
    pub ids_dropped: u64,
    pub entries: Vec<LogicalEntry>,
}

pub fn logical_trial(rng: &mut Rng, members: &[usize]) -> LogicalTrial {
    let mut entries = Vec::new();
    for id in members {
        if rng.chance(40) {
            entries.push(LogicalEntry {
                id: *id,
                trues: rng.below(4),
                falses: if rng.chance(25) { rng.below(3) + 1 } else { 0 },
                conflicted: rng.chance(4),
            });
        }
    }
    let flags = DigestFlags {
        inconclusive: rng.chance(10),
        degraded: rng.chance(10),
        perturbing: rng.chance(10),
        report_missing: rng.chance(10),
        has_conflicts: entries.iter().any(|e| e.conflicted),
        excluded: rng.chance(5),
    };
    LogicalTrial {
        flags,
        ids_dropped: if rng.chance(5) { rng.below(3) + 1 } else { 0 },
        entries,
    }
}

fn witness(n: u64) -> Witness {
    Witness {
        proc: "p1".to_owned(),
        thread: "w-0".to_owned(),
        ord: n,
        vals: [1, 2, 3, 4],
    }
}

/// The trial as a digest against `catalog`. Ids the catalog lacks are left out.
pub fn digest_in(trial: &LogicalTrial, catalog: &RunCatalog) -> TrialDigest {
    let entries = trial
        .entries
        .iter()
        .filter_map(|e| {
            let idx = catalog.index_of(&id_name(e.id))?;
            let kind = row(e.id).kind;
            let hits = e.trues + e.falses;
            let fails = failure_basis(kind, hits, e.falses).is_some();
            Some(DigestEntry {
                idx,
                kind,
                catalogued: true,
                conflicted: e.conflicted,
                hits,
                trues: e.trues,
                falses: e.falses,
                witness: fails.then(|| witness(e.id as u64)),
                decl: None,
            })
        })
        .collect();
    TrialDigest {
        flags: trial.flags,
        ids_dropped: trial.ids_dropped,
        entries,
    }
}

/// The index map `TrialDigest::remap` takes: run catalog index to `tree` index, `UNMAPPED_IDX` for ids the tree lacks.
pub fn map_into(run: &RunCatalog, tree: &RunCatalog) -> Vec<u32> {
    run.rows()
        .iter()
        .map(|r| tree.index_of(&r.id).unwrap_or(UNMAPPED_IDX))
        .collect()
}

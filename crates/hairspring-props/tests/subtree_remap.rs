//! `TrialDigest::remap` moves a digest from one run catalog to another by index, losing nothing without saying so.
mod common;

use common::subtree_gen::*;
use hairspring_props::*;

fn entry(idx: u32, trues: u64) -> DigestEntry {
    DigestEntry {
        idx,
        kind: AssertKind::Sometimes,
        catalogued: true,
        conflicted: false,
        hits: trues,
        trues,
        falses: 0,
        witness: None,
        decl: None,
    }
}

fn digest(entries: Vec<DigestEntry>) -> TrialDigest {
    TrialDigest {
        entries,
        ..TrialDigest::default()
    }
}

#[test]
fn remapping_then_folding_equals_folding_against_one_catalog() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed * 6_700_417);
        let tree = catalog_of(&rng.shuffled(&(0..UNIVERSE).collect::<Vec<_>>()));
        let runs: Vec<(RunCatalog, Vec<u32>)> = (0..3)
            .map(|_| {
                let run = catalog_of(&random_members(&mut rng, 60));
                let map = map_into(&run, &tree);
                (run, map)
            })
            .collect();

        let mut via_remap = RunProps::new();
        let mut direct = RunProps::new();
        for n in 0..60u64 {
            let (run, map) = &runs[rng.below(3) as usize];
            let members: Vec<usize> = run
                .rows()
                .iter()
                .map(|r| r.id[4..].parse().expect("universe id"))
                .collect();
            let trial = logical_trial(&mut rng, &members);
            via_remap = via_remap.add_digest(n, &digest_in(&trial, run).remap(map), &tree);
            direct = direct.add_digest(n, &digest_in(&trial, &tree), &tree);
        }
        assert_eq!(via_remap.summary(), direct.summary(), "seed {seed}");
        assert_eq!(via_remap.finish(), direct.finish(), "seed {seed}");
    }
}

#[test]
fn identity_map_and_round_trip_through_a_permutation_change_nothing() {
    for seed in 1..=30u64 {
        let mut rng = Rng(seed * 2_147_483_629);
        let members: Vec<usize> = (0..UNIVERSE).collect();
        let a = catalog_of(&members);
        let b = catalog_of(&rng.shuffled(&members));
        let there = map_into(&a, &b);
        let back = map_into(&b, &a);
        let identity: Vec<u32> = (0..UNIVERSE as u32).collect();
        for _ in 0..10 {
            let d = digest_in(&logical_trial(&mut rng, &members), &a);
            assert_eq!(d.remap(&identity), d, "seed {seed}: identity");
            assert_eq!(d.remap(&there).remap(&back), d, "seed {seed}: round trip");
            assert_eq!(d.remap(&there).entries.len(), d.entries.len());
        }
    }
}

#[test]
fn ids_missing_from_the_tree_are_dropped_and_counted_never_silent() {
    let d = TrialDigest {
        ids_dropped: 1,
        ..digest(vec![entry(0, 1), entry(1, 2), entry(2, 3)])
    };
    let out = d.remap(&[5, UNMAPPED_IDX, 7]);
    let kept: Vec<u32> = out.entries.iter().map(|e| e.idx).collect();
    assert_eq!(
        kept,
        vec![5, 7],
        "order is kept, the unmapped entry is gone"
    );
    assert_eq!(out.ids_dropped, 2, "the loss is added to ids_dropped");
    assert_eq!(out.entries[1].trues, 3, "kept entries are untouched");
}

#[test]
fn indexes_past_the_map_are_dropped_and_counted() {
    let out = digest(vec![entry(0, 1), entry(3, 1)]).remap(&[9, 8]);
    assert_eq!(out.entries.len(), 1);
    assert_eq!(out.ids_dropped, 1);
}

#[test]
fn two_entries_mapped_to_one_index_keep_the_first_and_count_the_second() {
    let out = digest(vec![entry(0, 1), entry(1, 2)]).remap(&[4, 4]);
    assert_eq!(out.entries.len(), 1, "indexes stay unique");
    assert_eq!(out.entries[0].trues, 1, "the first entry wins");
    assert_eq!(out.ids_dropped, 1);
}

#[test]
fn flags_witnesses_and_declarations_survive_and_an_empty_digest_is_empty() {
    let mut e = entry(0, 0);
    e.kind = AssertKind::Always;
    e.hits = 2;
    e.falses = 2;
    e.witness = Some(Witness {
        proc: "p1".to_owned(),
        thread: "w".to_owned(),
        ord: 3,
        vals: [1, 2, 3, 4],
    });
    e.decl = Some(Decl {
        msg: "m".to_owned(),
        loc: None,
    });
    let d = TrialDigest {
        flags: DigestFlags {
            degraded: true,
            perturbing: true,
            ..DigestFlags::default()
        },
        ..digest(vec![e.clone()])
    };
    let out = d.remap(&[6]);
    assert_eq!(out.flags, d.flags);
    assert_eq!(out.entries, vec![DigestEntry { idx: 6, ..e }]);

    assert_eq!(TrialDigest::default().remap(&[]), TrialDigest::default());
    assert_eq!(
        d.remap(&[]).ids_dropped,
        1,
        "an empty map drops everything and says so"
    );
}

#[test]
fn a_remapped_digest_still_encodes_and_decodes() {
    let d = digest(vec![entry(0, 1), entry(1, 2)]).remap(&[1, 0]);
    assert_eq!(TrialDigest::decode(&d.encode()), Ok(d));
}

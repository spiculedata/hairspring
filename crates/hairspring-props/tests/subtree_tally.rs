//! `NodeTally::merge` must give the same tally for every split, order and bracketing of the same trials, must never
//! hide that its failing-id set is cut or incomplete, and must not wrap its counters. Verdicts come from the store's
//! verdict column, never from the digest.
mod common;

use common::subtree_gen::*;
use hairspring_props::*;
use std::collections::BTreeSet;

const VERDICTS: [&str; 6] = [
    "pass",
    "violation",
    "inconclusive",
    "unusable",
    "infra",
    "novel",
];

type Row = (String, Option<TrialDigest>);

/// A catalog of `n` always-rules, so any index below `n` can fail.
fn wide_catalog(n: usize) -> RunCatalog {
    let rows = (0..n)
        .map(|i| CatalogRow {
            id: format!("wide{i}"),
            kind: AssertKind::Always,
            msg: String::new(),
            loc: None,
            decided: true,
        })
        .collect();
    RunCatalog::from_rows(rows).expect("valid rows")
}

fn tally_of(rows: &[Row], catalog: &RunCatalog) -> NodeTally {
    rows.iter().fold(NodeTally::empty(), |t, (v, d)| {
        t.merge(&NodeTally::of_trial(v, d.as_ref(), catalog))
    })
}

fn merge_all(tallies: &[NodeTally]) -> NodeTally {
    tallies
        .iter()
        .fold(NodeTally::empty(), |acc, t| acc.merge(t))
}

fn random_rows(rng: &mut Rng, count: usize) -> Vec<Row> {
    let members: Vec<usize> = (0..UNIVERSE).collect();
    let catalog = catalog_of(&members);
    (0..count)
        .map(|_| {
            let verdict = VERDICTS[rng.below(VERDICTS.len() as u64) as usize].to_owned();
            let digest = rng
                .chance(70)
                .then(|| digest_in(&logical_trial(rng, &members), &catalog));
            (verdict, digest)
        })
        .collect()
}

/// A digest whose entries all fail, at these catalog indexes.
fn failing_digest(idxs: &[u32]) -> TrialDigest {
    TrialDigest {
        entries: idxs
            .iter()
            .map(|idx| DigestEntry {
                idx: *idx,
                kind: AssertKind::Always,
                catalogued: true,
                conflicted: false,
                hits: 1,
                trues: 0,
                falses: 1,
                witness: None,
                decl: None,
            })
            .collect(),
        ..TrialDigest::default()
    }
}

fn violation_with(idxs: &[u32], catalog: &RunCatalog) -> NodeTally {
    NodeTally::of_trial("violation", Some(&failing_digest(idxs)), catalog)
}

fn ids(from: u32, to: u32) -> Vec<u32> {
    (from..to).collect()
}

fn universe_catalog() -> RunCatalog {
    catalog_of(&(0..UNIVERSE).collect::<Vec<_>>())
}

#[test]
fn merge_is_the_same_for_every_random_split_and_order() {
    let catalog = universe_catalog();
    for seed in 1..=60u64 {
        let mut rng = Rng(seed * 7919);
        let count = 1 + rng.below(40) as usize;
        let rows = random_rows(&mut rng, count);
        let whole = tally_of(&rows, &catalog);
        let unknown_rows = rows.iter().filter(|(v, _)| v == "novel").count() as u64;

        let groups = 1 + rng.below(6) as usize;
        let mut buckets: Vec<Vec<Row>> = vec![Vec::new(); groups];
        for r in rows {
            buckets[rng.below(groups as u64) as usize].push(r);
        }
        let parts: Vec<NodeTally> = buckets.iter().map(|b| tally_of(b, &catalog)).collect();

        assert_eq!(merge_all(&parts), whole, "seed {seed}: in order");
        assert_eq!(
            merge_all(&rng.shuffled(&parts)),
            whole,
            "seed {seed}: shuffled"
        );
        let right_nested = parts
            .iter()
            .rev()
            .fold(NodeTally::empty(), |acc, t| t.merge(&acc));
        assert_eq!(right_nested, whole, "seed {seed}: bracketed from the right");
        assert_eq!(
            merge_all(&parts).verdicts().unknown,
            unknown_rows,
            "seed {seed}: the unknown bucket survives every split"
        );
    }
}

#[test]
fn merge_is_associative_commutative_and_has_an_identity_over_mixed_sources() {
    let catalog = universe_catalog();
    for seed in 1..=60u64 {
        let mut rng = Rng(seed * 104_729);
        // A third of the parts come from a count query alone: violations with no ids behind them.
        let part = |rng: &mut Rng| {
            let count = 1 + rng.below(8) as usize;
            let rows = random_rows(rng, count);
            if rng.chance(33) {
                NodeTally::from_verdicts(tally_of(&rows, &catalog).verdicts().to_owned())
            } else {
                tally_of(&rows, &catalog)
            }
        };
        let (a, b, c) = (part(&mut rng), part(&mut rng), part(&mut rng));
        assert_eq!(a.merge(&b), b.merge(&a), "seed {seed}: commutative");
        assert_eq!(
            a.merge(&b).merge(&c),
            a.merge(&b.merge(&c)),
            "seed {seed}: associative"
        );
        assert_eq!(a.merge(&NodeTally::empty()), a, "seed {seed}: identity");
        assert_eq!(NodeTally::empty().merge(&a), a, "seed {seed}: identity");
    }
}

#[test]
fn capped_merge_is_associative_and_commutative_and_flags_the_cut() {
    let catalog = wide_catalog(600);
    for seed in 1..=60u64 {
        let mut rng = Rng(seed * 15_485_863);
        let pool = 1 + rng.below(600) as u32;
        let digests: Vec<TrialDigest> = (0..2 + rng.below(8))
            .map(|_| {
                let count = rng.below(300);
                let picked: BTreeSet<u32> = (0..count)
                    .map(|_| rng.below(u64::from(pool)) as u32)
                    .collect();
                failing_digest(&picked.into_iter().collect::<Vec<_>>())
            })
            .collect();
        let all_ids: BTreeSet<u32> = digests
            .iter()
            .flat_map(|d| d.entries.iter().map(|e| e.idx))
            .collect();

        let parts: Vec<NodeTally> = digests
            .iter()
            .map(|d| NodeTally::of_trial("violation", Some(d), &catalog))
            .collect();
        let whole = merge_all(&parts);
        assert_eq!(merge_all(&rng.shuffled(&parts)), whole, "seed {seed}");

        let expected: BTreeSet<u32> = all_ids
            .iter()
            .copied()
            .take(MAX_TALLY_FAILING_IDS)
            .collect();
        assert_eq!(
            whole.failing_ids(),
            &expected,
            "seed {seed}: lowest ids kept"
        );
        assert_eq!(
            whole.failing_ids_truncated(),
            all_ids.len() > MAX_TALLY_FAILING_IDS,
            "seed {seed}: flagged exactly when ids were lost"
        );
        assert_eq!(
            whole.verdicts().violation,
            parts.len() as u64,
            "seed {seed}: counts untouched"
        );
    }
}

#[test]
fn cap_boundary_is_exact() {
    let catalog = wide_catalog(MAX_TALLY_FAILING_IDS + 2);
    let cap = MAX_TALLY_FAILING_IDS as u32;
    let at_cap = violation_with(&ids(0, cap), &catalog);
    assert_eq!(at_cap.failing_ids().len(), MAX_TALLY_FAILING_IDS);
    assert!(
        !at_cap.failing_ids_truncated(),
        "exactly the cap is not a cut"
    );

    let over = violation_with(&ids(0, cap + 1), &catalog);
    assert_eq!(over.failing_ids().len(), MAX_TALLY_FAILING_IDS);
    assert!(over.failing_ids_truncated(), "one past the cap is a cut");
    assert!(
        !over.failing_ids().contains(&cap),
        "the highest id is the one dropped"
    );
    assert_eq!(over.verdicts().violation, 1);

    let same_again = at_cap.merge(&at_cap);
    assert!(
        !same_again.failing_ids_truncated(),
        "the same ids twice stay within the cap"
    );
    assert_eq!(same_again.failing_ids(), at_cap.failing_ids());

    let one_more = at_cap.merge(&violation_with(&[cap + 1], &catalog));
    assert!(
        one_more.failing_ids_truncated(),
        "a new id past a full set is a cut"
    );
    assert!(!one_more.failing_ids().contains(&(cap + 1)));

    assert!(
        over.merge(&NodeTally::empty()).failing_ids_truncated(),
        "the flag survives a merge with nothing"
    );
    assert!(NodeTally::empty().merge(&over).failing_ids_truncated());
}

#[test]
fn counts_saturate_at_the_u64_boundary_and_stay_associative() {
    let near = NodeTally::from_verdicts(VerdictCounts {
        pass: u64::MAX - 1,
        ..VerdictCounts::default()
    });
    let one = NodeTally::from_verdicts(VerdictCounts::default().with_named("pass", 1));

    assert_eq!(
        near.merge(&one).verdicts().pass,
        u64::MAX,
        "MAX-1 plus 1 reaches MAX exactly"
    );
    assert_eq!(
        near.merge(&one).merge(&one).verdicts().pass,
        u64::MAX,
        "past MAX stays at MAX"
    );
    assert_eq!(
        near.merge(&one.merge(&one)),
        near.merge(&one).merge(&one),
        "saturation keeps associativity"
    );
    assert_eq!(near.merge(&one), one.merge(&near));
    assert_eq!(
        VerdictCounts::default()
            .with_named("pass", u64::MAX)
            .with_named("pass", 9)
            .pass,
        u64::MAX
    );
    let full = VerdictCounts {
        pass: u64::MAX,
        violation: 1,
        ..VerdictCounts::default()
    };
    assert_eq!(full.total(), u64::MAX, "the total saturates too");
}

#[test]
fn query_counts_map_to_named_verdicts_and_unknown_names_stay_apart() {
    let counts = [
        ("pass", 5),
        ("violation", 2),
        ("inconclusive", 1),
        ("unusable", 3),
        ("infra", 4),
        ("novel", 7),
    ]
    .into_iter()
    .fold(VerdictCounts::default(), |c, (name, n)| {
        c.with_named(name, n)
    });
    assert_eq!(
        (counts.pass, counts.violation, counts.inconclusive),
        (5, 2, 1)
    );
    assert_eq!((counts.unusable, counts.infra, counts.unknown), (3, 4, 7));
    assert_eq!(NodeTally::from_verdicts(counts).verdicts().total(), 22);
}

#[test]
fn of_trial_counts_equal_from_verdicts_for_every_verdict_with_or_without_a_digest() {
    let catalog = universe_catalog();
    for seed in 1..=40u64 {
        let mut rng = Rng(seed * 6_700_417);
        let count = rng.below(50) as usize;
        let rows = random_rows(&mut rng, count);
        let counts = rows
            .iter()
            .fold(VerdictCounts::default(), |c, (v, _)| c.with_named(v, 1));
        assert_eq!(
            tally_of(&rows, &catalog).verdicts(),
            NodeTally::from_verdicts(counts).verdicts(),
            "seed {seed}"
        );
    }
    for name in VERDICTS {
        for digest in [
            None,
            Some(TrialDigest::default()),
            Some(failing_digest(&[0, 1])),
        ] {
            let t = NodeTally::of_trial(name, digest.as_ref(), &catalog);
            let expected = VerdictCounts::default().with_named(name, 1);
            assert_eq!(
                t.verdicts(),
                &expected,
                "{name}: the store's verdict decides the count"
            );
        }
    }
}

#[test]
fn a_violation_without_a_failing_property_is_still_a_violation() {
    let catalog = universe_catalog();
    // An exit-code or hang violation: the digest has no failing entry. A verdict-only digest has no entries at all.
    for digest in [
        TrialDigest::default(),
        digest_in(
            &LogicalTrial {
                flags: DigestFlags::default(),
                ids_dropped: 0,
                entries: Vec::new(),
            },
            &catalog,
        ),
    ] {
        let t = NodeTally::of_trial("violation", Some(&digest), &catalog);
        assert_eq!((t.verdicts().violation, t.verdicts().pass), (1, 0));
        assert!(t.failing_ids().is_empty());
        assert!(
            t.failing_ids_exhaustive(),
            "the digest says no property failed, so no ids are missing"
        );
    }
    // A passing verdict is not turned into a violation by a failing entry in the digest.
    let t = NodeTally::of_trial("pass", Some(&failing_digest(&[0])), &catalog);
    assert_eq!((t.verdicts().pass, t.verdicts().violation), (1, 0));
}

#[test]
fn a_violation_with_no_digest_does_not_claim_its_ids_are_known() {
    let catalog = wide_catalog(UNIVERSE);
    let none = NodeTally::of_trial("violation", None, &catalog);
    assert!(!none.failing_ids_exhaustive());
    assert!(NodeTally::of_trial("pass", None, &catalog).failing_ids_exhaustive());

    let five = NodeTally::from_verdicts(VerdictCounts::default().with_named("violation", 5));
    assert!(
        !five.failing_ids_exhaustive(),
        "violations from a count query have no ids"
    );
    let mixed = five.merge(&violation_with(&[1], &catalog));
    assert_eq!(mixed.verdicts().violation, 6);
    assert_eq!(
        mixed.failing_ids().iter().copied().collect::<Vec<_>>(),
        vec![1]
    );
    assert!(
        !mixed.failing_ids_exhaustive(),
        "6 violations, one known id: not complete"
    );
    assert!(
        !violation_with(&[1], &catalog)
            .merge(&five)
            .failing_ids_exhaustive(),
        "either order"
    );
    assert!(!five.merge(&NodeTally::empty()).failing_ids_exhaustive());

    let zero = NodeTally::from_verdicts(VerdictCounts::default().with_named("pass", 3));
    assert!(
        zero.failing_ids_exhaustive(),
        "no violations, nothing missing"
    );
    assert!(NodeTally::empty().failing_ids_exhaustive());
}

#[test]
fn flags_follow_the_run_fold_rules() {
    let catalog = universe_catalog();
    let flags = |f: DigestFlags| TrialDigest {
        flags: f,
        ..failing_digest(&[1])
    };

    let excluded = flags(DigestFlags {
        excluded: true,
        perturbing: true,
        ..DigestFlags::default()
    });
    let t = NodeTally::of_trial("unusable", Some(&excluded), &catalog);
    assert_eq!(
        (t.excluded(), t.degraded(), t.perturbing()),
        (1, 0, 0),
        "excluded trials add to nothing else"
    );
    assert!(t.failing_ids().is_empty());
    assert_eq!(t.verdicts().unusable, 1);

    let timed_out = flags(DigestFlags {
        inconclusive: true,
        perturbing: true,
        degraded: true,
        ..DigestFlags::default()
    });
    let t = NodeTally::of_trial("inconclusive", Some(&timed_out), &catalog);
    assert_eq!(
        (t.degraded(), t.perturbing()),
        (1, 1),
        "a degraded timed-out trial is still degraded"
    );
    assert!(
        t.failing_ids().is_empty(),
        "a timed-out trial's failure is not a finding"
    );

    let lossy = TrialDigest {
        ids_dropped: 2,
        ..TrialDigest::default()
    };
    assert_eq!(
        NodeTally::of_trial("pass", Some(&lossy), &catalog).degraded(),
        1
    );
    assert_eq!(tally_of(&[], &catalog), NodeTally::empty());
}

#[test]
fn kind_mismatches_and_conflict_only_entries_are_judged_as_the_run_fold_judges_them() {
    let sometimes = |id: &str| CatalogRow {
        id: id.to_owned(),
        kind: AssertKind::Sometimes,
        msg: String::new(),
        loc: None,
        decided: true,
    };
    let catalog = RunCatalog::from_rows(vec![sometimes("a"), sometimes("b")]).expect("rows");
    let own_kind = |idx: u32| catalog.get(idx).expect("row").kind;
    let entry =
        |idx: u32, kind: AssertKind, falses: u64, catalogued: bool, conflicted: bool| DigestEntry {
            idx,
            kind,
            catalogued,
            conflicted,
            hits: falses,
            trues: 0,
            falses,
            witness: None,
            decl: None,
        };
    let one = |e: DigestEntry| TrialDigest {
        entries: vec![e],
        ..TrialDigest::default()
    };

    // Under its own declared kind (Always) this entry fails, but the catalog says Sometimes: a mismatch, which the
    // run fold does not merge.
    let t = NodeTally::of_trial(
        "pass",
        Some(&one(entry(0, AssertKind::Always, 1, true, false))),
        &catalog,
    );
    assert!(
        t.failing_ids().is_empty(),
        "a mismatched entry's own kind does not decide"
    );

    // A conflict on a catalogued entry is a failure; a conflict-only entry may be one, so ids are not exhaustive.
    let t = NodeTally::of_trial(
        "violation",
        Some(&one(entry(1, own_kind(1), 0, true, true))),
        &catalog,
    );
    assert_eq!(t.failing_ids().iter().copied().collect::<Vec<_>>(), vec![1]);
    let t = NodeTally::of_trial(
        "violation",
        Some(&one(entry(1, own_kind(1), 0, false, true))),
        &catalog,
    );
    assert!(t.failing_ids().is_empty());
    assert!(
        !t.failing_ids_exhaustive(),
        "a conflict-only entry may fail an id this trial cannot see"
    );
}

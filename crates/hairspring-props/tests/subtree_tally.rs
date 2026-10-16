//! `NodeTally::merge` must give the same tally for every split, order and bracketing of the same trials, must never
//! hide that its failing-id set was cut, and must not wrap its counters.
mod common;

use common::subtree_gen::*;
use hairspring_props::*;
use std::collections::BTreeSet;

fn tally_of(digests: &[TrialDigest]) -> NodeTally {
    digests
        .iter()
        .fold(NodeTally::empty(), |t, d| t.merge(&NodeTally::of_digest(d)))
}

fn merge_all(tallies: &[NodeTally]) -> NodeTally {
    tallies
        .iter()
        .fold(NodeTally::empty(), |acc, t| acc.merge(t))
}

fn random_digests(rng: &mut Rng, count: usize) -> Vec<TrialDigest> {
    let members: Vec<usize> = (0..UNIVERSE).collect();
    let catalog = catalog_of(&members);
    (0..count)
        .map(|_| digest_in(&logical_trial(rng, &members), &catalog))
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

fn ids(from: u32, to: u32) -> Vec<u32> {
    (from..to).collect()
}

#[test]
fn merge_is_the_same_for_every_random_split_and_order() {
    for seed in 1..=60u64 {
        let mut rng = Rng(seed * 7919);
        let count = 1 + rng.below(40) as usize;
        let digests = random_digests(&mut rng, count);
        let whole = tally_of(&digests);

        let groups = 1 + rng.below(6) as usize;
        let mut buckets: Vec<Vec<TrialDigest>> = vec![Vec::new(); groups];
        for d in digests {
            buckets[rng.below(groups as u64) as usize].push(d);
        }
        let parts: Vec<NodeTally> = buckets.iter().map(|b| tally_of(b)).collect();

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
    }
}

#[test]
fn merge_is_associative_commutative_and_has_an_identity() {
    for seed in 1..=40u64 {
        let mut rng = Rng(seed * 104_729);
        let part = |rng: &mut Rng| {
            let count = 1 + rng.below(8) as usize;
            tally_of(&random_digests(rng, count))
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

        let parts: Vec<NodeTally> = digests.iter().map(NodeTally::of_digest).collect();
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
        let failing_digests = digests.iter().filter(|d| !d.entries.is_empty()).count() as u64;
        assert_eq!(
            whole.verdicts().violation,
            failing_digests,
            "seed {seed}: counts untouched"
        );
        assert_eq!(
            whole.verdicts().total(),
            digests.len() as u64,
            "seed {seed}: no trial lost"
        );
    }
}

#[test]
fn cap_boundary_is_exact() {
    let at_cap = NodeTally::of_digest(&failing_digest(&ids(0, MAX_TALLY_FAILING_IDS as u32)));
    assert_eq!(at_cap.failing_ids().len(), MAX_TALLY_FAILING_IDS);
    assert!(
        !at_cap.failing_ids_truncated(),
        "exactly the cap is not a cut"
    );

    let over = NodeTally::of_digest(&failing_digest(&ids(0, MAX_TALLY_FAILING_IDS as u32 + 1)));
    assert_eq!(over.failing_ids().len(), MAX_TALLY_FAILING_IDS);
    assert!(over.failing_ids_truncated(), "one past the cap is a cut");
    assert!(
        !over.failing_ids().contains(&(MAX_TALLY_FAILING_IDS as u32)),
        "the highest id is the one dropped"
    );
    assert_eq!(over.verdicts().violation, 1);

    let same_again = at_cap.merge(&at_cap);
    assert!(
        !same_again.failing_ids_truncated(),
        "the same ids twice stay within the cap"
    );
    assert_eq!(same_again.failing_ids(), at_cap.failing_ids());

    let one_more = at_cap.merge(&NodeTally::of_digest(&failing_digest(&[10_000])));
    assert!(
        one_more.failing_ids_truncated(),
        "a new id past a full set is a cut"
    );
    assert!(!one_more.failing_ids().contains(&10_000));

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
        "saturation does not break associativity"
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
    let tally = NodeTally::from_verdicts(counts);
    assert!(tally.failing_ids().is_empty() && !tally.failing_ids_truncated());
    assert_eq!(tally.verdicts().total(), 22);
}

#[test]
fn one_digest_is_classified_as_the_run_fold_would() {
    let clean = TrialDigest::default();
    assert_eq!(NodeTally::of_digest(&clean).verdicts().pass, 1);

    let excluded = TrialDigest {
        flags: DigestFlags {
            excluded: true,
            ..DigestFlags::default()
        },
        ..failing_digest(&[1])
    };
    let t = NodeTally::of_digest(&excluded);
    assert_eq!((t.excluded(), t.verdicts().total()), (1, 0));
    assert!(t.failing_ids().is_empty());

    let timed_out = TrialDigest {
        flags: DigestFlags {
            inconclusive: true,
            perturbing: true,
            ..DigestFlags::default()
        },
        ..failing_digest(&[1])
    };
    let t = NodeTally::of_digest(&timed_out);
    assert_eq!(
        (
            t.verdicts().inconclusive,
            t.verdicts().violation,
            t.perturbing()
        ),
        (1, 0, 1)
    );
    assert!(
        t.failing_ids().is_empty(),
        "a timed-out trial's failure is not a finding"
    );

    let lossy = TrialDigest {
        ids_dropped: 2,
        ..TrialDigest::default()
    };
    assert_eq!(NodeTally::of_digest(&lossy).degraded(), 1);

    assert_eq!(tally_of(&[]), NodeTally::empty());
}

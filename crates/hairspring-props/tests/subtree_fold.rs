//! `fold_subtree` must agree with `RunProps` over the same digests on every statistic they share, carry no rate, and
//! stop at its digest cap.
mod common;

use common::subtree_gen::*;
use hairspring_props::*;
use std::cell::Cell;

fn tree_digests(rng: &mut Rng, count: usize) -> (RunCatalog, Vec<(u64, TrialDigest)>) {
    let members: Vec<usize> = rng.shuffled(&(0..UNIVERSE).collect::<Vec<_>>());
    let catalog = catalog_of(&members);
    let digests = (0..count as u64)
        .map(|seed| {
            (
                seed * 3 + 1,
                digest_in(&logical_trial(rng, &members), &catalog),
            )
        })
        .collect();
    (catalog, digests)
}

fn run_props(catalog: &RunCatalog, digests: &[(u64, TrialDigest)]) -> RunProps {
    digests.iter().fold(RunProps::new(), |run, (seed, d)| {
        run.add_digest(*seed, d, catalog)
    })
}

fn expected_stat(s: PropertyStat) -> SubtreeStat {
    SubtreeStat {
        id: s.id,
        kind: s.kind,
        must_hit: s.must_hit,
        msg: s.msg,
        loc: s.loc,
        trials: s.trials,
        trials_rated: s.trials_rated,
        trials_unreported: s.trials_unreported,
        trials_degraded: s.trials_degraded,
        trials_perturbing: s.trials_perturbing,
        trials_conflicted: s.trials_conflicted,
        trials_inconclusive_with_failures: s.trials_inconclusive_with_failures,
        trials_claimable: s.trials_claimable,
        trials_hit: s.trials_hit,
        trials_true: s.trials_true,
        trials_false: s.trials_false,
        evals: s.evals,
        evals_true: s.evals_true,
        evals_false: s.evals_false,
        violated: s.status == PropertyStatus::Violation,
        first_failure: s.first_failure,
    }
}

fn empty_digest() -> TrialDigest {
    TrialDigest::default()
}

#[test]
fn fold_subtree_equals_run_props_over_the_same_digests() {
    for seed in 1..=60u64 {
        let mut rng = Rng(seed * 982_451_653);
        let count = rng.below(70) as usize;
        let (catalog, digests) = tree_digests(&mut rng, count);
        let oracle = run_props(&catalog, &digests);
        let summary = oracle.summary();
        let expected: Vec<SubtreeStat> = oracle.finish().into_iter().map(expected_stat).collect();

        let fold =
            fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("under the cap");
        assert_eq!(fold.summary, summary, "seed {seed}");
        assert_eq!(fold.stats, expected, "seed {seed}");
    }
}

#[test]
fn the_tally_agrees_with_the_run_fold_on_what_they_share() {
    for seed in 1..=60u64 {
        let mut rng = Rng(seed * 49_979_687);
        let count = 1 + rng.below(70) as usize;
        let (catalog, digests) = tree_digests(&mut rng, count);
        let fold =
            fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("under the cap");
        let summary = fold.summary;
        let tally = fold.tally;

        let excluded = digests.iter().filter(|(_, d)| d.flags.excluded).count() as u64;
        assert_eq!(tally.excluded(), excluded, "seed {seed}");
        assert_eq!(
            tally.verdicts().total() + excluded,
            digests.len() as u64,
            "seed {seed}"
        );
        assert_eq!(tally.verdicts().total(), summary.trials, "seed {seed}");
        assert_eq!(
            tally.verdicts().inconclusive,
            summary.inconclusive_trials,
            "seed {seed}"
        );
        assert_eq!(tally.perturbing(), summary.perturbing_trials, "seed {seed}");

        let violating = digests
            .iter()
            .filter(|(s, d)| {
                let alone = run_props(&catalog, &[(*s, d.clone())]);
                alone
                    .finish()
                    .iter()
                    .any(|p| p.status == PropertyStatus::Violation)
            })
            .count() as u64;
        assert_eq!(tally.verdicts().violation, violating, "seed {seed}");

        let violated_idxs: std::collections::BTreeSet<u32> = fold
            .stats
            .iter()
            .filter(|s| s.violated)
            .filter_map(|s| catalog.index_of(&s.id))
            .collect();
        assert_eq!(tally.failing_ids(), &violated_idxs, "seed {seed}");
        assert!(!tally.failing_ids_truncated());
    }
}

#[test]
fn order_does_not_change_the_result() {
    for seed in 1..=30u64 {
        let mut rng = Rng(seed * 32_452_843);
        let (catalog, digests) = tree_digests(&mut rng, 40);
        let shuffled = rng.shuffled(&digests);
        let a = fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("ok");
        let b = fold_subtree(shuffled.iter().map(|(s, d)| (*s, d)), &catalog).expect("ok");
        assert_eq!(a.tally, b.tally, "seed {seed}");
        assert_eq!(a.stats, b.stats, "seed {seed}");
    }
}

#[test]
fn merging_the_tallies_of_any_split_equals_the_tally_of_the_whole() {
    for seed in 1..=30u64 {
        let mut rng = Rng(seed * 86_028_121);
        let (catalog, digests) = tree_digests(&mut rng, 50);
        let whole = fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("ok");
        let cut = rng.below(51) as usize;
        let (left, right) = digests.split_at(cut);
        let part = |ds: &[(u64, TrialDigest)]| {
            fold_subtree(ds.iter().map(|(s, d)| (*s, d)), &catalog)
                .expect("ok")
                .tally
        };
        assert_eq!(part(left).merge(&part(right)), whole.tally, "seed {seed}");
    }
}

#[test]
fn empty_input_folds_to_nothing() {
    let catalog = catalog_of(&[0, 1, 2]);
    let fold = fold_subtree(Vec::<(u64, TrialDigest)>::new(), &catalog).expect("empty is fine");
    assert_eq!(fold.tally, NodeTally::empty());
    assert!(fold.stats.is_empty());
    assert_eq!(fold.summary, RunProps::new().summary());
}

#[test]
fn the_fold_exposes_no_rate_or_interval() {
    let mut rng = Rng(77);
    let (catalog, digests) = tree_digests(&mut rng, 30);
    let fold = fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("ok");
    assert!(!fold.stats.is_empty());
    let text = format!("{fold:?}");
    for word in [
        "Rate",
        "rate:",
        "lower",
        "upper",
        "point",
        "wilson",
        "rule_of_three",
        "RateBasis",
    ] {
        assert!(!text.contains(word), "the fold output mentions {word}");
    }
}

#[test]
fn counters_saturate_as_the_run_fold_does() {
    let catalog = catalog_of(&[3]);
    let near_max = |trues: u64| TrialDigest {
        entries: vec![DigestEntry {
            idx: 0,
            kind: row(3).kind,
            catalogued: true,
            conflicted: false,
            hits: trues,
            trues,
            falses: 0,
            witness: None,
            decl: None,
        }],
        ..TrialDigest::default()
    };
    let digests = vec![
        (1, near_max(u64::MAX - 1)),
        (2, near_max(5)),
        (3, near_max(u64::MAX)),
    ];
    let oracle = run_props(&catalog, &digests).finish();
    assert_eq!(oracle[0].evals, u64::MAX, "the oracle saturates");
    let fold = fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("ok");
    assert_eq!(fold.stats[0].evals, u64::MAX);
    assert_eq!(fold.stats[0].evals_true, u64::MAX);
    assert_eq!(fold.stats[0].trials, 3);
}

#[test]
fn entries_outside_the_catalog_are_ignored_as_the_run_fold_ignores_them() {
    let catalog = catalog_of(&[0]);
    let mut digest = digest_in(
        &LogicalTrial {
            flags: DigestFlags::default(),
            ids_dropped: 0,
            entries: vec![LogicalEntry {
                id: 0,
                trues: 1,
                falses: 0,
                conflicted: false,
            }],
        },
        &catalog,
    );
    digest.entries.push(DigestEntry {
        idx: 99,
        ..digest.entries[0].clone()
    });
    let digests = vec![(1, digest)];
    let fold = fold_subtree(digests.iter().map(|(s, d)| (*s, d)), &catalog).expect("ok");
    assert_eq!(
        fold.stats,
        run_props(&catalog, &digests)
            .finish()
            .into_iter()
            .map(expected_stat)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_digest_cap_is_exact_and_the_stream_is_not_read_past_it() {
    let catalog = RunCatalog::new();
    let pulled = Cell::new(0usize);
    let stream = |n: usize| {
        pulled.set(0);
        (0..n).map(|i| {
            pulled.set(pulled.get() + 1);
            (i as u64, empty_digest())
        })
    };

    let at_cap =
        fold_subtree(stream(MAX_DIGESTS_PER_FOLD), &catalog).expect("exactly the cap folds");
    assert_eq!(at_cap.tally.verdicts().pass, MAX_DIGESTS_PER_FOLD as u64);

    let over = fold_subtree(stream(MAX_DIGESTS_PER_FOLD + 50), &catalog);
    assert_eq!(
        over.unwrap_err(),
        FoldError::LimitExceeded {
            limit: MAX_DIGESTS_PER_FOLD
        }
    );
    assert_eq!(
        pulled.get(),
        MAX_DIGESTS_PER_FOLD + 1,
        "one digest past the cap is read, no more"
    );
}

#[test]
fn excluded_digests_count_toward_the_cap_but_not_the_statistics() {
    let catalog = catalog_of(&[0]);
    let excluded = TrialDigest {
        flags: DigestFlags {
            excluded: true,
            ..DigestFlags::default()
        },
        ..TrialDigest::default()
    };
    let fold = fold_subtree(vec![(1, excluded)], &catalog).expect("ok");
    assert_eq!((fold.tally.excluded(), fold.summary.trials), (1, 0));
    assert!(fold.stats.is_empty());
}

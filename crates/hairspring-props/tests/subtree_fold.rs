//! `fold_subtree` must agree with `RunProps` over the same digests on every statistic they share, take its verdicts
//! from the store and not the digest, carry no rate, and stop at its digest cap.
mod common;

use common::subtree_gen::*;
use hairspring_props::*;
use std::cell::Cell;
use std::collections::BTreeSet;

type Trial = (u64, String, TrialDigest);

const OVERRIDES: [&str; 5] = ["pass", "violation", "inconclusive", "unusable", "infra"];

fn run_props(catalog: &RunCatalog, trials: &[Trial]) -> RunProps {
    trials.iter().fold(RunProps::new(), |run, (seed, _, d)| {
        run.add_digest(*seed, d, catalog)
    })
}

/// What the store would record: mostly what the digest implies, sometimes a verdict the digest cannot show (a hang or
/// an exit-code violation whose properties all held).
fn store_verdict(rng: &mut Rng, catalog: &RunCatalog, d: &TrialDigest) -> String {
    if rng.chance(15) {
        return OVERRIDES[rng.below(OVERRIDES.len() as u64) as usize].to_owned();
    }
    let alone = run_props(
        catalog,
        &[
            (1, String::new(), priming_digest(catalog)),
            (2, String::new(), d.clone()),
        ],
    );
    let name = if d.flags.excluded {
        "unusable"
    } else if d.flags.inconclusive {
        "inconclusive"
    } else if alone
        .finish()
        .iter()
        .any(|p| p.status == PropertyStatus::Violation)
    {
        "violation"
    } else {
        "pass"
    };
    name.to_owned()
}

/// A tree catalog and trials against it. The first trial primes every id with the catalog's kind, as a real run's
/// catalog would have, so kind mismatches in later trials are mismatches in the run fold too.
fn tree_trials(rng: &mut Rng, count: usize) -> (RunCatalog, Vec<Trial>) {
    let members: Vec<usize> = rng.shuffled(&(0..UNIVERSE).collect::<Vec<_>>());
    let catalog = catalog_of(&members);
    let mut trials = vec![(0, "pass".to_owned(), priming_digest(&catalog))];
    for seed in 0..count as u64 {
        let d = digest_in(&logical_trial(rng, &members), &catalog);
        trials.push((seed * 3 + 1, store_verdict(rng, &catalog, &d), d));
    }
    (catalog, trials)
}

fn fold(catalog: &RunCatalog, trials: &[Trial]) -> SubtreeFold {
    fold_subtree(trials.iter().map(|(s, v, d)| (*s, v.as_str(), d)), catalog)
        .expect("under the cap")
}

fn expected_stat(s: PropertyStat) -> SubtreeStat {
    SubtreeStat::from(s)
}

fn counts_of(trials: &[Trial]) -> VerdictCounts {
    trials
        .iter()
        .fold(VerdictCounts::default(), |c, (_, v, _)| c.with_named(v, 1))
}

#[test]
fn fold_subtree_equals_run_props_over_the_same_digests() {
    for seed in 1..=80u64 {
        let mut rng = Rng(seed * 982_451_653);
        let count = rng.below(70) as usize;
        let (catalog, trials) = tree_trials(&mut rng, count);
        let oracle = run_props(&catalog, &trials);
        let summary = oracle.summary();
        let expected: Vec<SubtreeStat> = oracle.finish().into_iter().map(expected_stat).collect();

        let got = fold(&catalog, &trials);
        assert_eq!(got.summary, summary, "seed {seed}");
        assert_eq!(got.stats, expected, "seed {seed}");
    }
}

#[test]
fn the_tally_agrees_with_the_run_fold_including_mismatches_and_conflict_only_entries() {
    let mut saw_mismatch = false;
    let mut saw_conflict_only = false;
    for seed in 1..=80u64 {
        let mut rng = Rng(seed * 49_979_687);
        let count = 1 + rng.below(70) as usize;
        let (catalog, trials) = tree_trials(&mut rng, count);
        let got = fold(&catalog, &trials);
        let (tally, summary) = (&got.tally, &got.summary);
        saw_mismatch |= summary.kind_mismatches > 0;
        saw_conflict_only |= trials
            .iter()
            .any(|(_, _, d)| d.entries.iter().any(|e| e.conflicted && !e.catalogued));

        assert_eq!(
            tally.verdicts(),
            &counts_of(&trials),
            "seed {seed}: verdicts are the store's"
        );
        let excluded = trials.iter().filter(|(_, _, d)| d.flags.excluded).count() as u64;
        assert_eq!(tally.excluded(), excluded, "seed {seed}");
        assert_eq!(
            tally.degraded(),
            summary.degraded_trials,
            "seed {seed}: degraded follows the run fold"
        );
        assert_eq!(tally.perturbing(), summary.perturbing_trials, "seed {seed}");

        let violated: BTreeSet<u32> = got
            .stats
            .iter()
            .filter(|s| s.violated)
            .filter_map(|s| catalog.index_of(&s.id))
            .collect();
        assert_eq!(
            tally.failing_ids(),
            &violated,
            "seed {seed}: failing ids are the run fold's violations"
        );
        assert!(tally.failing_ids_exhaustive(), "seed {seed}");
    }
    assert!(
        saw_mismatch && saw_conflict_only,
        "the generator must exercise both cases"
    );
}

#[test]
fn a_degraded_timed_out_trial_counts_as_degraded_as_the_run_fold_counts_it() {
    let catalog = catalog_of(&[0]);
    let timed_out = TrialDigest {
        flags: DigestFlags {
            inconclusive: true,
            degraded: true,
            ..DigestFlags::default()
        },
        ..TrialDigest::default()
    };
    let trials = vec![(1, "inconclusive".to_owned(), timed_out)];
    let got = fold(&catalog, &trials);
    assert_eq!(run_props(&catalog, &trials).summary().degraded_trials, 1);
    assert_eq!(got.summary.degraded_trials, 1);
    assert_eq!(got.tally.degraded(), 1);
}

#[test]
fn verdict_only_digests_keep_their_stored_verdicts() {
    // The shape sim and WASM nodes have: a digest with no property entries, and a violation the digest cannot show.
    let catalog = RunCatalog::new();
    let names = [
        "violation",
        "violation",
        "pass",
        "infra",
        "unusable",
        "inconclusive",
        "novel",
    ];
    let trials: Vec<Trial> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (i as u64, (*n).to_owned(), TrialDigest::default()))
        .collect();
    let got = fold(&catalog, &trials);
    let v = got.tally.verdicts();
    assert_eq!(
        (
            v.violation,
            v.pass,
            v.infra,
            v.unusable,
            v.inconclusive,
            v.unknown
        ),
        (2, 1, 1, 1, 1, 1)
    );
    assert!(got.stats.is_empty());
    assert!(
        got.tally.failing_ids_exhaustive(),
        "a digest with no entries says no property failed"
    );
}

#[test]
fn order_does_not_change_the_result() {
    for seed in 1..=30u64 {
        let mut rng = Rng(seed * 32_452_843);
        let (catalog, trials) = tree_trials(&mut rng, 40);
        let (first, rest) = trials.split_first().expect("priming trial");
        let mut shuffled = vec![first.clone()];
        shuffled.extend(rng.shuffled(rest));
        let a = fold(&catalog, &trials);
        let b = fold(&catalog, &shuffled);
        assert_eq!(a.tally, b.tally, "seed {seed}");
        assert_eq!(a.stats, b.stats, "seed {seed}");
    }
}

#[test]
fn merging_the_tallies_of_any_split_keeps_every_count_of_the_whole() {
    for seed in 1..=30u64 {
        let mut rng = Rng(seed * 86_028_121);
        let (catalog, trials) = tree_trials(&mut rng, 50);
        let whole = fold(&catalog, &trials).tally;
        let (left, right) = trials.split_at(rng.below(51) as usize);
        let merged = fold(&catalog, left)
            .tally
            .merge(&fold(&catalog, right).tally);
        assert_eq!(merged.verdicts(), whole.verdicts(), "seed {seed}");
        assert_eq!(
            (merged.excluded(), merged.degraded(), merged.perturbing()),
            (whole.excluded(), whole.degraded(), whole.perturbing()),
            "seed {seed}"
        );
    }
}

#[test]
fn empty_input_folds_to_nothing() {
    let catalog = catalog_of(&[0, 1, 2]);
    let got =
        fold_subtree(Vec::<(u64, &str, TrialDigest)>::new(), &catalog).expect("empty is fine");
    assert_eq!(got.tally, NodeTally::empty());
    assert!(got.stats.is_empty());
    assert_eq!(got.summary, RunProps::new().summary());
}

#[test]
fn the_fold_exposes_no_rate_or_interval() {
    let mut rng = Rng(77);
    let (catalog, trials) = tree_trials(&mut rng, 30);
    let got = fold(&catalog, &trials);
    assert!(!got.stats.is_empty());
    let text = format!("{got:?}");
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
    let trials: Vec<Trial> = vec![
        (1, "pass".to_owned(), near_max(u64::MAX - 1)),
        (2, "pass".to_owned(), near_max(5)),
        (3, "pass".to_owned(), near_max(u64::MAX)),
    ];
    let oracle = run_props(&catalog, &trials).finish();
    assert_eq!(oracle[0].evals, u64::MAX, "the oracle saturates");
    let got = fold(&catalog, &trials);
    assert_eq!(got.stats[0].evals, u64::MAX);
    assert_eq!(got.stats[0].evals_true, u64::MAX);
    assert_eq!(got.stats[0].trials, 3);
}

#[test]
fn entries_outside_the_catalog_are_ignored_as_the_run_fold_ignores_them() {
    let catalog = catalog_of(&[0]);
    let trial = LogicalTrial {
        flags: DigestFlags::default(),
        ids_dropped: 0,
        entries: vec![LogicalEntry {
            id: 0,
            trues: 1,
            falses: 0,
            conflicted: false,
            kind_override: None,
            conflict_only: false,
        }],
    };
    let mut digest = digest_in(&trial, &catalog);
    digest.entries.push(DigestEntry {
        idx: 99,
        ..digest.entries[0].clone()
    });
    let trials = vec![(1, "pass".to_owned(), digest)];
    let got = fold(&catalog, &trials);
    let expected: Vec<SubtreeStat> = run_props(&catalog, &trials)
        .finish()
        .into_iter()
        .map(expected_stat)
        .collect();
    assert_eq!(got.stats, expected);
}

#[test]
fn the_digest_cap_is_exact_and_the_stream_is_not_read_past_it() {
    let catalog = RunCatalog::new();
    let pulled = Cell::new(0usize);
    let stream = |n: usize| {
        pulled.set(0);
        (0..n).map(|i| {
            pulled.set(pulled.get() + 1);
            (i as u64, "pass", TrialDigest::default())
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
fn excluded_digests_are_counted_by_verdict_but_add_nothing_to_the_statistics() {
    let catalog = catalog_of(&[0]);
    let excluded = TrialDigest {
        flags: DigestFlags {
            excluded: true,
            ..DigestFlags::default()
        },
        ..TrialDigest::default()
    };
    let got = fold_subtree(vec![(1, "unusable", excluded)], &catalog).expect("ok");
    assert_eq!(
        (
            got.tally.excluded(),
            got.tally.verdicts().unusable,
            got.summary.trials
        ),
        (1, 1, 0)
    );
    assert!(got.stats.is_empty());
}

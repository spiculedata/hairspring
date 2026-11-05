//! Tests written after a mutation-testing run (`cargo mutants`) showed that changing the code under test did not
//! fail any existing test. Each test says which behaviour it pins.
use hairspring_props::limits::{MAX_IDS_PER_RUN, MAX_PROVISIONAL_REFS};
use hairspring_props::run_rate::RunCounts;
use hairspring_props::*;

#[test]
fn the_host_side_limits_have_their_documented_values() {
    assert_eq!(MAX_IDS_PER_RUN, 16_384);
    assert_eq!(MAX_PROVISIONAL_REFS, 32_768);
}

#[test]
fn exactly_one_hundredth_of_a_percent_is_a_number_not_less_than() {
    assert_eq!(percent::nearest(0.0001), "0.01%");
    assert_eq!(percent::lower(0.0001), "0.01%");
    assert_eq!(percent::upper(0.0001), "0.01%");
    assert_eq!(percent::nearest(0.00009), "<0.01%");
}

#[test]
fn run_counts_report_the_hits_they_were_given() {
    let counts = RunCounts::new(3, 10, 2, 1).unwrap();
    assert_eq!(counts.hits(), 3);
    assert_eq!(counts.rated(), 10);
    assert_eq!(counts.excluded(), 3);
}

#[test]
fn a_fold_error_names_the_limit() {
    let text = FoldError::LimitExceeded { limit: 7 }.to_string();
    assert!(text.contains('7'), "{text}");
    assert!(text.contains("fold the children"), "{text}");
}

fn always_catalog(n: usize) -> RunCatalog {
    let rows = (0..n)
        .map(|i| CatalogRow {
            id: format!("rule{i}"),
            kind: AssertKind::Always,
            msg: String::new(),
            loc: None,
            decided: true,
        })
        .collect();
    RunCatalog::from_rows(rows).expect("valid rows")
}

fn entry(idx: u32, catalogued: bool, conflicted: bool, falses: u64) -> DigestEntry {
    DigestEntry {
        idx,
        kind: AssertKind::Always,
        catalogued,
        conflicted,
        hits: 1,
        trues: 1 - falses.min(1),
        falses,
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
fn a_subtree_stat_is_violated_only_for_a_property_that_failed() {
    let catalog = always_catalog(2);
    let d = digest(vec![entry(0, true, false, 1), entry(1, true, false, 0)]);
    let fold = fold_subtree([(1u64, "violation", &d)], &catalog).unwrap();
    let violated = |id: &str| fold.stats.iter().find(|s| s.id == id).unwrap().violated;
    assert!(violated("rule0"));
    assert!(!violated("rule1"));
}

#[test]
fn a_passing_entry_does_not_make_the_failing_ids_incomplete() {
    let catalog = always_catalog(2);
    let d = digest(vec![entry(0, true, false, 0), entry(1, true, false, 1)]);
    let t = NodeTally::of_trial("violation", Some(&d), &catalog);
    assert!(t.failing_ids_complete());
    assert!(t.failing_ids_exhaustive());
    assert_eq!(t.failing_ids().iter().copied().collect::<Vec<_>>(), [1]);
}

#[test]
fn an_unreported_entry_that_is_not_conflicted_does_not_make_the_ids_incomplete() {
    let catalog = always_catalog(1);
    let d = digest(vec![entry(0, false, false, 0)]);
    let t = NodeTally::of_trial("pass", Some(&d), &catalog);
    assert!(t.failing_ids_complete());
}

#[test]
fn a_conflict_only_entry_leaves_the_failing_ids_incomplete() {
    let catalog = always_catalog(1);
    let d = digest(vec![entry(0, false, true, 0)]);
    let t = NodeTally::of_trial("violation", Some(&d), &catalog);
    assert!(!t.failing_ids_complete());
    assert!(!t.failing_ids_exhaustive());
}

#[test]
fn a_violation_with_no_digest_is_incomplete_and_a_pass_is_complete() {
    let catalog = always_catalog(1);
    assert!(!NodeTally::of_trial("violation", None, &catalog).failing_ids_complete());
    assert!(NodeTally::of_trial("pass", None, &catalog).failing_ids_complete());
}

#[test]
fn a_whole_subtree_fold_knows_every_failing_id_even_when_one_trial_alone_could_not() {
    let catalog = always_catalog(1);
    let d = digest(vec![entry(0, false, true, 0)]);
    assert!(!NodeTally::of_trial("violation", Some(&d), &catalog).failing_ids_complete());
    let fold = fold_subtree([(1u64, "violation", &d)], &catalog).unwrap();
    assert!(fold.tally.failing_ids_complete());
    assert!(fold.tally.failing_ids_exhaustive());
}

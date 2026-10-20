use hairspring_props::claim_text::{BuildRecord, KnowledgeFlag, Label, SeedSelection, Tier};
use hairspring_props::run_claim::claim_lines;
use hairspring_props::run_rate::RunCounts;

const ALL_FLAGS: [KnowledgeFlag; 5] = [
    KnowledgeFlag::ConfigFromPublicReproducer,
    KnowledgeFlag::TargetedWorkload,
    KnowledgeFlag::WindowWidening,
    KnowledgeFlag::BugShapedInvariant,
    KnowledgeFlag::BugKnownInAdvance,
];

const BANNED: [&str; 6] = [
    "unassisted",
    "found",
    "discovered",
    "flaky",
    "bug rate",
    "reproduced",
];

fn counts(hits: u64, rated: u64, inconclusive: u64) -> RunCounts {
    RunCounts::new(hits, rated, inconclusive, 0).expect("hits never exceed rated")
}

fn label(
    instrumented: bool,
    behaviour_changes: bool,
    knowledge: &[KnowledgeFlag],
    seeds: SeedSelection,
) -> Label {
    Label {
        build: BuildRecord {
            instrumented,
            behaviour_changes,
        },
        knowledge: Some(knowledge.to_vec()),
        seed_selection: seeds,
    }
}

fn plain() -> Label {
    label(false, false, &[], SeedSelection::Blind)
}

#[test]
fn a_run_with_no_declared_label_makes_no_claim_and_says_so() {
    let out = claim_lines(None, &counts(0, 200, 0), false);
    assert_eq!(
        out,
        [
            "chips: label not declared",
            "no claim: label not declared: the arm has no knowledge list, so it is incomplete and makes no claim",
        ]
    );
}

#[test]
fn a_declared_empty_list_gets_the_strongest_phrase_with_chips_beside_it() {
    let out = claim_lines(Some(&plain()), &counts(0, 200, 0), false);
    assert_eq!(out[0], "chips: tier: unmodified | seed selection: blind");
    assert_eq!(
        out[1],
        "claim: No trial failed a property in 200 rated trials; exact one-sided 95% upper bound on the failure rate: 1.49% (binomial, 0 of 200); unmodified binary, blind seeds; declared configuration knowledge: none."
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn a_claim_over_a_run_with_excluded_trials_says_how_many_were_not_rated() {
    let out = claim_lines(Some(&plain()), &counts(0, 200, 8), false);
    assert_eq!(
        out[1],
        "claim: No trial failed a property in 200 rated trials (8 trials not rated); exact one-sided 95% upper bound on the failure rate: 1.49% (binomial, 0 of 200); unmodified binary, blind seeds; declared configuration knowledge: none."
    );
    let one = claim_lines(Some(&plain()), &counts(0, 200, 1), false);
    assert!(one[1].contains("(1 trial not rated)"), "{one:?}");
}

#[test]
fn the_claim_bound_is_rounded_up() {
    let out = claim_lines(Some(&plain()), &counts(0, 300, 0), false);
    assert!(
        out[1].contains("failure rate: 1.00% (binomial, 0 of 300)"),
        "{out:?}"
    );
}

#[test]
fn a_hit_in_one_run_is_a_candidate_with_the_single_run_caveat_and_no_sentence() {
    let out = claim_lines(Some(&plain()), &counts(1, 40, 0), true);
    assert_eq!(out[0], "chips: tier: unmodified | seed selection: blind");
    assert_eq!(
        out[1],
        "no claim: a failure with no confirming replay is a candidate, not a claim"
    );
    assert_eq!(out[2], "caveats: single_run");
    assert_eq!(out.len(), 3);
}

#[test]
fn a_run_with_nothing_rated_makes_no_claim() {
    let out = claim_lines(Some(&plain()), &counts(0, 0, 4), false);
    assert_eq!(
        out[1],
        "no claim: no rated trials, so there is nothing to claim"
    );
}

#[test]
fn a_declared_window_widening_shows_the_assisted_chip() {
    let l = label(
        false,
        false,
        &[KnowledgeFlag::WindowWidening],
        SeedSelection::Blind,
    );
    assert_eq!(l.tier(), Tier::Assisted);
    let out = claim_lines(Some(&l), &counts(0, 50, 0), false);
    assert!(out[0].contains("tier: assisted"), "{out:?}");
    assert!(out[0].contains("window widening"), "{out:?}");
    assert!(out[1].contains("assisted build"), "{out:?}");
}

fn knowledge_sets() -> Vec<Vec<KnowledgeFlag>> {
    (0..1u32 << ALL_FLAGS.len())
        .map(|mask| {
            ALL_FLAGS
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, f)| *f)
                .collect()
        })
        .collect()
}

#[test]
fn over_every_label_and_run_shape_the_rules_hold() {
    let shapes = [
        (counts(0, 200, 0), false),
        (counts(0, 200, 7), false),
        (counts(0, 0, 3), false),
        (counts(3, 90, 1), false),
        (counts(1, 12, 0), true),
    ];
    for knowledge in knowledge_sets() {
        for (instrumented, changes) in [(false, false), (true, false), (false, true)] {
            for seeds in [
                SeedSelection::Blind,
                SeedSelection::Chosen,
                SeedSelection::Guided,
            ] {
                let l = label(instrumented, changes, &knowledge, seeds);
                for (c, stopped) in &shapes {
                    check(&l, &knowledge, c, *stopped);
                }
            }
        }
    }
}

fn check(l: &Label, knowledge: &[KnowledgeFlag], c: &RunCounts, stopped: bool) {
    let out = claim_lines(Some(l), c, stopped);
    let all = out.join("\n");
    let lower = all.to_lowercase();
    for word in BANNED {
        assert!(!lower.contains(word), "{word} in {all}");
    }
    assert!(
        all.contains(&format!("tier: {}", l.tier().as_str())),
        "{all}"
    );
    for f in knowledge {
        assert!(
            all.contains(f.phrase()),
            "{} missing from {all}",
            f.phrase()
        );
    }
    let claim = out.iter().find(|line| line.starts_with("claim: "));
    let single_run = stopped || c.hits() > 0;
    assert_eq!(claim.is_some(), !single_run && c.rated() > 0, "{all}");
    assert_eq!(all.contains("caveats: single_run"), single_run, "{all}");
    assert!(
        claim.is_some() || out.iter().any(|line| line.starts_with("no claim: ")),
        "{all}"
    );
}

#[test]
fn reporting_leaves_the_label_and_counts_unchanged() {
    let l = label(
        true,
        false,
        &[KnowledgeFlag::TargetedWorkload],
        SeedSelection::Chosen,
    );
    let c = counts(0, 30, 2);
    let (l0, c0) = (l.clone(), c);
    let _ = claim_lines(Some(&l), &c, false);
    assert_eq!((l, c), (l0, c0));
}

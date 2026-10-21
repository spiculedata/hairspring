//! Honesty labels and claim sentences are computed from recorded fields, so they cannot say more than the fields do.
use hairspring_props::claim_text::*;

const ALL_FLAGS: [KnowledgeFlag; 5] = [
    KnowledgeFlag::ConfigFromPublicReproducer,
    KnowledgeFlag::TargetedWorkload,
    KnowledgeFlag::WindowWidening,
    KnowledgeFlag::BugShapedInvariant,
    KnowledgeFlag::BugKnownInAdvance,
];
const SEEDS: [SeedSelection; 3] = [
    SeedSelection::Blind,
    SeedSelection::Chosen,
    SeedSelection::Guided,
];

fn no_failure(rated: u64) -> Subject {
    Subject::NoFailure { rated, unrated: 0 }
}

fn builds() -> Vec<BuildRecord> {
    [false, true]
        .into_iter()
        .flat_map(|instrumented| {
            [false, true]
                .into_iter()
                .map(move |behaviour_changes| BuildRecord {
                    instrumented,
                    behaviour_changes,
                })
        })
        .collect()
}

/// Every subset of the knowledge flags.
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

fn subjects() -> Vec<Subject> {
    vec![
        no_failure(4790),
        Subject::Failure {
            seed: 1016,
            state: FindingState::SingleRun,
        },
        Subject::Failure {
            seed: 1016,
            state: FindingState::Confirmed,
        },
        Subject::Failure {
            seed: 1016,
            state: FindingState::Verified,
        },
    ]
}

fn every_label() -> Vec<Label> {
    let mut out = Vec::new();
    for build in builds() {
        for knowledge in knowledge_sets() {
            for seed_selection in SEEDS {
                out.push(Label {
                    build,
                    knowledge: Some(knowledge.clone()),
                    seed_selection,
                });
            }
        }
    }
    out
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn label(build: BuildRecord, knowledge: &[KnowledgeFlag], seed_selection: SeedSelection) -> Label {
    Label {
        build,
        knowledge: Some(knowledge.to_vec()),
        seed_selection,
    }
}

const PLAIN: BuildRecord = BuildRecord {
    instrumented: false,
    behaviour_changes: false,
};

#[test]
fn tier_follows_the_build_record() {
    let tier = |instrumented, behaviour_changes| {
        label(
            BuildRecord {
                instrumented,
                behaviour_changes,
            },
            &[],
            SeedSelection::Blind,
        )
        .tier()
    };
    assert_eq!(tier(false, false), Tier::Unmodified);
    assert_eq!(tier(true, false), Tier::Instrumented);
    assert_eq!(tier(false, true), Tier::Assisted);
    assert_eq!(tier(true, true), Tier::Assisted);
}

#[test]
fn a_declared_window_widening_forces_the_assisted_tier() {
    for build in builds() {
        let l = label(
            build,
            &[KnowledgeFlag::WindowWidening],
            SeedSelection::Blind,
        );
        assert_eq!(l.tier(), Tier::Assisted);
    }
}

#[test]
fn the_word_unassisted_is_never_generated_for_any_input() {
    for l in every_label() {
        for s in subjects() {
            let out = generate(&l, &s);
            let mut all = out.text.clone().unwrap_or_default();
            all.push_str(&l.chips().join(" "));
            assert!(!all.to_lowercase().contains("unassisted"), "{all}");
        }
    }
}

#[test]
fn found_and_discovered_are_never_generated() {
    for l in every_label() {
        for s in subjects() {
            let text = generate(&l, &s).text.unwrap_or_default();
            for w in words(&text) {
                assert!(w != "found" && w != "discovered" && w != "flaky", "{text}");
            }
        }
    }
}

#[test]
fn reproduced_appears_only_for_confirmed_or_verified_findings() {
    for l in every_label() {
        for s in subjects() {
            let text = generate(&l, &s).text.unwrap_or_default();
            let has = words(&text).iter().any(|w| w == "reproduced");
            let allowed = matches!(
                s,
                Subject::Failure {
                    state: FindingState::Confirmed | FindingState::Verified,
                    ..
                }
            );
            assert_eq!(has, allowed && !text.is_empty(), "{text} for {s:?}");
        }
    }
}

#[test]
fn a_single_run_failure_gets_a_caveat_and_no_claim_sentence() {
    let l = label(PLAIN, &[], SeedSelection::Blind);
    let out = generate(
        &l,
        &Subject::Failure {
            seed: 5,
            state: FindingState::SingleRun,
        },
    );
    assert_eq!(out.text, None);
    assert!(out.caveats.contains(&"single_run"));
}

#[test]
fn an_arm_with_no_knowledge_list_produces_no_claim_and_is_listed_as_incomplete() {
    let undeclared = Label {
        build: PLAIN,
        knowledge: None,
        seed_selection: SeedSelection::Blind,
    };
    assert!(!undeclared.declared());
    for s in subjects() {
        let out = generate(&undeclared, &s);
        assert_eq!(out.text, None, "{s:?}");
        assert!(out
            .withheld
            .iter()
            .any(|w| w.contains("label not declared")));
    }
    assert_eq!(undeclared.chips(), vec!["label not declared".to_owned()]);
}

#[test]
fn an_empty_knowledge_list_is_a_declaration() {
    let l = label(PLAIN, &[], SeedSelection::Blind);
    assert!(l.declared());
    assert!(generate(&l, &no_failure(10)).text.is_some());
}

#[test]
fn the_strongest_phrase_is_unmodified_binary_blind_seeds_with_a_configuration_note() {
    let l = label(PLAIN, &[], SeedSelection::Blind);
    let text = generate(&l, &no_failure(4790)).text.unwrap();
    assert!(text.contains("unmodified binary, blind seeds"), "{text}");
    assert!(
        text.contains("declared configuration knowledge: none"),
        "{text}"
    );
}

#[test]
fn targeted_workload_and_bug_shaped_invariant_always_appear_next_to_the_claim() {
    for l in every_label() {
        let flags = l.knowledge.clone().unwrap();
        for s in subjects() {
            let Some(text) = generate(&l, &s).text else {
                continue;
            };
            if flags.contains(&KnowledgeFlag::TargetedWorkload) {
                assert!(text.contains("targeted workload"), "{text}");
            }
            if flags.contains(&KnowledgeFlag::BugShapedInvariant) {
                assert!(text.contains("bug-shaped invariant"), "{text}");
            }
        }
    }
}

#[test]
fn every_declared_flag_is_stated_in_the_note_and_the_chips() {
    for l in every_label() {
        let flags = l.knowledge.clone().unwrap();
        let text = generate(&l, &no_failure(9)).text.unwrap();
        let chips = l.chips().join(" | ");
        for f in flags {
            assert!(text.contains(f.phrase()), "{text} lacks {}", f.phrase());
            assert!(chips.contains(f.phrase()), "{chips} lacks {}", f.phrase());
        }
    }
}

#[test]
fn a_zero_failure_claim_states_the_trial_count_and_the_upper_bound() {
    let l = label(PLAIN, &[], SeedSelection::Blind);
    let text = generate(
        &l,
        &Subject::NoFailure {
            rated: 300,
            unrated: 0,
        },
    )
    .text
    .unwrap();
    assert!(
        text.starts_with("No trial failed a property in 300 rated trials"),
        "{text}"
    );
    assert!(
        text.contains(
            "exact one-sided 95% upper bound on the failure rate: 1.00% (binomial, 0 of 300)"
        ),
        "{text}"
    );
    assert!(!text.contains("at most") && !text.contains("<="), "{text}");
    assert!(!text.contains("(("), "no nested parentheses: {text}");
}

#[test]
fn a_zero_failure_claim_over_few_trials_states_the_exact_bound() {
    let l = label(PLAIN, &[], SeedSelection::Blind);
    let text = generate(&l, &no_failure(1)).text.unwrap();
    assert!(text.contains("95.00% (binomial, 0 of 1)"), "{text}");
    assert!(!text.contains("3/1") && !text.contains("at most"), "{text}");
}

#[test]
fn a_claim_over_zero_rated_trials_is_never_generated() {
    let l = label(PLAIN, &[], SeedSelection::Blind);
    let out = generate(&l, &no_failure(0));
    assert_eq!(out.text, None);
    assert!(!out.withheld.is_empty());
}

#[test]
fn a_claim_never_contains_text_from_the_program_under_test() {
    // The only inputs are numbers and enums, so there is no way to put a property id or message into a sentence.
    let l = label(
        PLAIN,
        &[KnowledgeFlag::TargetedWorkload],
        SeedSelection::Chosen,
    );
    let text = generate(
        &l,
        &Subject::Failure {
            seed: 1016,
            state: FindingState::Confirmed,
        },
    )
    .text
    .unwrap();
    assert!(text.contains("seed 1016"), "{text}");
    assert!(text.contains("chosen seeds"), "{text}");
}

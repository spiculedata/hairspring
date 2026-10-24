//! The claim wording for a failure whose confirmation checks are recorded: it follows the same ladder as every other
//! claim (nothing is "reproduced" below confirmed), says what environment the checks ran in, and never uses the words
//! the rules forbid.
use hairspring_props::claim_text::*;

fn declared(seed_selection: SeedSelection) -> Label {
    Label {
        build: BuildRecord {
            instrumented: false,
            behaviour_changes: false,
        },
        knowledge: Some(vec![]),
        seed_selection,
    }
}

fn checked(state: FindingState, pinned: bool) -> Subject {
    Subject::CheckedFailure {
        seed: 3,
        state,
        pinned,
    }
}

const STATES: [FindingState; 3] = [
    FindingState::SingleRun,
    FindingState::Confirmed,
    FindingState::Verified,
];

#[test]
fn a_failure_below_confirmed_is_a_candidate_with_no_sentence() {
    for pinned in [true, false] {
        let out = generate(
            &declared(SeedSelection::Blind),
            &checked(FindingState::SingleRun, pinned),
        );
        assert_eq!(out.text, None);
        assert!(out.caveats.contains(&"single_run"));
        assert!(!out.withheld.is_empty());
    }
}

#[test]
fn a_confirmed_failure_in_a_pinned_environment_is_reproduced_under_the_recorded_environment() {
    let out = generate(
        &declared(SeedSelection::Blind),
        &checked(FindingState::Confirmed, true),
    );
    let text = out.text.expect("a sentence");
    assert!(
        text.starts_with("Reproduced (confirmed): a property failed in seed 3"),
        "{text}"
    );
    assert!(text.contains("under the recorded environment"), "{text}");
    assert!(text.contains("unmodified binary, blind seeds"), "{text}");
    assert!(out.caveats.is_empty(), "{:?}", out.caveats);
    assert!(out.withheld.is_empty());
}

#[test]
fn a_verified_failure_says_verified() {
    let out = generate(
        &declared(SeedSelection::Blind),
        &checked(FindingState::Verified, true),
    );
    let text = out.text.expect("a sentence");
    assert!(text.starts_with("Reproduced (verified):"), "{text}");
}

#[test]
fn a_failure_checked_without_a_pinned_environment_says_so_and_carries_the_caveat() {
    let out = generate(
        &declared(SeedSelection::Blind),
        &checked(FindingState::Confirmed, false),
    );
    let text = out.text.expect("a sentence");
    assert!(text.contains("environment not pinned"), "{text}");
    assert!(!text.contains("under the recorded environment"), "{text}");
    assert!(out.caveats.contains(&"environment_not_pinned"));
}

#[test]
fn an_arm_that_never_declared_its_knowledge_makes_no_claim_about_a_checked_failure() {
    for state in STATES {
        let out = generate(&Label::undeclared(), &checked(state, true));
        assert_eq!(out.text, None, "{state:?}");
        assert!(!out.withheld.is_empty());
    }
}

#[test]
fn no_sentence_ever_says_found_discovered_or_unassisted() {
    for state in STATES {
        for pinned in [true, false] {
            for selection in [
                SeedSelection::Blind,
                SeedSelection::Chosen,
                SeedSelection::Guided,
            ] {
                let out = generate(&declared(selection), &checked(state, pinned));
                let text = out.text.unwrap_or_default().to_lowercase();
                for word in ["found", "discovered", "unassisted"] {
                    assert!(!text.contains(word), "{word} in {text}");
                }
            }
        }
    }
}

#[test]
fn reproduced_appears_only_for_confirmed_or_verified() {
    for state in STATES {
        let out = generate(&declared(SeedSelection::Blind), &checked(state, true));
        let reproduced = out
            .text
            .as_deref()
            .is_some_and(|t| t.to_lowercase().contains("reproduced"));
        assert_eq!(
            reproduced,
            matches!(state, FindingState::Confirmed | FindingState::Verified),
            "{state:?}"
        );
    }
}

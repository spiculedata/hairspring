//! Honesty labels and claim sentences, computed from recorded fields.
//!
//! A published result has to say how much help the search had. The label and the sentence built from it are derived
//! here from what was recorded (the build, the declared knowledge, how seeds were chosen) so wording cannot drift from
//! the facts. The inputs to a sentence are numbers and enums only: nothing from the program under test can reach it.
//!
//! Rules, each with a test over every combination of inputs:
//! 1. The word "unassisted" is never generated. The strongest phrase is "unmodified binary, blind seeds", followed by
//!    the configuration note.
//! 2. "Reproduced" is used only for a confirmed or verified finding. A finding seen in a single run gets the
//!    `single_run` caveat and no sentence.
//! 3. "Found" and "discovered" are never generated.
//! 4. `targeted_workload` and `bug_shaped_invariant` always appear next to the claim, with every other declared flag.
//! 5. A label with no declared knowledge list produces no claim, and says why.

use crate::honesty::{plural, zero_failure_bound_clause};
use serde::{Deserialize, Serialize};

/// How much the build differs from the program as shipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Built as shipped, nothing added.
    Unmodified,
    /// Property calls compiled in, behaviour unchanged.
    Instrumented,
    /// Behaviour was changed to help the search.
    Assisted,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Unmodified => "unmodified",
            Tier::Instrumented => "instrumented",
            Tier::Assisted => "assisted",
        }
    }

    fn phrase(self) -> &'static str {
        match self {
            Tier::Unmodified => "unmodified binary",
            Tier::Instrumented => "instrumented build",
            Tier::Assisted => "assisted build",
        }
    }
}

/// What the person running the search knew or arranged, declared per arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeFlag {
    ConfigFromPublicReproducer,
    TargetedWorkload,
    WindowWidening,
    BugShapedInvariant,
    BugKnownInAdvance,
}

impl KnowledgeFlag {
    pub fn phrase(self) -> &'static str {
        match self {
            KnowledgeFlag::ConfigFromPublicReproducer => "configuration from a public reproducer",
            KnowledgeFlag::TargetedWorkload => "targeted workload",
            KnowledgeFlag::WindowWidening => "window widening",
            KnowledgeFlag::BugShapedInvariant => "bug-shaped invariant",
            KnowledgeFlag::BugKnownInAdvance => "bug known in advance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedSelection {
    Blind,
    Chosen,
    Guided,
}

impl SeedSelection {
    pub fn as_str(self) -> &'static str {
        match self {
            SeedSelection::Blind => "blind",
            SeedSelection::Chosen => "chosen",
            SeedSelection::Guided => "guided",
        }
    }

    fn phrase(self) -> &'static str {
        match self {
            SeedSelection::Blind => "blind seeds",
            SeedSelection::Chosen => "chosen seeds",
            SeedSelection::Guided => "guided seeds",
        }
    }
}

/// The facts about the build that decide its tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRecord {
    /// Property calls are compiled into the program.
    pub instrumented: bool,
    /// The program's behaviour was changed to help the search.
    pub behaviour_changes: bool,
}

/// An arm's recorded label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub build: BuildRecord,
    /// `None` when the arm never declared its knowledge. An empty list is a declaration: "none".
    pub knowledge: Option<Vec<KnowledgeFlag>>,
    pub seed_selection: SeedSelection,
}

impl Label {
    /// Derived, never set: a declared window widening forces `Assisted`.
    pub fn tier(&self) -> Tier {
        let widened = self
            .knowledge
            .as_ref()
            .is_some_and(|k| k.contains(&KnowledgeFlag::WindowWidening));
        if widened || self.build.behaviour_changes {
            Tier::Assisted
        } else if self.build.instrumented {
            Tier::Instrumented
        } else {
            Tier::Unmodified
        }
    }

    /// The label of an arm that never declared its knowledge: it makes no claim.
    pub fn undeclared() -> Label {
        Label {
            build: BuildRecord {
                instrumented: false,
                behaviour_changes: false,
            },
            knowledge: None,
            seed_selection: SeedSelection::Blind,
        }
    }

    pub fn declared(&self) -> bool {
        self.knowledge.is_some()
    }

    /// The chips shown beside a result.
    pub fn chips(&self) -> Vec<String> {
        let Some(knowledge) = &self.knowledge else {
            return vec!["label not declared".to_owned()];
        };
        let mut chips = vec![
            format!("tier: {}", self.tier().as_str()),
            format!("seed selection: {}", self.seed_selection.as_str()),
        ];
        chips.extend(knowledge.iter().map(|f| f.phrase().to_owned()));
        chips
    }

    fn knowledge_note(&self) -> String {
        let flags = self.knowledge.as_deref().unwrap_or_default();
        let list = if flags.is_empty() {
            "none".to_owned()
        } else {
            flags
                .iter()
                .map(|f| f.phrase())
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("declared configuration knowledge: {list}")
    }

    fn how(&self) -> String {
        format!("{}, {}", self.tier().phrase(), self.seed_selection.phrase())
    }
}

/// Where a finding stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingState {
    /// Seen in one run only.
    SingleRun,
    /// Replayed and seen again.
    Confirmed,
    Verified,
}

/// What a claim is about. Non-exhaustive: callers outside this crate must allow for subjects added later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Subject {
    /// No trial failed a property in `rated` rated trials, and `unrated` trials were left out of that count.
    NoFailure { rated: u64, unrated: u64 },
    /// A property failed in `seed`.
    Failure { seed: u64, state: FindingState },
    /// A hit seen in the one run that is being reported, with no replay: a candidate, never a claim.
    SingleRunHit,
}

/// A claim sentence, if one may be made, and the reasons it was not.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ClaimOutput {
    pub text: Option<String>,
    pub caveats: Vec<&'static str>,
    pub withheld: Vec<&'static str>,
}

const NOT_DECLARED: &str =
    "label not declared: the arm has no knowledge list, so it is incomplete and makes no claim";
const NOTHING_RATED: &str = "no rated trials, so there is nothing to claim";
const SINGLE_RUN: &str = "a failure with no confirming replay is a candidate, not a claim";

/// Trials left out of a count are said so beside it, so the sentence does not read as covering them.
fn unrated_note(unrated: u64) -> String {
    match unrated {
        0 => String::new(),
        n => format!(" ({} not rated)", plural(n, "trial", "trials")),
    }
}

/// The claim for `subject` under `label`.
pub fn generate(label: &Label, subject: &Subject) -> ClaimOutput {
    let mut out = ClaimOutput::default();
    if matches!(
        subject,
        Subject::SingleRunHit
            | Subject::Failure {
                state: FindingState::SingleRun,
                ..
            }
    ) {
        out.caveats.push("single_run");
        out.withheld.push(SINGLE_RUN);
    }
    if !label.declared() {
        out.withheld.push(NOT_DECLARED);
    }
    if matches!(subject, Subject::NoFailure { rated: 0, .. }) {
        out.withheld.push(NOTHING_RATED);
    }
    if !out.withheld.is_empty() {
        return out;
    }
    let how = label.how();
    let note = label.knowledge_note();
    out.text = Some(match *subject {
        // Already withheld above; there is never a sentence for a candidate.
        Subject::SingleRunHit => return out,
        Subject::NoFailure { rated, unrated } => format!(
            "No trial failed a property in {}{}; {}; {how}; {note}.",
            plural(rated, "rated trial", "rated trials"),
            unrated_note(unrated),
            zero_failure_bound_clause(rated)
        ),
        Subject::Failure { seed, state } => {
            let status = match state {
                FindingState::Verified => "verified",
                _ => "confirmed",
            };
            format!("Reproduced ({status}): a property failed in seed {seed}; {how}; {note}.")
        }
    });
    out
}

//! The claim lines of one run's report: chips, then a claim sentence or the reasons there is none.
//!
//! Everything comes from the recorded label and the counts. Nothing typed by hand and nothing from the program under
//! test reaches a line. The wording rules are those of [`crate::claim_text`]; this module only decides which subject a
//! run has and lays the answer out.

use crate::claim_text::{generate, Label, Subject};
use crate::run_rate::RunCounts;

/// A run that ended on its first hit, or has any hit, is a single-run candidate; otherwise its subject is the rated
/// runs that did not hit.
fn subject(counts: &RunCounts, stopped_at_first_hit: bool) -> Subject {
    if stopped_at_first_hit || counts.hits() > 0 {
        Subject::SingleRunHit
    } else {
        Subject::NoFailure {
            rated: counts.rated(),
            unrated: counts.excluded(),
        }
    }
}

/// The lines for one run under its recorded `label` (`None` when none was declared, which makes no claim).
pub fn claim_lines(
    label: Option<&Label>,
    counts: &RunCounts,
    stopped_at_first_hit: bool,
) -> Vec<String> {
    let undeclared = Label::undeclared();
    let label = label.unwrap_or(&undeclared);
    let claim = generate(label, &subject(counts, stopped_at_first_hit));
    let mut lines = vec![format!("chips: {}", label.chips().join(" | "))];
    match claim.text {
        Some(text) => lines.push(format!("claim: {text}")),
        None => lines.push(format!("no claim: {}", claim.withheld.join("; "))),
    }
    if !claim.caveats.is_empty() {
        lines.push(format!("caveats: {}", claim.caveats.join(", ")));
    }
    lines
}

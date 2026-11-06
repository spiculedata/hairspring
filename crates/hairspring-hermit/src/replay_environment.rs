//! What replay does with the environment a bundle recorded: report how it differs, and refuse when the difference
//! makes the recorded schedule unreliable.
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{compare, EnvironmentComparison, EnvironmentIdentity};
use hairspring_engine::EngineError;

/// Fields whose change means the dynamic loader behaves differently, which moves the guest's first branches and so the
/// whole schedule. Measured: a rewritten `ld.so.cache` alone made old recordings stop replaying.
const LOADER_PREFIX: &str = "host.loader.";
const HERMIT_HASH: &str = "hermit.path.sha256";
const NOT_RECORDED: &str = "hermit.path.sha256 was not recorded";
const NOT_RECORDED_AND_UNREADABLE: &str =
    "hermit.path.sha256 was not recorded and cannot be read now";
const NOT_NOTICED: &str = "a different Hermit build would not be noticed";

/// The fields that say where, and with what arguments, the program was started. Replay always starts it from the
/// current configuration, so a change here is reported rather than followed.
const TARGET_FIELDS: [&str; 2] = ["target.command.sha256", "target.cwd.sha256"];
const TARGET_DIFFERS: &str = "command line or working directory differs from the recording";
const TARGET_DIFFERS_EFFECT: &str =
    "replay started the program from the current configuration; a program that reads its own paths can follow a different schedule";

const GUEST_PASSED: &str = "env.guest.passed";
const PASSED_NONE: &str = "none";

/// Added to a miss when the command line or working directory differs.
pub const MISS_WITH_CHANGED_TARGET: &str =
    "possible cause: the command line or working directory differs from the recording";

/// Said when a replay misses on a host that differs from the recording.
pub const MISS_ON_CHANGED_HOST: &str =
    "the environment differs from the recording, so this is not evidence the bug is fixed";

pub struct EnvironmentCheck {
    pub warnings: Vec<String>,
    pub differs: bool,
    /// The command line or working directory is not the recorded one.
    pub target_differs: bool,
}

fn changed_target_fields(comparison: &EnvironmentComparison) -> Vec<&str> {
    match comparison {
        EnvironmentComparison::Differs(changes) => changes
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| TARGET_FIELDS.contains(name))
            .collect(),
        _ => Vec::new(),
    }
}

/// A line naming the target fields an otherwise recorded environment lacks, so their absence is not silent.
fn target_not_recorded(recorded: &EnvironmentIdentity) -> Option<String> {
    let missing: Vec<&str> = TARGET_FIELDS
        .iter()
        .copied()
        .filter(|name| recorded.get(name).is_none())
        .collect();
    let verb = if missing.len() == 1 { "was" } else { "were" };
    (!missing.is_empty()).then(|| {
        format!(
            "{} {verb} not recorded: a changed command line or working directory would not be noticed",
            missing.join(" and ")
        )
    })
}

/// The line that says how to replay with the recorded `pass_env` setting, when `env.guest.passed` is what differs. A
/// recording from before the field existed reads as `none`, so replaying one under the default says so too.
fn guest_passed_note(comparison: &EnvironmentComparison) -> Option<String> {
    let EnvironmentComparison::Differs(changes) = comparison else {
        return None;
    };
    let change = changes.iter().find(|c| c.name == GUEST_PASSED)?;
    let now = change.current.as_deref().unwrap_or("unavailable");
    let advice = if change.recorded == PASSED_NONE {
        "set pass_env = false on the target to replay as recorded"
    } else {
        "set pass_env = true on the target, with the --base-env it had, to replay as recorded"
    };
    Some(format!(
        "the bundle was recorded with {GUEST_PASSED}={} and this target now uses {now}; {advice}",
        change.recorded
    ))
}

fn loader_changed(comparison: &EnvironmentComparison) -> bool {
    match comparison {
        EnvironmentComparison::Differs(changes) => {
            changes.iter().any(|c| c.name.starts_with(LOADER_PREFIX))
        }
        _ => false,
    }
}

/// Compares the bundle's recorded environment with `current`. Each recorded field is read on its own, so one malformed
/// field is reported and the rest still count. Refuses (as an environment mismatch, not a verdict) when the loader
/// changed, unless `allow_change` is set; every other difference is reported and the replay goes ahead.
pub fn check(
    bundle: &Bundle,
    current: &EnvironmentIdentity,
    allow_change: bool,
) -> Result<EnvironmentCheck, EngineError> {
    let parsed = EnvironmentIdentity::from_bundle(bundle);
    let comparison = compare(parsed.identity.as_ref(), current);
    let mut warnings: Vec<String> = parsed.warning().into_iter().collect();
    // A field that could not be read cannot be checked, so it counts as a change: it must not let a miss pass as a fix.
    // The same goes for a field this build does not know: it was written by a newer build and cannot be compared.
    let tampered = parsed.is_partly_unreadable();
    if loader_changed(&comparison) && !allow_change {
        let detail = comparison.warning().unwrap_or_default();
        return Err(EngineError::EnvironmentMismatch(format!(
            "replay refused, the dynamic loader changed since the bundle was recorded and its schedule may not reproduce (pass --allow-environment-change on the CLI, or allow_environment_change in the API, to replay anyway); {detail}"
        )));
    }
    if let Some(recorded) = &parsed.identity {
        // The comparison only looks at recorded fields, so a hash missing from the recording would otherwise read as
        // "the same binary". (A hash recorded but unreadable now is already a reported change.)
        let readable_now = current.get(HERMIT_HASH).is_some();
        if recorded.get(HERMIT_HASH).is_none() {
            let why = if readable_now {
                NOT_RECORDED
            } else {
                NOT_RECORDED_AND_UNREADABLE
            };
            warnings.push(format!("{why}: {NOT_NOTICED}"));
        }
        warnings.extend(target_not_recorded(recorded));
    }
    warnings.extend(comparison.warning());
    warnings.extend(parsed.unknown_warning());
    warnings.extend(guest_passed_note(&comparison));
    let target_changed = changed_target_fields(&comparison);
    if !target_changed.is_empty() {
        warnings.push(format!(
            "{TARGET_DIFFERS} ({}): {TARGET_DIFFERS_EFFECT}",
            target_changed.join(", ")
        ));
    }
    Ok(EnvironmentCheck {
        warnings,
        differs: comparison.differs() || tampered,
        target_differs: !target_changed.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(pairs: &[(&str, &str)]) -> EnvironmentIdentity {
        pairs
            .iter()
            .fold(EnvironmentIdentity::default(), |id, (k, v)| {
                id.with_field(*k, *v).unwrap()
            })
    }

    fn bundle_with(id: &EnvironmentIdentity) -> Bundle {
        id.apply_to(Bundle::new(1, "b", vec![]))
    }

    fn warnings(recorded: &EnvironmentIdentity, current: &EnvironmentIdentity) -> Vec<String> {
        check(&bundle_with(recorded), current, false)
            .unwrap()
            .warnings
    }

    fn with_hash() -> EnvironmentIdentity {
        identity(&[
            ("host.container", "none"),
            ("hermit.path.sha256", &"a".repeat(64)),
            ("target.command.sha256", &"b".repeat(64)),
            ("target.cwd.sha256", &"c".repeat(64)),
        ])
    }

    fn without_hash() -> EnvironmentIdentity {
        identity(&[
            ("host.container", "none"),
            ("target.command.sha256", &"b".repeat(64)),
            ("target.cwd.sha256", &"c".repeat(64)),
        ])
    }

    #[test]
    fn a_hash_missing_on_both_sides_says_so() {
        assert_eq!(
            warnings(&without_hash(), &without_hash()),
            [format!("{NOT_RECORDED_AND_UNREADABLE}: {NOT_NOTICED}")]
        );
    }

    #[test]
    fn a_hash_not_recorded_but_readable_now_says_so() {
        assert_eq!(
            warnings(&without_hash(), &with_hash()),
            [format!("{NOT_RECORDED}: {NOT_NOTICED}")]
        );
    }

    #[test]
    fn a_hash_recorded_but_unreadable_now_is_reported_as_unavailable() {
        let w = warnings(&with_hash(), &without_hash());
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].contains("hermit.path.sha256 (recorded aaaaaaaaaaaa, now unavailable)"),
            "{w:?}"
        );
    }

    #[test]
    fn a_hash_present_and_equal_on_both_sides_gives_no_warning() {
        assert!(warnings(&with_hash(), &with_hash()).is_empty());
    }

    fn with_passed(value: &str) -> EnvironmentIdentity {
        with_hash().with_field(GUEST_PASSED, value).unwrap()
    }

    fn passed_notes(recorded: &EnvironmentIdentity, current: &EnvironmentIdentity) -> Vec<String> {
        warnings(recorded, current)
            .into_iter()
            .filter(|w| w.starts_with("the bundle was recorded with"))
            .collect()
    }

    #[test]
    fn an_old_recording_under_the_new_default_says_how_to_replay_as_recorded() {
        let notes = passed_notes(&with_hash(), &with_passed("hairspring-1"));
        assert_eq!(
            notes,
            ["the bundle was recorded with env.guest.passed=none and this target now uses hairspring-1; set pass_env = false on the target to replay as recorded"]
        );
        let check = check(
            &bundle_with(&with_hash()),
            &with_passed("hairspring-1"),
            false,
        )
        .unwrap();
        assert!(check.differs);
    }

    #[test]
    fn a_new_recording_replayed_with_passing_off_says_how_to_turn_it_on() {
        let notes = passed_notes(&with_passed("hairspring-1"), &with_passed("none"));
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("env.guest.passed=hairspring-1"),
            "{notes:?}"
        );
        assert!(notes[0].contains("now uses none"), "{notes:?}");
        assert!(notes[0].contains("set pass_env = true"), "{notes:?}");
    }

    #[test]
    fn no_note_when_the_setting_is_the_same_or_an_old_recording_meets_none() {
        assert!(passed_notes(&with_passed("none"), &with_passed("none")).is_empty());
        assert!(passed_notes(&with_hash(), &with_passed("none")).is_empty());
        assert!(
            !check(&bundle_with(&with_hash()), &with_passed("none"), false)
                .unwrap()
                .differs
        );
    }

    #[test]
    fn a_bundle_with_no_record_gets_only_the_no_record_note() {
        let now = identity(&[("host.container", "none")]);
        let out = check(&Bundle::new(1, "b", vec![]), &now, false).unwrap();
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("no environment recorded"));
    }
}

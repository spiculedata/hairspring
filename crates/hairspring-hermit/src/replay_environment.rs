//! What replay does with the environment a bundle recorded: report how it differs, and refuse when the difference
//! makes the recorded schedule unreliable.
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{compare, EnvironmentComparison, EnvironmentIdentity};
use hairspring_engine::EngineError;

/// Fields whose change means the dynamic loader behaves differently, which moves the guest's first branches and so the
/// whole schedule. Measured: a rewritten `ld.so.cache` alone made old recordings stop replaying.
const LOADER_PREFIX: &str = "host.loader.";
const HERMIT_HASH: &str = "hermit.path.sha256";

/// Said when a replay misses on a host that differs from the recording.
pub const MISS_ON_CHANGED_HOST: &str =
    "the environment differs from the recording, so this is not evidence the bug is fixed";

pub struct EnvironmentCheck {
    pub warnings: Vec<String>,
    pub differs: bool,
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
    let tampered = !parsed.malformed.is_empty();
    if loader_changed(&comparison) && !allow_change {
        let detail = comparison.warning().unwrap_or_default();
        return Err(EngineError::EnvironmentMismatch(format!(
            "replay refused, the dynamic loader changed since the bundle was recorded and its schedule may not reproduce (pass --allow-environment-change on the CLI, or allow_environment_change in the API, to replay anyway); {detail}"
        )));
    }
    if let Some(recorded) = &parsed.identity {
        // Nothing is compared for a field neither side has, and that silence reads as "the same binary".
        if recorded.get(HERMIT_HASH).is_none() && current.get(HERMIT_HASH).is_none() {
            warnings.push(format!(
                "{HERMIT_HASH} unavailable on both sides: cannot compare"
            ));
        }
    }
    warnings.extend(comparison.warning());
    Ok(EnvironmentCheck {
        warnings,
        differs: comparison.differs() || tampered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNAVAILABLE: &str = "hermit.path.sha256 unavailable on both sides: cannot compare";

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

    #[test]
    fn a_hermit_hash_missing_on_both_sides_is_said_not_silently_equal() {
        let recorded = identity(&[("host.container", "none")]);
        let now = identity(&[("host.container", "none")]);
        assert!(warnings(&recorded, &now).iter().any(|w| w == UNAVAILABLE));
    }

    #[test]
    fn a_hermit_hash_present_on_either_side_gives_no_such_warning() {
        let hash = "a".repeat(64);
        let with = identity(&[("host.container", "none"), ("hermit.path.sha256", &hash)]);
        let without = identity(&[("host.container", "none")]);
        assert!(!warnings(&with, &with).iter().any(|w| w == UNAVAILABLE));
        assert!(!warnings(&with, &without).iter().any(|w| w == UNAVAILABLE));
        assert!(!warnings(&without, &with).iter().any(|w| w == UNAVAILABLE));
    }

    #[test]
    fn a_bundle_with_no_record_gets_only_the_no_record_note() {
        let now = identity(&[("host.container", "none")]);
        let out = check(&Bundle::new(1, "b", vec![]), &now, false).unwrap();
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert!(out.warnings[0].contains("no environment recorded"));
    }
}

//! What replay does with the environment a bundle recorded: report how it differs, and refuse when the difference
//! makes the recorded schedule unreliable.
use hairspring_core::bundle::Bundle;
use hairspring_core::environment::{compare, EnvironmentComparison, EnvironmentIdentity};
use hairspring_engine::EngineError;

/// Fields whose change means the dynamic loader behaves differently, which moves the guest's first branches and so the
/// whole schedule. Measured: a rewritten `ld.so.cache` alone made old recordings stop replaying.
const LOADER_PREFIX: &str = "host.loader.";

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
    warnings.extend(comparison.warning());
    Ok(EnvironmentCheck {
        warnings,
        differs: comparison.differs() || tampered,
    })
}

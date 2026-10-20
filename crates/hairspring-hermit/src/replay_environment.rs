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

/// Compares the bundle's recorded environment with `current`. Refuses (as infrastructure, not a verdict) when the
/// loader changed, unless `allow_change` is set; every other difference is reported and the replay goes ahead.
pub fn check(
    bundle: &Bundle,
    current: &EnvironmentIdentity,
    allow_change: bool,
) -> Result<EnvironmentCheck, EngineError> {
    let recorded = match EnvironmentIdentity::from_bundle(bundle) {
        Ok(recorded) => recorded,
        Err(e) => {
            return Ok(EnvironmentCheck {
                warnings: vec![format!(
                    "the recorded environment is malformed ({e}); it was not compared"
                )],
                differs: false,
            })
        }
    };
    let comparison = compare(recorded.as_ref(), current);
    if loader_changed(&comparison) && !allow_change {
        let detail = comparison.warning().unwrap_or_default();
        return Err(EngineError::Backend(format!(
            "{detail}; the dynamic loader changed since the bundle was recorded, so its schedule may not reproduce. Replay refused: pass --allow-environment-change to replay anyway"
        )));
    }
    Ok(EnvironmentCheck {
        warnings: comparison.warning().into_iter().collect(),
        differs: comparison.differs(),
    })
}

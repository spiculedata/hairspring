//! The environment a guest program sees. Hermit passes the host environment through, and the host
//! filesystem and environment are *inputs* to a run, so we start from nothing and pin everything.
use std::collections::BTreeMap;

const FIXED_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
/// A directory that does not exist: Python finds no bytecode cache and, with writing disabled, never creates one.
const EMPTY_PYCACHE: &str = "/hairspring-empty-pycache";

fn is_python(command: &[String]) -> bool {
    command
        .first()
        .and_then(|c| c.rsplit('/').next())
        .is_some_and(|name| name.starts_with("python"))
}

/// Minimal pinned environment plus the target's configured variables (which win).
pub fn guest_env(
    command: &[String],
    configured: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        ("PATH".to_string(), FIXED_PATH.to_string()),
        ("LANG".to_string(), "C".to_string()),
        ("LC_ALL".to_string(), "C".to_string()),
        ("TZ".to_string(), "UTC".to_string()),
    ]);
    if is_python(command) {
        // Native Python runs rewrite `.pyc` files, which changes what a sandboxed Python executes
        // (measured: the first run after a native run differed). Freeze that state.
        env.insert("PYTHONDONTWRITEBYTECODE".to_string(), "1".to_string());
        env.insert("PYTHONPYCACHEPREFIX".to_string(), EMPTY_PYCACHE.to_string());
        env.insert("PYTHONHASHSEED".to_string(), "0".to_string());
    }
    env.extend(configured.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

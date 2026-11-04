//! The mode a run is stored with. The service writes a run's mode into the `config_json` it hands to `create_run`
//! (the same text that becomes the job payload), and the store copies it into `runs.mode` so that listing and reading
//! a run report it without opening the payload. Both backends use these functions, so they cannot disagree.

/// The stored mode of runs that are not sweeps, and the column's default.
pub(crate) const SEARCH_MODE: &str = "search";
/// The stored mode of a sweep.
pub(crate) const SWEEP_MODE: &str = "sweep";

/// The mode `config_json` names: `sweep` only for a JSON object whose `mode` is exactly the string `sweep`. Anything
/// else (a search, a payload that predates modes, text that is not JSON) is a search, as it is for the service.
pub(crate) fn mode_of_config(config_json: &str) -> &'static str {
    let is_sweep = serde_json::from_str::<serde_json::Value>(config_json)
        .ok()
        .is_some_and(|v| v.get("mode").and_then(|m| m.as_str()) == Some(SWEEP_MODE));
    if is_sweep {
        SWEEP_MODE
    } else {
        SEARCH_MODE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_object_whose_mode_is_sweep_is_a_sweep() {
        assert_eq!(mode_of_config(r#"{"mode":"sweep"}"#), "sweep");
        assert_eq!(
            mode_of_config(r#"{"seed":1,"mode":"sweep","x":[]}"#),
            "sweep"
        );
        for other in [
            r#"{"mode":"search"}"#,
            r#"{"mode":"SWEEP"}"#,
            r#"{"mode":3}"#,
            r#"{"mode":null}"#,
            "{}",
            "[]",
            "null",
            r#""sweep""#,
            "",
            "{\"mode\":\"sweep\"",
            "not json",
        ] {
            assert_eq!(mode_of_config(other), "search", "{other}");
        }
    }
}

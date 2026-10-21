//! One-file repro bundle: plain text so it can be pasted into an issue.
//!
//! Format v2 is line oriented `key=value`; free-text values are percent-escaped
//! so a value can never inject a new line or field. v1 bundles still decode. v3 is v2 plus recorded
//! execution-environment params (the exact names in `environment::KNOWN_FIELDS`).
use crate::ActionId;
use std::collections::BTreeMap;

/// Written only by bundles that record an execution environment (see `crate::environment`). A reader that predates
/// it rejects the header, so bundles with nothing to record keep writing v2.
const HEADER_V3: &str = "hairspring-bundle v3";
const HEADER_V2: &str = "hairspring-bundle v2";
const HEADER_V1: &str = "hairspring-bundle v1";
// Bundles written under the project's former name (Anthill) still decode.
const LEGACY_HEADER_V2: &str = "anthill-bundle v2";
const LEGACY_HEADER_V1: &str = "anthill-bundle v1";
const DEFAULT_BACKEND: &str = "sim";
const PARAM_PREFIX: &str = "param.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    pub seed: u64,
    /// Build identifier; a bundle should only be replayed on the same build.
    pub build: String,
    pub path: Vec<ActionId>,
    /// Name of the target the bundle was produced from (empty if unknown).
    pub target: String,
    /// Backend that produced it: `sim`, `hermit`, `wasm`, ...
    pub backend: String,
    /// Backend specific replay parameters (for example `sched_seed`).
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BundleError {
    BadHeader,
    MissingField(&'static str),
    BadNumber(String),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::BadHeader => write!(f, "not an hairspring bundle (bad header)"),
            BundleError::MissingField(n) => write!(f, "bundle is missing required field `{n}`"),
            BundleError::BadNumber(v) => write!(f, "bundle has an invalid number `{v}`"),
        }
    }
}

impl std::error::Error for BundleError {}

fn escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\n', "%0A")
        .replace('\r', "%0D")
}

fn unescape(s: &str) -> String {
    s.replace("%0D", "\r")
        .replace("%0A", "\n")
        .replace("%25", "%")
}

impl Bundle {
    pub fn new(seed: u64, build: impl Into<String>, path: Vec<ActionId>) -> Self {
        Bundle {
            seed,
            build: build.into(),
            path,
            target: String::new(),
            backend: DEFAULT_BACKEND.to_string(),
            params: BTreeMap::new(),
        }
    }

    pub fn with_target(self, target: impl Into<String>) -> Self {
        Bundle {
            target: target.into(),
            ..self
        }
    }

    pub fn with_backend(self, backend: impl Into<String>) -> Self {
        Bundle {
            backend: backend.into(),
            ..self
        }
    }

    pub fn with_param(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let mut params = self.params;
        params.insert(key.into(), value.into());
        Bundle { params, ..self }
    }

    /// Whether any param belongs to the recorded execution environment, which is what makes a bundle v3.
    fn records_environment(&self) -> bool {
        self.params
            .keys()
            .any(|k| crate::environment::is_known_field(k))
    }

    pub fn encode(&self) -> String {
        let path: Vec<String> = self.path.iter().map(|id| format!("{id:016x}")).collect();
        let header = if self.records_environment() {
            HEADER_V3
        } else {
            HEADER_V2
        };
        let mut out = format!(
            "{header}\nseed={}\nbuild={}\ntarget={}\nbackend={}\n",
            self.seed,
            escape(&self.build),
            escape(&self.target),
            escape(&self.backend)
        );
        for (k, v) in &self.params {
            out.push_str(&format!(
                "{PARAM_PREFIX}{}={}\n",
                escape(k).replace('=', "%3D"),
                escape(v)
            ));
        }
        out.push_str(&format!("path={}\n", path.join(",")));
        out
    }

    pub fn decode(text: &str) -> Result<Bundle, BundleError> {
        let mut lines = text.lines();
        match lines.next() {
            Some(HEADER_V3)
            | Some(HEADER_V2)
            | Some(HEADER_V1)
            | Some(LEGACY_HEADER_V2)
            | Some(LEGACY_HEADER_V1) => {}
            _ => return Err(BundleError::BadHeader),
        }
        let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
        let mut params = BTreeMap::new();
        for line in lines {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if let Some(name) = k.strip_prefix(PARAM_PREFIX) {
                params.insert(unescape(&name.replace("%3D", "=")), unescape(v));
            } else {
                fields.insert(k, v);
            }
        }
        let get = |name: &'static str| {
            fields
                .get(name)
                .copied()
                .ok_or(BundleError::MissingField(name))
        };
        let seed_text = get("seed")?;
        let build = unescape(get("build")?);
        let path_text = get("path")?;
        let seed = seed_text
            .parse::<u64>()
            .map_err(|_| BundleError::BadNumber(seed_text.to_string()))?;
        let path = path_text
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| u64::from_str_radix(s, 16).map_err(|_| BundleError::BadNumber(s.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        let target = fields
            .get("target")
            .map(|v| unescape(v))
            .unwrap_or_default();
        let backend = fields
            .get("backend")
            .map(|v| unescape(v))
            .unwrap_or_else(|| DEFAULT_BACKEND.to_string());
        Ok(Bundle {
            seed,
            build,
            path,
            target,
            backend,
            params,
        })
    }
}

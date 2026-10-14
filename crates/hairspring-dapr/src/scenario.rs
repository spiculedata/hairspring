use crate::faults::FaultPlan;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const MAX_COPIES: u32 = 1000;
const MAX_TOTAL_MESSAGES: u64 = 5000;
const DEFAULT_PUBSUB: &str = "pubsub";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSection {
    pub command: Vec<String>,
    pub port: Option<u16>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    pub startup_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishSpec {
    pub pubsub: Option<String>,
    pub topic: String,
    #[serde(default)]
    pub data: Value,
    pub count: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_rounds: u32,
    pub delivery_timeout_secs: u64,
    pub max_attempts: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_rounds: 400,
            delivery_timeout_secs: 5,
            max_attempts: 6,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scenario {
    pub app: AppSection,
    pub faults: FaultPlan,
    pub publish: Vec<PublishSpec>,
    pub limits: Limits,
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Scenario, String> {
        let s: Scenario = toml::from_str(text).map_err(|e| format!("invalid scenario: {e}"))?;
        s.faults.validate()?;
        for p in &s.publish {
            if p.topic.is_empty() {
                return Err("every [[publish]] needs a `topic`".to_string());
            }
            if p.count.unwrap_or(1) > MAX_COPIES {
                return Err(format!("`count` may be at most {MAX_COPIES}"));
            }
        }
        if s.total_messages() > MAX_TOTAL_MESSAGES {
            return Err(format!(
                "a scenario may publish at most {MAX_TOTAL_MESSAGES} messages"
            ));
        }
        Ok(s)
    }

    pub fn total_messages(&self) -> u64 {
        self.publish
            .iter()
            .map(|p| u64::from(p.count.unwrap_or(1)))
            .sum()
    }

    /// Messages to publish, expanded by `count`: (pubsub, topic, data).
    pub fn messages(&self) -> Vec<(String, String, Value)> {
        self.publish
            .iter()
            .flat_map(|p| {
                (0..p.count.unwrap_or(1)).map(move |_| {
                    (
                        p.pubsub
                            .clone()
                            .unwrap_or_else(|| DEFAULT_PUBSUB.to_string()),
                        p.topic.clone(),
                        p.data.clone(),
                    )
                })
            })
            .collect()
    }
}

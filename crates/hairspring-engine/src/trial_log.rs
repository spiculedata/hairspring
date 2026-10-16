//! One reported trial: what a worker sends to the server for every trial it judged.
//!
//! The wire form is JSON. Seeds travel as decimal strings (a scheduling seed can exceed 2^53, which JSON consumers
//! cannot represent as a number), and the binary digest as lowercase hex.
use hairspring_props::CatalogUpdate;
use serde::{Deserialize, Serialize};

/// How a trial ended, as far as the run is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportedVerdict {
    Pass,
    Violation,
    /// Timed out, or cut short by the wall-clock budget. Neither a pass nor a finding.
    Inconclusive,
    /// Finished, but its events could not be used. Neither a pass nor a finding.
    Unusable,
    /// The host or Hermit failed; the run cannot go on.
    Infra,
}

impl ReportedVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportedVerdict::Pass => "pass",
            ReportedVerdict::Violation => "violation",
            ReportedVerdict::Inconclusive => "inconclusive",
            ReportedVerdict::Unusable => "unusable",
            ReportedVerdict::Infra => "infra",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialReport {
    /// Position in the run's judging order, from 0.
    pub seq: u64,
    #[serde(with = "decimal")]
    pub seed: u64,
    pub verdict: ReportedVerdict,
    /// One line of text; empty for a pass.
    pub reason: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub behaviour_hash: Option<String>,
    pub schedule_fp: Option<String>,
    pub syscalls: u64,
    pub work_ops: u64,
    pub elapsed_ms: u64,
    /// Catalog rows this trial added or decided.
    pub catalog: Vec<CatalogUpdate>,
    /// The encoded trial digest.
    #[serde(with = "hex")]
    pub digest: Vec<u8>,
    /// Kept trial detail (a JSON record), as text.
    pub detail: Option<String>,
    /// Kept output tails (JSON), as text.
    pub output: Option<String>,
}

/// A `u64` as a decimal string.
pub mod decimal {
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let text = String::deserialize(d)?;
        // Only plain digits: no sign, no spaces, no exponent.
        if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(de::Error::custom("expected a decimal string"));
        }
        text.parse().map_err(de::Error::custom)
    }
}

/// Bytes as lowercase hex.
pub mod hex {
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(v.len() * 2);
        for b in v {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() % 2 != 0 || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(de::Error::custom("expected lowercase hex"));
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(de::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> TrialReport {
        TrialReport {
            seq: 3,
            seed: u64::MAX - 7,
            verdict: ReportedVerdict::Violation,
            reason: "always 'a.b' was false".to_string(),
            exit_code: Some(1),
            signal: None,
            behaviour_hash: Some("abc".to_string()),
            schedule_fp: None,
            syscalls: 10,
            work_ops: 2,
            elapsed_ms: 5,
            catalog: Vec::new(),
            digest: vec![0, 1, 0xab, 0xff],
            detail: Some("{}".to_string()),
            output: None,
        }
    }

    #[test]
    fn a_report_round_trips_and_keeps_a_seed_above_two_to_the_fifty_three() {
        let json = serde_json::to_string(&report()).unwrap();
        assert!(
            json.contains(&format!("\"seed\":\"{}\"", u64::MAX - 7)),
            "{json}"
        );
        assert!(json.contains("\"digest\":\"0001abff\""), "{json}");
        let back: TrialReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report());
    }

    #[test]
    fn malformed_seeds_and_digests_are_refused() {
        let json = serde_json::to_string(&report()).unwrap();
        for (from, to) in [
            (format!("\"{}\"", u64::MAX - 7), "\"-5\"".to_string()),
            (format!("\"{}\"", u64::MAX - 7), "\"1e3\"".to_string()),
            (
                format!("\"{}\"", u64::MAX - 7),
                "\"99999999999999999999999\"".to_string(),
            ),
            ("\"0001abff\"".to_string(), "\"0001ABFF\"".to_string()),
            ("\"0001abff\"".to_string(), "\"0001abf\"".to_string()),
            ("\"0001abff\"".to_string(), "\"zz\"".to_string()),
        ] {
            let bad = json.replace(&from, &to);
            assert_ne!(bad, json);
            assert!(serde_json::from_str::<TrialReport>(&bad).is_err(), "{to}");
        }
        let number = json.replace(&format!("\"{}\"", u64::MAX - 7), "5");
        assert!(
            serde_json::from_str::<TrialReport>(&number).is_err(),
            "a bare number"
        );
    }
}

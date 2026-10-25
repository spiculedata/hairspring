//! One reported trial: what a worker sends to the server for every trial it judged.
//!
//! The wire form is JSON. Seeds travel as decimal strings (a scheduling seed can exceed 2^53, which JSON consumers
//! cannot represent as a number), and the binary digest as lowercase hex.
use hairspring_props::{CatalogUpdate, DigestFlags, TrialDigest};
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

/// Longest reason kept for a trial, in characters.
pub const MAX_REASON_CHARS: usize = 500;

/// One line of at most [`MAX_REASON_CHARS`] characters: control characters become spaces, runs of spaces collapse.
pub fn one_line(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(MAX_REASON_CHARS).collect()
}

impl TrialReport {
    /// The record of a trial from a backend that reports no guest events (the simulation and WebAssembly backends).
    ///
    /// Its digest is the excluded one, as for a process trial whose guest sent no events: the trial is stored for the
    /// record and counts toward no property statistic. A timed-out trial is marked inconclusive in its digest, so the
    /// digest and the verdict agree.
    pub fn without_events(
        seq: u64,
        seed: u64,
        verdict: ReportedVerdict,
        reason: &str,
        elapsed_ms: u64,
    ) -> TrialReport {
        let digest = TrialDigest {
            flags: DigestFlags {
                excluded: true,
                inconclusive: verdict == ReportedVerdict::Inconclusive,
                ..DigestFlags::default()
            },
            ..TrialDigest::default()
        };
        TrialReport {
            seq,
            seed,
            verdict,
            reason: one_line(reason),
            exit_code: None,
            signal: None,
            behaviour_hash: None,
            schedule_fp: None,
            syscalls: 0,
            work_ops: 0,
            elapsed_ms,
            catalog: Vec::new(),
            digest: digest.encode(),
            detail: None,
            output: None,
        }
    }
}

/// A `u64` as a decimal string.
pub mod decimal {
    use serde::{de, Deserialize, Deserializer, Serializer};

    /// Plain digits only: no sign, no spaces, no exponent.
    pub fn parse(text: &str) -> Result<u64, &'static str> {
        const BAD: &str = "expected a decimal string";
        if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(BAD);
        }
        text.parse().map_err(|_| BAD)
    }

    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        parse(&String::deserialize(d)?).map_err(de::Error::custom)
    }
}

/// A list of `u64` as decimal strings.
pub mod decimal_list {
    use serde::ser::SerializeSeq;
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u64], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for n in v {
            seq.serialize_element(&n.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
        Vec::<String>::deserialize(d)?
            .iter()
            .map(|t| super::decimal::parse(t).map_err(de::Error::custom))
            .collect()
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
    fn a_report_without_events_carries_a_digest_the_server_can_decode() {
        let r = TrialReport::without_events(4, 99, ReportedVerdict::Violation, "boom\nline", 7);
        let digest = TrialDigest::decode(&r.digest).unwrap();
        assert!(digest.flags.excluded && digest.entries.is_empty());
        assert!(!digest.flags.inconclusive);
        assert_eq!((r.seq, r.seed, r.elapsed_ms), (4, 99, 7));
        assert_eq!(r.reason, "boom line");
        assert!(r.catalog.is_empty() && r.detail.is_none() && r.output.is_none());
    }

    #[test]
    fn a_timed_out_report_without_events_is_marked_inconclusive_in_its_digest() {
        let r = TrialReport::without_events(0, 1, ReportedVerdict::Inconclusive, "timeout", 1);
        assert!(TrialDigest::decode(&r.digest).unwrap().flags.inconclusive);
    }

    #[test]
    fn a_reason_is_one_line_and_bounded() {
        assert_eq!(one_line("a\n\tb   c"), "a b c");
        assert_eq!(one_line(&"x".repeat(900)).chars().count(), MAX_REASON_CHARS);
    }

    #[test]
    fn a_list_of_seeds_round_trips_as_decimal_strings() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Seeds(#[serde(with = "decimal_list")] Vec<u64>);
        let seeds = Seeds(vec![1, u64::MAX]);
        let json = serde_json::to_string(&seeds).unwrap();
        assert_eq!(json, format!("[\"1\",\"{}\"]", u64::MAX));
        assert_eq!(serde_json::from_str::<Seeds>(&json).unwrap(), seeds);
        assert!(serde_json::from_str::<Seeds>("[5]").is_err());
        assert!(serde_json::from_str::<Seeds>("[\"-5\"]").is_err());
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

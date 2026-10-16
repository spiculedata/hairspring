//! The body a worker posts when a job is done.
use hairspring_engine::report::RunOutcome;
use serde_json::{json, Value};

/// The `complete` request body. Property statistics are left out: the server folds the per-trial records it was sent
/// itself and does not read a worker's figures, and a run with thousands of properties would otherwise exceed the
/// server's request body limit and the result would be lost. The per-trial records travel in their own requests; the
/// outcome carries only how many there were.
pub fn completion_body(worker_id: &str, secret: &str, outcome: RunOutcome) -> Value {
    json!({ "worker_id": worker_id, "worker_secret": secret, "outcome": outcome.without_properties() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_props::limits::MAX_IDS_PER_PROC;
    use hairspring_props::{PropertyStat, RunProps, TrialEvents};

    /// The server's request body limit (`MAX_BODY_BYTES` in `hairspring-server`).
    const SERVER_BODY_LIMIT: usize = 2 * 1024 * 1024;

    fn outcome_with_many_properties() -> RunOutcome {
        let report = concat!(
            "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\",\"sdk\":\"c\",\"sdk_version\":\"0.1.0\",\"proc\":\"p1\",\"catalog_hash\":\"x\",\"signal_handlers\":false}\n",
            "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"a.id\",\"type\":\"always\",\"must_hit\":true}\n",
            "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"a.id\",\"hits\":1,\"trues\":1,\"falses\":0}\n",
            "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\",\"status\":0,\"sites\":1,\"evals\":1,\"dropped\":0,\"threads\":0}\n",
        );
        let run = RunProps::new().add(1, &TrialEvents::from_output(report.as_bytes()).fold());
        let summary = run.summary();
        let one = run.finish().remove(0);
        let stats: Vec<PropertyStat> = (0..MAX_IDS_PER_PROC)
            .map(|i| PropertyStat {
                id: format!("module.component.property_{i:04}"),
                msg: "m".repeat(512),
                ..one.clone()
            })
            .collect();
        RunOutcome::passed(1, 1, 1).with_properties(stats, summary)
    }

    #[test]
    fn the_completion_body_names_the_trial_count_and_carries_no_records() {
        let report = hairspring_engine::trial_log::TrialReport {
            seq: 0,
            seed: 1,
            verdict: hairspring_engine::trial_log::ReportedVerdict::Pass,
            reason: String::new(),
            exit_code: Some(0),
            signal: None,
            behaviour_hash: None,
            schedule_fp: None,
            syscalls: 0,
            work_ops: 0,
            elapsed_ms: 0,
            catalog: Vec::new(),
            digest: vec![1, 0, 0, 0],
            detail: None,
            output: Some("{}".to_string()),
        };
        let outcome = RunOutcome {
            trials_reported: Some(1),
            ..RunOutcome::passed(1, 1, 1).with_trial_log(vec![report])
        };
        let text = completion_body("w", "s", outcome).to_string();
        assert!(text.contains("\"trials_reported\":1"), "{text}");
        assert!(
            !text.contains("trial_log") && !text.contains("digest"),
            "{text}"
        );
    }

    #[test]
    fn the_completion_body_of_a_run_with_the_most_properties_fits_the_server_limit() {
        let outcome = outcome_with_many_properties();
        let unstripped =
            serde_json::to_vec(&json!({ "worker_id": "w", "outcome": &outcome })).unwrap();
        assert!(
            unstripped.len() > SERVER_BODY_LIMIT,
            "the fixture must be big enough to matter: {} bytes",
            unstripped.len()
        );
        let body = serde_json::to_vec(&completion_body("w", "s", outcome)).unwrap();
        assert!(body.len() < SERVER_BODY_LIMIT, "{} bytes", body.len());
        assert!(!String::from_utf8(body).unwrap().contains("properties"));
    }
}

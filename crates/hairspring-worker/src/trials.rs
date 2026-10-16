//! Sending a run's per-trial records to the server in batches that fit its request limits.
use hairspring_engine::trial_log::TrialReport;

/// Most records in one request (the server refuses more).
pub const MAX_TRIALS_PER_REQUEST: usize = 256;
/// Most bytes of JSON for the records in one request. The server's body limit is 2 MiB; this leaves room for the
/// envelope.
pub const MAX_REQUEST_RECORD_BYTES: usize = 1536 * 1024;

/// Splits `reports` into consecutive runs that each fit one request. A single record is never split; one that is
/// alone over the limit is sent alone and left for the server to refuse.
pub fn request_batches(reports: &[TrialReport]) -> Vec<&[TrialReport]> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0usize, 0usize);
    for (i, r) in reports.iter().enumerate() {
        // Serialising can only fail for a map with non-string keys; a record has none.
        let size = serde_json::to_vec(r).map_or(0, |v| v.len() + 1);
        if i > start
            && (i - start >= MAX_TRIALS_PER_REQUEST || bytes + size > MAX_REQUEST_RECORD_BYTES)
        {
            out.push(&reports[start..i]);
            (start, bytes) = (i, 0);
        }
        bytes += size;
    }
    if start < reports.len() {
        out.push(&reports[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_engine::trial_log::ReportedVerdict;

    fn report(seq: u64, detail_bytes: usize) -> TrialReport {
        TrialReport {
            seq,
            seed: seq,
            verdict: ReportedVerdict::Pass,
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
            detail: (detail_bytes > 0).then(|| "x".repeat(detail_bytes)),
            output: None,
        }
    }

    fn lengths(reports: &[TrialReport]) -> Vec<usize> {
        request_batches(reports).iter().map(|b| b.len()).collect()
    }

    #[test]
    fn small_records_are_batched_by_count() {
        let reports: Vec<_> = (0..600).map(|i| report(i, 0)).collect();
        assert_eq!(lengths(&reports), vec![256, 256, 88]);
    }

    #[test]
    fn big_records_are_batched_by_size_and_every_batch_fits() {
        let reports: Vec<_> = (0..20).map(|i| report(i, 200 * 1024)).collect();
        let batches = request_batches(&reports);
        assert!(batches.len() > 1);
        for b in &batches {
            let body = serde_json::to_vec(
                &serde_json::json!({"worker_id": "wrk_0123456789abcdef", "trials": b}),
            )
            .unwrap();
            assert!(body.len() < 2 * 1024 * 1024, "{} bytes", body.len());
        }
        assert_eq!(batches.iter().map(|b| b.len()).sum::<usize>(), 20);
    }

    #[test]
    fn order_is_kept_and_nothing_is_lost_or_repeated() {
        let reports: Vec<_> = (0..700)
            .map(|i| report(i, if i % 50 == 0 { 100 * 1024 } else { 0 }))
            .collect();
        let seqs: Vec<u64> = request_batches(&reports)
            .iter()
            .flat_map(|b| b.iter().map(|r| r.seq))
            .collect();
        assert_eq!(seqs, (0..700).collect::<Vec<_>>());
    }

    #[test]
    fn no_records_means_no_requests() {
        assert!(request_batches(&[]).is_empty());
    }
}

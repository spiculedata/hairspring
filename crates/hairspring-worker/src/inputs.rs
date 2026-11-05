//! Reports the inputs of a job to the server before the job runs: one digest per input the engine's backend can
//! describe. Best effort. A server that predates the endpoint, or a refusal, leaves the run without recorded inputs,
//! which readers see as "not recorded"; it never stops the job.
use crate::{Api, CallError};
use hairspring_engine::engine::Engine;
use hairspring_service::{LeasedJob, RunInputsSubmission};
use serde_json::json;
use std::sync::atomic::Ordering;

/// What an older server answers to a path it does not have.
const NOT_THERE: [u16; 2] = [404, 405];

pub(crate) fn report(api: &Api, engine: &Engine, worker_id: &str, secret: &str, job: &LeasedJob) {
    if api.inputs_unsupported.load(Ordering::Relaxed) {
        return;
    }
    let Some(draft) = engine.run_inputs(&job.target) else {
        return;
    };
    let inputs = RunInputsSubmission::from_hashes(&draft.input_hashes());
    let body = json!({ "worker_id": worker_id, "worker_secret": secret, "inputs": inputs });
    let path = format!("/api/worker/jobs/{}/inputs", job.job_id);
    // Only fixed words and the status are printed: nothing the server sent back is repeated.
    match api.post(&path, &body) {
        Ok(_) => {}
        Err(CallError::Status(code, _)) if NOT_THERE.contains(&code) => {
            api.inputs_unsupported.store(true, Ordering::Relaxed);
            eprintln!(
                "hairspring-worker: this server does not record run inputs (it answered {code}); runs of this worker have none recorded"
            );
        }
        Err(CallError::Status(code, _)) => eprintln!(
            "hairspring-worker: the server answered {code} to the inputs of job {}; the run has none recorded",
            job.job_id
        ),
        Err(CallError::Transport(_)) => eprintln!(
            "hairspring-worker: could not reach the server with the inputs of job {}; the run has none recorded",
            job.job_id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkerConfig;
    use hairspring_engine::config::Config;
    use hairspring_engine::mode::RunMode;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// A server that answers every request with `status` and counts them; it reads each request to its end.
    fn stub(status: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut seen = Vec::new();
                let mut block = [0u8; 4096];
                loop {
                    let n = stream.read(&mut block).unwrap_or(0);
                    seen.extend_from_slice(&block[..n]);
                    let text = String::from_utf8_lossy(&seen).to_string();
                    let Some(head_end) = text.find("\r\n\r\n") else {
                        if n == 0 {
                            break;
                        }
                        continue;
                    };
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if seen.len() >= head_end + 4 + length || n == 0 {
                        break;
                    }
                }
                counted.fetch_add(1, Ordering::SeqCst);
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (url, hits)
    }

    fn engine() -> Engine {
        let registry = Arc::new(hairspring_engine::demos::default_registry());
        Engine::new(registry.clone(), Config::from_registry(&registry))
    }

    fn job(n: u32, target: &str) -> LeasedJob {
        LeasedJob {
            job_id: format!("job_{n:016x}"),
            run_id: format!("run_{n:016x}"),
            target: target.into(),
            seed: 1,
            budget: None,
            mode: RunMode::Search,
        }
    }

    fn api(url: &str) -> Api {
        Api::new(&WorkerConfig::new(url, "test")).unwrap()
    }

    #[test]
    fn a_job_whose_target_has_inputs_is_reported_once_per_job() {
        let (url, hits) = stub("200 OK");
        let engine = engine();
        let target = engine.target_names().into_iter().next().unwrap();
        let api = api(&url);
        report(
            &api,
            &engine,
            "wrk_0000000000000001",
            "secret",
            &job(1, &target),
        );
        report(
            &api,
            &engine,
            "wrk_0000000000000001",
            "secret",
            &job(2, &target),
        );
        assert_eq!(hits.load(Ordering::SeqCst), 2, "one report for each job");
    }

    #[test]
    fn a_target_the_engine_cannot_describe_sends_nothing() {
        let (url, hits) = stub("200 OK");
        report(
            &api(&url),
            &engine(),
            "wrk_0000000000000001",
            "s",
            &job(1, "no-such-target"),
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_server_without_the_endpoint_is_asked_once_and_then_left_alone() {
        let (url, hits) = stub("404 Not Found");
        let engine = engine();
        let target = engine.target_names().into_iter().next().unwrap();
        let api = api(&url);
        for n in 1..=4 {
            report(&api, &engine, "wrk_0000000000000001", "s", &job(n, &target));
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "remembered after the first 404"
        );
        assert!(api.inputs_unsupported.load(Ordering::SeqCst));
    }

    #[test]
    fn a_refusal_that_is_not_a_missing_endpoint_does_not_stop_later_reports() {
        let (url, hits) = stub("400 Bad Request");
        let engine = engine();
        let target = engine.target_names().into_iter().next().unwrap();
        let api = api(&url);
        for n in 1..=3 {
            report(&api, &engine, "wrk_0000000000000001", "s", &job(n, &target));
        }
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        assert!(!api.inputs_unsupported.load(Ordering::SeqCst));
    }
}

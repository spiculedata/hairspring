//! Remote worker: pulls jobs from an Anthill server, runs them on the local engine
//! (which may have backends the server lacks, such as Hermit on a Linux box) and
//! reports progress and results back.
use anthill_engine::backend::Progress;
use anthill_engine::engine::{Engine, RunRequest};
use anthill_engine::report::RunOutcome;
use anthill_service::LeasedJob;
use serde_json::{json, Value};
use std::cell::Cell;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
const RESULT_RETRY_WINDOW: Duration = Duration::from_secs(120);
const TICK: Duration = Duration::from_millis(25);
const STATUS_TOO_MANY_REQUESTS: u16 = 429;

#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub server: String,
    pub token: Option<String>,
    pub name: String,
    /// How long to wait before asking again when the queue is empty.
    pub poll: Duration,
    /// How often to renew the lease while a run is silent (must be well under the server's lease).
    pub heartbeat: Duration,
}

impl WorkerConfig {
    pub fn new(server: impl Into<String>, name: impl Into<String>) -> WorkerConfig {
        WorkerConfig {
            server: server.into(),
            token: None,
            name: name.into(),
            poll: Duration::from_secs(1),
            heartbeat: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub enum WorkerError {
    /// Credentials were rejected; retrying cannot help.
    Unauthorized,
    Config(String),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::Unauthorized => write!(f, "the server rejected this worker's token"),
            WorkerError::Config(m) => write!(f, "invalid worker configuration: {m}"),
        }
    }
}

impl std::error::Error for WorkerError {}

/// A reply that is not a success.
enum CallError {
    Transport(String),
    Status(u16, String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Transport(m) => write!(f, "cannot reach the server: {m}"),
            CallError::Status(code, body) => write!(
                f,
                "server answered {code}: {}",
                body.chars().take(200).collect::<String>()
            ),
        }
    }
}

impl CallError {
    /// Worth retrying: the network failed, the server is busy or broken.
    fn transient(&self) -> bool {
        match self {
            CallError::Transport(_) => true,
            CallError::Status(code, _) => *code == STATUS_TOO_MANY_REQUESTS || *code >= 500,
        }
    }

    fn is_unknown_worker(&self) -> bool {
        matches!(self, CallError::Status(400, body) if body.contains("unknown worker"))
    }
}

struct Api {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
}

impl Api {
    fn new(cfg: &WorkerConfig) -> Result<Api, WorkerError> {
        if !(cfg.server.starts_with("http://") || cfg.server.starts_with("https://")) {
            return Err(WorkerError::Config(
                "server must start with http:// or https://".to_string(),
            ));
        }
        let loopback = ["//127.0.0.1", "//localhost", "//[::1]"]
            .iter()
            .any(|h| cfg.server.contains(h));
        if cfg.server.starts_with("http://") && !loopback && cfg.token.is_some() {
            eprintln!("anthill-worker: warning: the API token is sent over plain http to a non-local server; use https or a private network");
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build()
            .into();
        let token = cfg.token.clone().filter(|t| !t.trim().is_empty());
        Ok(Api {
            agent,
            base: cfg.server.trim_end_matches('/').to_string(),
            token,
        })
    }

    fn post(&self, path: &str, body: &Value) -> Result<(u16, String), CallError> {
        let mut req = self
            .agent
            .post(format!("{}{}", self.base, path))
            .header("Content-Type", "application/json");
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let mut resp = req
            .send(body.to_string())
            .map_err(|e| CallError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        match status {
            200..=299 => Ok((status, text)),
            s => Err(CallError::Status(s, text)),
        }
    }

    /// Retries transient failures with backoff for up to `RESULT_RETRY_WINDOW`; a finished result is worth keeping.
    fn post_reliably(
        &self,
        path: &str,
        body: &Value,
        stop: &AtomicBool,
    ) -> Result<(u16, String), CallError> {
        let deadline = Instant::now() + RESULT_RETRY_WINDOW;
        let mut backoff = Duration::from_millis(200);
        loop {
            match self.post(path, body) {
                Ok(r) => return Ok(r),
                Err(e)
                    if e.transient()
                        && Instant::now() < deadline
                        && !stop.load(Ordering::SeqCst) =>
                {
                    eprintln!("anthill-worker: {e}; retrying");
                    sleep_unless_stopped(stop, backoff);
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

fn check_auth(e: &CallError) -> Result<(), WorkerError> {
    match e {
        CallError::Status(401, _) | CallError::Status(403, _) => Err(WorkerError::Unauthorized),
        _ => Ok(()),
    }
}

fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) {
    let mut waited = Duration::ZERO;
    while waited < total && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        waited += TICK;
    }
}

fn register(api: &Api, cfg: &WorkerConfig, stop: &AtomicBool) -> Result<String, WorkerError> {
    let mut backoff = Duration::from_millis(200);
    loop {
        match api.post(
            "/api/worker/register",
            &json!({ "name": cfg.name, "caps": { "backends": ["sim"] } }),
        ) {
            Ok((_, body)) => {
                if let Some(id) = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| v["worker_id"].as_str().map(str::to_string))
                {
                    return Ok(id);
                }
            }
            Err(e) => {
                check_auth(&e)?;
                eprintln!("anthill-worker: registration failed, retrying: {e}");
            }
        }
        if stop.load(Ordering::SeqCst) {
            return Err(WorkerError::Config(
                "stopped before the worker could register".to_string(),
            ));
        }
        sleep_unless_stopped(stop, backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn execute(
    api: &Api,
    engine: &Engine,
    worker_id: &str,
    job: &LeasedJob,
    heartbeat: Duration,
    stop: &AtomicBool,
) -> Result<(), WorkerError> {
    let job_path = |action: &str| format!("/api/worker/jobs/{}/{action}", job.job_id);
    let req = RunRequest {
        target: job.target.clone(),
        seed: job.seed,
        budget: job.budget,
    };
    let interrupted = Cell::new(false);
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut last = Instant::now();
            while !done.load(Ordering::SeqCst) {
                std::thread::sleep(TICK);
                if last.elapsed() >= heartbeat {
                    let _ = api.post(&job_path("heartbeat"), &json!({ "worker_id": worker_id }));
                    last = Instant::now();
                }
            }
        });
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            engine.run(&req, &mut |p: Progress| {
                if stop.load(Ordering::SeqCst) {
                    interrupted.set(true);
                    return false;
                }
                let body = json!({ "worker_id": worker_id, "steps": p.steps, "trials": p.trials, "message": p.message });
                // Best effort; the reply tells us if the run was cancelled meanwhile.
                match api.post(&job_path("progress"), &body) {
                    Ok((_, text)) => serde_json::from_str::<Value>(&text).ok().and_then(|v| v["continue"].as_bool()).unwrap_or(true),
                    Err(_) => true,
                }
            })
        }));
        done.store(true, Ordering::SeqCst);
        outcome
    });
    let (path, body) = match result {
        Ok(Ok(_)) if interrupted.get() => (
            job_path("fail"),
            json!({ "worker_id": worker_id, "error": "interrupted by worker shutdown" }),
        ),
        Ok(Ok(outcome)) => (
            job_path("complete"),
            json!({ "worker_id": worker_id, "outcome": outcome }),
        ),
        Ok(Err(e)) => (
            job_path("complete"),
            json!({ "worker_id": worker_id, "outcome": RunOutcome::error(e.to_string()) }),
        ),
        Err(_) => (
            job_path("fail"),
            json!({ "worker_id": worker_id, "error": "worker panicked while running the target" }),
        ),
    };
    if let Err(e) = api.post_reliably(&path, &body, &AtomicBool::new(false)) {
        check_auth(&e)?;
        // The lease may have expired and been reassigned; nothing more we can do for this job.
        eprintln!(
            "anthill-worker: could not report the result of job {}: {e}",
            job.job_id
        );
    }
    Ok(())
}

/// Registers with the server and processes jobs until `stop` is set.
/// Transient network errors are retried, a forgotten worker re-registers,
/// and rejected credentials end the loop.
pub fn run_loop(engine: &Engine, cfg: &WorkerConfig, stop: &AtomicBool) -> Result<(), WorkerError> {
    let api = Api::new(cfg)?;
    let mut worker_id = register(&api, cfg, stop)?;
    let mut backoff = Duration::from_millis(200);
    while !stop.load(Ordering::SeqCst) {
        match api.post("/api/worker/lease", &json!({ "worker_id": worker_id })) {
            Ok((204, _)) => sleep_unless_stopped(stop, cfg.poll),
            Ok((_, body)) => match serde_json::from_str::<LeasedJob>(&body) {
                Ok(job) => {
                    backoff = Duration::from_millis(200);
                    execute(&api, engine, &worker_id, &job, cfg.heartbeat, stop)?;
                }
                Err(e) => {
                    eprintln!("anthill-worker: ignoring an unreadable job from the server: {e}");
                    sleep_unless_stopped(stop, cfg.poll);
                }
            },
            Err(e) if e.is_unknown_worker() => {
                eprintln!(
                    "anthill-worker: the server no longer knows this worker; registering again"
                );
                worker_id = register(&api, cfg, stop)?;
            }
            Err(e) => {
                check_auth(&e)?;
                eprintln!("anthill-worker: lease request failed, retrying: {e}");
                sleep_unless_stopped(stop, backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
    Ok(())
}

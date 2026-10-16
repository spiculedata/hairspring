//! Hairspring server: REST API, SSE, MCP endpoint, bearer-token auth and the embedded web UI.
mod api;
mod auth;
mod error;
mod ratelimit;
mod read_api;
mod sse;
mod ui;
mod worker_api;

pub use auth::Auth;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use hairspring_engine::engine::Engine;
use hairspring_mcp::McpServer;
use hairspring_service::Service;
use ratelimit::RateLimiter;
use sha2::{Digest, Sha256};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Request bodies larger than this are rejected with 413.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Default cap on concurrent event streams.
const DEFAULT_MAX_SSE_STREAMS: usize = 64;
/// Default registrations per token per window: far above a fleet restarting, far below the worker cap.
const DEFAULT_REGISTER_LIMIT: u32 = 100;
const DEFAULT_REGISTER_WINDOW: Duration = Duration::from_secs(3600);
/// Cap on threads used for blocking storage and engine work.
const MAX_BLOCKING_THREADS: usize = 128;
const WORKER_IDLE: Duration = Duration::from_millis(200);
const WORKER_TOUCH: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub auth: Auth,
    /// Worker threads inside this process that execute queued runs.
    pub embedded_workers: usize,
    /// Requests per minute per caller; 0 disables limiting.
    pub rate_limit_per_min: u32,
    /// Concurrent server-sent-event streams allowed in total.
    pub max_sse_streams: usize,
    /// Workers one token may register per [`ServerConfig::register_window`]; 0 disables the limit. A token that
    /// can register without bound could fill the server's worker cap. Refused registrations get a 429 with
    /// `Retry-After` set to the window.
    pub register_limit: u32,
    /// The span `register_limit` is counted over. Counts are held in memory, so they start again when the server
    /// restarts.
    pub register_window: Duration,
    /// Shut down on Ctrl-C (for the CLI).
    pub handle_ctrl_c: bool,
}

impl ServerConfig {
    pub fn new(addr: SocketAddr, auth: Auth) -> ServerConfig {
        ServerConfig {
            addr,
            auth,
            embedded_workers: 1,
            rate_limit_per_min: 1200,
            max_sse_streams: DEFAULT_MAX_SSE_STREAMS,
            register_limit: DEFAULT_REGISTER_LIMIT,
            register_window: DEFAULT_REGISTER_WINDOW,
            handle_ctrl_c: false,
        }
    }

    fn validate(&self) -> io::Result<()> {
        if !self.addr.ip().is_loopback() && self.auth.is_open() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to listen on a non-loopback address without API tokens; configure at least one token",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct AppState {
    pub svc: Service,
    pub engine: Engine,
    pub mcp: Arc<McpServer>,
    pub auth: Arc<Auth>,
    pub stopping: Arc<AtomicBool>,
    limiter: Arc<RateLimiter>,
    /// Counts worker registrations per token.
    pub register_limiter: Arc<RateLimiter>,
    /// What a refused registration is told to wait: the whole window, the longest it can take to clear.
    pub register_retry_after_secs: u64,
    pub sse_slots: Arc<tokio::sync::Semaphore>,
}

/// Handle to a running server. Dropping it shuts the server down.
pub struct RunningServer {
    addr: SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    workers: Vec<JoinHandle<()>>,
}

impl RunningServer {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Blocks until the server stops (Ctrl-C or `shutdown`).
    pub fn wait(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.stop();
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn digest_prefix(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Who a request counts against. A valid token gets its own bucket; everything else (no token, a wrong
/// token, a made-up token) shares one bucket per client address, so guessing cannot dodge the limit.
fn limit_key(state: &AppState, peer: SocketAddr, authorization: Option<&str>) -> String {
    if !state.auth.is_open() {
        if let Some((_, key)) = state.auth.identify(authorization) {
            return key;
        }
    }
    ratelimit::anonymous_key(peer.ip())
}

async fn limit_requests(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let limited = (path.starts_with("/api/") && path != "/api/health") || path == "/mcp";
    if limited {
        let authorization = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        if !s.limiter.check(&limit_key(&s, peer, authorization)) {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, "60")],
                "{\"error\":\"rate limit exceeded\"}",
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// `host` or `host:port` -> true if it names a loopback address.
fn is_loopback_host(authority: &str) -> bool {
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Without tokens the server trusts anything that can reach it, so it must not be drivable from a
/// web page: reject unknown Host headers (DNS rebinding) and cross-site Origins (CSRF).
async fn guard_open_mode(State(s): State<AppState>, req: Request, next: Next) -> Response {
    if s.auth.is_open() {
        let headers = req.headers();
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let origin_ok = match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            None => true,
            Some(o) => o
                .strip_prefix("http://")
                .or_else(|| o.strip_prefix("https://"))
                .is_some_and(|a| a.eq_ignore_ascii_case(host)),
        };
        if !is_loopback_host(host) || !origin_ok {
            return (StatusCode::FORBIDDEN, "{\"error\":\"forbidden: this server has no API tokens and only accepts same-origin local requests\"}").into_response();
        }
    }
    next.run(req).await
}

async fn harden(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/") || req.uri().path() == "/mcp";
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    res
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(api::health))
        .route("/api/targets", get(api::targets))
        .route("/api/runs", get(api::list_runs).post(api::create_run))
        .route("/api/runs/{id}", get(api::get_run))
        .route("/api/runs/{id}/cancel", post(api::cancel_run))
        .route("/api/runs/{id}/events", get(sse::events))
        .route("/api/runs/{id}/properties", get(read_api::properties))
        .route("/api/runs/{id}/properties/{idx}", get(read_api::property))
        .route("/api/runs/{id}/trials", get(read_api::trials))
        .route("/api/runs/{id}/trials/{seq}", get(read_api::trial))
        .route(
            "/api/runs/{id}/trials/{seq}/output/{stream}",
            get(read_api::trial_output),
        )
        .route("/api/runs/{id}/strip", get(read_api::strip))
        .route("/api/runs/{id}/coverage", get(read_api::coverage))
        .route("/api/failures", get(api::list_failures))
        .route("/api/failures/{id}", get(api::get_failure))
        .route("/api/bundles/{hash}", get(api::get_bundle))
        .route("/api/replay", post(api::replay))
        .route("/api/workers", get(api::workers))
        .route("/api/workers/{id}/trust", post(api::set_worker_trust))
        .route("/api/worker/register", post(worker_api::register))
        .route("/api/worker/lease", post(worker_api::lease))
        .route(
            "/api/worker/jobs/{job}/progress",
            post(worker_api::progress),
        )
        .route(
            "/api/worker/jobs/{job}/complete",
            post(worker_api::complete),
        )
        .route("/api/worker/jobs/{job}/fail", post(worker_api::fail))
        .route("/api/worker/jobs/{job}/trials", post(worker_api::trials))
        .route(
            "/api/worker/jobs/{job}/heartbeat",
            post(worker_api::heartbeat),
        )
        .route("/mcp", post(api::mcp))
        .fallback(ui::handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            guard_open_mode,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_requests,
        ))
        .layer(middleware::from_fn(harden))
        .with_state(state)
}

fn spawn_workers(
    n: usize,
    svc: &Service,
    engine: &Engine,
    stop: &Arc<AtomicBool>,
) -> Vec<JoinHandle<()>> {
    (0..n)
        .filter_map(|i| {
            let caps = "{\"backends\":[\"embedded\"]}";
            let worker = match svc.register_local_worker(&format!("embedded-{i}"), caps) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("hairspring-server: cannot register embedded worker {i}: {e}");
                    return None;
                }
            };
            let (svc, engine, stop) = (svc.clone(), engine.clone(), stop.clone());
            std::thread::Builder::new()
                .name(format!("hairspring-worker-{i}"))
                .spawn(move || {
                    let mut last_touch = Instant::now();
                    while !stop.load(Ordering::SeqCst) {
                        match svc.execute_next(&engine, &worker.id) {
                            Ok(true) => continue,
                            Ok(false) => std::thread::sleep(WORKER_IDLE),
                            Err(e) => {
                                eprintln!("hairspring-server: embedded worker error: {e}");
                                std::thread::sleep(Duration::from_secs(1));
                            }
                        }
                        if last_touch.elapsed() > WORKER_TOUCH {
                            let _ = svc.touch_worker(&worker.id);
                            last_touch = Instant::now();
                        }
                    }
                })
                .ok()
        })
        .collect()
}

/// Binds the listener and starts serving on a background thread.
pub fn start(config: ServerConfig, svc: Service, engine: Engine) -> io::Result<RunningServer> {
    config.validate()?;
    let listener = std::net::TcpListener::bind(config.addr)?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let stopping = Arc::new(AtomicBool::new(false));
    let state = AppState {
        svc: svc.clone(),
        engine: engine.clone(),
        mcp: Arc::new(McpServer::new(svc.clone(), engine.clone(), false)),
        auth: Arc::new(config.auth.clone()),
        stopping: stopping.clone(),
        limiter: Arc::new(RateLimiter::new(config.rate_limit_per_min)),
        register_limiter: Arc::new(RateLimiter::with_window(
            config.register_limit,
            config.register_window,
        )),
        register_retry_after_secs: config.register_window.as_secs().max(1),
        sse_slots: Arc::new(tokio::sync::Semaphore::new(config.max_sse_streams)),
    };
    let workers = spawn_workers(config.embedded_workers, &svc, &engine, &stopping);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let ctrl_c = config.handle_ctrl_c;
    let thread = std::thread::Builder::new().name("hairspring-server".to_string()).spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(4).max_blocking_threads(MAX_BLOCKING_THREADS).enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("hairspring-server: cannot start the async runtime: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("hairspring-server: cannot use the listener: {e}");
                    return;
                }
            };
            let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
            let shutdown = async move {
                tokio::select! {
                    _ = rx => {}
                    _ = async { if ctrl_c { let _ = tokio::signal::ctrl_c().await; } else { std::future::pending::<()>().await } } => {}
                }
            };
            if let Err(e) = axum::serve(listener, app).with_graceful_shutdown(shutdown).await {
                eprintln!("hairspring-server: server error: {e}");
            }
        });
    })?;
    Ok(RunningServer {
        addr,
        shutdown: Some(tx),
        stopping,
        thread: Some(thread),
        workers,
    })
}

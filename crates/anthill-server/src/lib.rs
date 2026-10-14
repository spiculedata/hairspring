//! Anthill server: REST API, SSE, MCP endpoint, bearer-token auth and the embedded web UI.
mod api;
mod auth;
mod error;
mod ratelimit;
mod sse;
mod ui;
mod worker_api;

pub use auth::Auth;

use anthill_engine::engine::Engine;
use anthill_mcp::McpServer;
use anthill_service::Service;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
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

async fn limit_requests(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let limited = (path.starts_with("/api/") && path != "/api/health") || path == "/mcp";
    if limited {
        let key = match req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        {
            Some(h) => format!("t:{}", digest_prefix(h)),
            None => format!("ip:{}", peer.ip()),
        };
        if !s.limiter.check(&key) {
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
        .route("/api/failures", get(api::list_failures))
        .route("/api/failures/{id}", get(api::get_failure))
        .route("/api/bundles/{hash}", get(api::get_bundle))
        .route("/api/replay", post(api::replay))
        .route("/api/workers", get(api::workers))
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
        .route(
            "/api/worker/jobs/{job}/heartbeat",
            post(worker_api::heartbeat),
        )
        .route("/mcp", post(api::mcp))
        .fallback(ui::handler)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
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
            let worker = match svc.register_worker(&format!("embedded-{i}"), caps) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("anthill-server: cannot register embedded worker {i}: {e}");
                    return None;
                }
            };
            let (svc, engine, stop) = (svc.clone(), engine.clone(), stop.clone());
            std::thread::Builder::new()
                .name(format!("anthill-worker-{i}"))
                .spawn(move || {
                    let mut last_touch = Instant::now();
                    while !stop.load(Ordering::SeqCst) {
                        match svc.execute_next(&engine, &worker.id) {
                            Ok(true) => continue,
                            Ok(false) => std::thread::sleep(WORKER_IDLE),
                            Err(e) => {
                                eprintln!("anthill-server: embedded worker error: {e}");
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
    };
    let workers = spawn_workers(config.embedded_workers, &svc, &engine, &stopping);
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let ctrl_c = config.handle_ctrl_c;
    let thread = std::thread::Builder::new().name("anthill-server".to_string()).spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("anthill-server: cannot start the async runtime: {e}");
                return;
            }
        };
        rt.block_on(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("anthill-server: cannot use the listener: {e}");
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
                eprintln!("anthill-server: server error: {e}");
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

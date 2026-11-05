//! Hairspring server: REST API, SSE, MCP endpoint, bearer-token auth and the embedded web UI.
mod api;
mod auth;
mod error;
mod ndjson;
mod organisation;
mod ratelimit;
mod read_api;
mod sse;
mod ui;
mod worker_api;

pub use auth::Auth;
pub use organisation::{parse_organisation, MAX_ORGANISATION_CHARS};

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
    /// The organisation's name, shown in the web UI's top bar when set. Plain text of at most
    /// [`MAX_ORGANISATION_CHARS`] characters; a blank name is none. `GET /api/health` serves it, and only to a caller
    /// the server would answer (a valid token, or any caller when there are no tokens).
    pub organisation: Option<String>,
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
            organisation: None,
        }
    }

    /// The name to serve: [`ServerConfig::organisation`] checked and trimmed, or `None`.
    fn organisation_name(&self) -> io::Result<Option<Arc<str>>> {
        let Some(raw) = &self.organisation else {
            return Ok(None);
        };
        parse_organisation(raw)
            .map(|name| name.map(Arc::from))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }

    fn validate(&self) -> io::Result<()> {
        self.organisation_name()?;
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
    /// The checked organisation name, if one is configured.
    pub organisation: Option<Arc<str>>,
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

/// Whether a limited request may go ahead. Anything without a valid token (no token, a wrong token, a made-up
/// token) counts against one bucket per client address (the address for IPv4, the /64 for IPv6, taken from the
/// connection and never from forwarding headers), so guessing cannot dodge the limit. A valid token counts
/// against its own bucket, and is also refused, without spending either budget, while its client address's
/// anonymous bucket is over the limit: otherwise the right guess would be told apart from the wrong ones by its
/// 200, and the limit would not slow guessing at all. The windows are fixed and a refused request still counts
/// against the anonymous bucket's window, so valid tokens from that address are refused for as long as such
/// traffic keeps the bucket over the limit, and for up to one window (a minute) after it stops. That is the
/// trade-off of limiting guessing per address; behind a shared address (NAT, a proxy, an IPv6 /64) it affects
/// every caller there. Any other code that counts a request against `limiter` should call this rather than
/// `check`, so a valid token never spends its address's anonymous budget and locks other tokens on that address
/// out.
fn admit(state: &AppState, peer: SocketAddr, authorization: Option<&str>) -> bool {
    let anonymous = ratelimit::anonymous_key(peer.ip());
    let token = if state.auth.is_open() {
        None
    } else {
        state.auth.identify(authorization)
    };
    match token {
        Some((_, key)) => !state.limiter.is_over(&anonymous) && state.limiter.check(&key),
        None => state.limiter.check(&anonymous),
    }
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
        if !admit(&s, peer, authorization) {
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
        .route("/api/me", get(api::me))
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
        .route("/api/failures/{id}/checks", get(read_api::failure_checks))
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
        organisation: config.organisation_name()?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use hairspring_engine::config::Config;
    use hairspring_engine::demos::default_registry;
    use hairspring_store::{SqliteStore, Store};

    const TOKEN: &str = "read-token-0123456789abcdef";
    const LIMIT: u32 = 2;
    const WINDOW: Duration = Duration::from_millis(600);

    fn state_with_window(window: Duration) -> AppState {
        let registry = Arc::new(default_registry());
        let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
        let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
        let svc = Service::new(store);
        AppState {
            svc: svc.clone(),
            engine: engine.clone(),
            mcp: Arc::new(McpServer::new(svc, engine, false)),
            auth: Arc::new(Auth::from_spec(&format!("{TOKEN}:read")).unwrap()),
            stopping: Arc::new(AtomicBool::new(false)),
            limiter: Arc::new(RateLimiter::with_window(LIMIT, window)),
            register_limiter: Arc::new(RateLimiter::new(0)),
            register_retry_after_secs: 1,
            sse_slots: Arc::new(tokio::sync::Semaphore::new(1)),
            organisation: None,
        }
    }

    #[test]
    fn a_valid_token_refused_because_of_its_address_keeps_its_own_allowance() {
        // Arrange: the address's anonymous bucket goes over the limit, and the valid token is refused for it.
        let state = state_with_window(WINDOW);
        let peer: SocketAddr = "192.0.2.7:4000".parse().unwrap();
        let bearer = format!("Bearer {TOKEN}");
        for _ in 0..=LIMIT {
            admit(&state, peer, Some("Bearer wrong-guess"));
        }
        // Refuse the token part-way through the address's window, so that if these requests did spend the
        // token's own allowance, that window would still be open after the address's has ended.
        std::thread::sleep(WINDOW / 2);
        for _ in 0..LIMIT + 3 {
            assert!(
                !admit(&state, peer, Some(&bearer)),
                "refused while the address is over the limit"
            );
        }

        // Act: wait until only the address's window has ended, then use the token.
        std::thread::sleep(WINDOW * 2 / 3);
        let allowed = (0..LIMIT + 3)
            .filter(|_| admit(&state, peer, Some(&bearer)))
            .count();

        // Assert: the refused requests did not spend the token's own allowance.
        assert_eq!(allowed, LIMIT as usize);
    }
}

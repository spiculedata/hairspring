//! The fake Dapr sidecar: state store and publish endpoints with seeded fault injection and bounded memory.
use crate::faults::{chance, FaultPlan};
use crate::http::{serve, Request, Response};
use anthill_core::rng::Rng;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MAX_NAME_LEN: usize = 64;
const MAX_KEY_LEN: usize = 256;

/// Bounds on what an app (or anything that can reach the loopback port) can make the sidecar hold.
#[derive(Debug, Clone, Copy)]
pub struct SidecarLimits {
    pub max_keys_per_store: usize,
    pub max_stores: usize,
    /// Total bytes of keys and values across all stores.
    pub max_total_bytes: usize,
    /// Messages the app may have published but not yet been delivered.
    pub max_published: usize,
}

impl Default for SidecarLimits {
    fn default() -> Self {
        SidecarLimits {
            max_keys_per_store: 10_000,
            max_stores: 64,
            max_total_bytes: 16 * 1024 * 1024,
            max_published: 10_000,
        }
    }
}

type Store = HashMap<String, (String, u64)>;

struct Shared {
    rng: Rng,
    faults: FaultPlan,
    limits: SidecarLimits,
    stores: HashMap<String, Store>,
    total_bytes: usize,
    next_etag: u64,
    published: Vec<(String, String, Value)>,
}

pub struct SidecarHandle {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
}

impl SidecarHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Messages the app published through the sidecar since the last call.
    pub fn take_published(&self) -> Vec<(String, String, Value)> {
        std::mem::take(
            &mut self
                .shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .published,
        )
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME_LEN
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn error(status: u16, code: &str, msg: &str) -> Response {
    Response::json(status, &json!({ "errorCode": code, "message": msg }))
}

fn limit_error(msg: &str) -> Response {
    error(413, "ERR_STATE_LIMIT", msg)
}

/// Decodes `%XX` escapes; `None` for a malformed escape or non-UTF-8 result.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn is_loopback_authority(authority: &str) -> bool {
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

/// The sidecar speaks to the app on loopback. A browser page (Origin) or a rebinding attack (foreign Host) is not the app.
fn caller_is_local(r: &Request) -> bool {
    r.header("origin").is_none() && r.header("host").is_some_and(is_loopback_authority)
}

fn save(shared: &Mutex<Shared>, store: &str, body: &[u8]) -> Response {
    let Ok(Value::Array(items)) = serde_json::from_slice::<Value>(body) else {
        return error(
            400,
            "ERR_MALFORMED_REQUEST",
            "body must be a JSON array of {key, value}",
        );
    };
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    let (err_p, conflict_p) = (s.faults.state_error, s.faults.state_conflict);
    if chance(&mut s.rng, err_p) {
        return error(500, "ERR_STATE_SAVE", "injected state store failure");
    }
    if chance(&mut s.rng, conflict_p) {
        return error(409, "ERR_STATE_SAVE", "injected etag conflict");
    }
    let mut writes = Vec::new();
    for item in &items {
        let Some(key) = item["key"]
            .as_str()
            .filter(|k| !k.is_empty() && k.len() <= MAX_KEY_LEN)
        else {
            return error(
                400,
                "ERR_MALFORMED_REQUEST",
                "every item needs a non-empty `key`",
            );
        };
        let existing = s
            .stores
            .get(store)
            .and_then(|m| m.get(key))
            .map(|(_, e)| *e);
        if let (Some(want), Some(have)) = (item["etag"].as_str(), existing) {
            if want != have.to_string() {
                return error(409, "ERR_STATE_SAVE", "etag mismatch");
            }
        }
        if item["options"]["concurrency"] == "first-write" && existing.is_some() {
            return error(409, "ERR_STATE_SAVE", "key already exists (first-write)");
        }
        writes.push((key.to_string(), item["value"].to_string()));
    }
    if let Some(refusal) = check_limits(&s, store, &writes) {
        return refusal;
    }
    for (key, value) in writes {
        s.next_etag += 1;
        let etag = s.next_etag;
        let old = s
            .stores
            .entry(store.to_string())
            .or_default()
            .insert(key.clone(), (value.clone(), etag));
        s.total_bytes =
            s.total_bytes + key.len() + value.len() - old.map_or(0, |(v, _)| key.len() + v.len());
    }
    Response::empty(204)
}

fn check_limits(s: &Shared, store: &str, writes: &[(String, String)]) -> Option<Response> {
    if !s.stores.contains_key(store) && s.stores.len() >= s.limits.max_stores {
        return Some(limit_error("too many state stores"));
    }
    let existing = s.stores.get(store);
    let mut new_keys = 0usize;
    let mut added = 0usize;
    for (key, value) in writes {
        match existing.and_then(|m| m.get(key)) {
            Some((old, _)) => added += value.len().saturating_sub(old.len()),
            None => {
                new_keys += 1;
                added += key.len() + value.len();
            }
        }
    }
    if existing.map_or(0, |m| m.len()) + new_keys > s.limits.max_keys_per_store {
        return Some(limit_error("too many keys in this state store"));
    }
    if s.total_bytes + added > s.limits.max_total_bytes {
        return Some(limit_error("state store is full"));
    }
    None
}

fn get(shared: &Mutex<Shared>, store: &str, key: &str) -> Response {
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    let p = s.faults.state_error;
    if chance(&mut s.rng, p) {
        return error(500, "ERR_STATE_GET", "injected state store failure");
    }
    match s.stores.get(store).and_then(|m| m.get(key)) {
        None => Response::empty(204),
        Some((value, etag)) => Response {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: value.clone().into_bytes(),
        }
        .with_header("ETag", &etag.to_string()),
    }
}

fn publish(shared: &Mutex<Shared>, pubsub: &str, topic: &str, body: &[u8]) -> Response {
    let Ok(data) = serde_json::from_slice::<Value>(body) else {
        return error(400, "ERR_MALFORMED_REQUEST", "body must be JSON");
    };
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    if s.published.len() >= s.limits.max_published {
        return error(429, "ERR_PUBSUB_LIMIT", "too many undelivered messages");
    }
    s.published
        .push((pubsub.to_string(), topic.to_string(), data));
    Response::empty(204)
}

fn route(shared: &Mutex<Shared>, r: Request) -> Response {
    if !caller_is_local(&r) {
        return error(
            403,
            "ERR_FORBIDDEN",
            "the simulated sidecar only serves local, non-browser callers",
        );
    }
    let Some(segments) = r
        .path
        .trim_start_matches('/')
        .split('/')
        .map(percent_decode)
        .collect::<Option<Vec<String>>>()
    else {
        return error(
            400,
            "ERR_MALFORMED_REQUEST",
            "malformed percent-escape in the path",
        );
    };
    let seg: Vec<&str> = segments.iter().map(String::as_str).collect();
    match (r.method.as_str(), seg.as_slice()) {
        ("GET", ["v1.0", "healthz"]) => Response::empty(204),
        ("POST", ["v1.0", "state", store]) if valid_name(store) => save(shared, store, &r.body),
        ("GET", ["v1.0", "state", store, key])
            if valid_name(store) && !key.is_empty() && key.len() <= MAX_KEY_LEN =>
        {
            get(shared, store, key)
        }
        ("DELETE", ["v1.0", "state", store, key]) if valid_name(store) && !key.is_empty() => {
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            let removed = s.stores.get_mut(*store).and_then(|m| m.remove(*key));
            if let Some((value, _)) = removed {
                s.total_bytes = s.total_bytes.saturating_sub(key.len() + value.len());
            }
            Response::empty(204)
        }
        ("POST", ["v1.0", "publish", pubsub, topic]) if valid_name(pubsub) && valid_name(topic) => {
            publish(shared, pubsub, topic, &r.body)
        }
        (_, ["v1.0", "state", ..]) | (_, ["v1.0", "publish", ..]) => error(
            400,
            "ERR_MALFORMED_REQUEST",
            "invalid state or publish request",
        ),
        _ => Response::text(404, "not found"),
    }
}

/// Starts the fake sidecar on `bind` (use port 0 for an ephemeral port) with default limits.
pub fn start_sidecar(bind: &str, seed: u64, faults: FaultPlan) -> std::io::Result<SidecarHandle> {
    start_sidecar_with(bind, seed, faults, SidecarLimits::default())
}

pub fn start_sidecar_with(
    bind: &str,
    seed: u64,
    faults: FaultPlan,
    limits: SidecarLimits,
) -> std::io::Result<SidecarHandle> {
    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(Shared {
        rng: Rng::new(seed),
        faults,
        limits,
        stores: HashMap::new(),
        total_bytes: 0,
        next_etag: 0,
        published: Vec::new(),
    }));
    let handler_state = shared.clone();
    serve(
        listener,
        Arc::new(move |r| route(&handler_state, r)),
        stop.clone(),
    );
    Ok(SidecarHandle { addr, stop, shared })
}

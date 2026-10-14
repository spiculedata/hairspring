//! The fake Dapr sidecar: state store and publish endpoints with seeded fault injection.
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

type Store = HashMap<String, (String, u64)>;

struct Shared {
    rng: Rng,
    faults: FaultPlan,
    stores: HashMap<String, Store>,
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
    for (key, value) in writes {
        s.next_etag += 1;
        let etag = s.next_etag;
        s.stores
            .entry(store.to_string())
            .or_default()
            .insert(key, (value, etag));
    }
    Response::empty(204)
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

fn route(shared: &Mutex<Shared>, r: Request) -> Response {
    let segments: Vec<&str> = r.path.trim_start_matches('/').split('/').collect();
    match (r.method.as_str(), segments.as_slice()) {
        ("GET", ["v1.0", "healthz"]) => Response::empty(204),
        ("POST", ["v1.0", "state", store]) if valid_name(store) => save(shared, store, &r.body),
        ("GET", ["v1.0", "state", store, key])
            if valid_name(store) && !key.is_empty() && key.len() <= MAX_KEY_LEN =>
        {
            get(shared, store, key)
        }
        ("DELETE", ["v1.0", "state", store, key]) if valid_name(store) && !key.is_empty() => {
            shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .stores
                .get_mut(*store)
                .map(|m| m.remove(*key));
            Response::empty(204)
        }
        ("POST", ["v1.0", "publish", pubsub, topic]) if valid_name(pubsub) && valid_name(topic) => {
            match serde_json::from_slice::<Value>(&r.body) {
                Ok(data) => {
                    shared
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .published
                        .push((pubsub.to_string(), topic.to_string(), data));
                    Response::empty(204)
                }
                Err(_) => error(400, "ERR_MALFORMED_REQUEST", "body must be JSON"),
            }
        }
        (_, ["v1.0", "state", ..]) | (_, ["v1.0", "publish", ..]) => error(
            400,
            "ERR_MALFORMED_REQUEST",
            "invalid state or publish request",
        ),
        _ => Response::text(404, "not found"),
    }
}

/// Starts the fake sidecar on `bind` (use port 0 for an ephemeral port).
pub fn start_sidecar(bind: &str, seed: u64, faults: FaultPlan) -> std::io::Result<SidecarHandle> {
    let listener = TcpListener::bind(bind)?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(Shared {
        rng: Rng::new(seed),
        faults,
        stores: HashMap::new(),
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

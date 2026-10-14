use crate::rpc::Reply;
use crate::validate::{bundle_hash, failure_id, run_id};
use crate::McpServer;
use serde_json::{json, Value};

const SCHEME: &str = "hairspring://";
const LIST_LIMIT: u32 = 20;
const INVALID_PARAMS: i64 = -32602;
const NOT_FOUND: i64 = -32002;

fn internal_error(detail: impl std::fmt::Display) -> Reply {
    eprintln!("hairspring-mcp: internal error: {detail}");
    Reply::Error(-32603, "internal error".to_string())
}

pub fn templates() -> Value {
    json!({ "resourceTemplates": [
        { "uriTemplate": "hairspring://runs/{id}", "name": "Run", "description": "A stored run with its failures", "mimeType": "application/json" },
        { "uriTemplate": "hairspring://failures/{id}", "name": "Failure", "description": "Minimal reproduction steps, bundle and explanation", "mimeType": "application/json" },
        { "uriTemplate": "hairspring://bundles/{hash}", "name": "Bundle", "description": "A replayable reproduction bundle", "mimeType": "text/plain" }
    ] })
}

pub fn list(server: &McpServer) -> Reply {
    let svc = server.service();
    let runs = match svc.list_runs(LIST_LIMIT) {
        Ok(r) => r,
        Err(e) => return internal_error(e),
    };
    let failures = match svc.recent_failures(LIST_LIMIT) {
        Ok(f) => f,
        Err(e) => return internal_error(e),
    };
    let mut items: Vec<Value> = runs
        .iter()
        .map(|r| json!({ "uri": format!("{SCHEME}runs/{}", r.id), "name": format!("run {} ({}, {})", r.id, r.target, r.state.as_str()), "mimeType": "application/json" }))
        .collect();
    items.extend(failures.iter().map(|f| json!({ "uri": format!("{SCHEME}failures/{}", f.id), "name": format!("failure {} ({})", f.id, f.target), "mimeType": "application/json" })));
    Reply::Result(json!({ "resources": items }))
}

pub fn read(server: &McpServer, params: &Value) -> Reply {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return Reply::Error(INVALID_PARAMS, "`uri` is required".to_string());
    };
    let Some(rest) = uri.strip_prefix(SCHEME) else {
        return Reply::Error(
            INVALID_PARAMS,
            format!("unsupported URI scheme (expected {SCHEME})"),
        );
    };
    let Some((kind, id)) = rest.split_once('/') else {
        return Reply::Error(INVALID_PARAMS, "malformed resource URI".to_string());
    };
    let svc = server.service();
    let found = match kind {
        "runs" if run_id(id) => svc.get_run(id).map(|o| {
            o.map(|d| {
                (
                    serde_json::to_string(&d).unwrap_or_default(),
                    "application/json",
                )
            })
        }),
        "failures" if failure_id(id) => svc.get_failure(id).map(|o| {
            o.map(|d| {
                (
                    serde_json::to_string(&d).unwrap_or_default(),
                    "application/json",
                )
            })
        }),
        "bundles" if bundle_hash(id) => svc.get_bundle(id).map(|o| o.map(|t| (t, "text/plain"))),
        "runs" | "failures" | "bundles" => {
            return Reply::Error(INVALID_PARAMS, format!("invalid {kind} identifier"))
        }
        _ => return Reply::Error(INVALID_PARAMS, "unknown resource type".to_string()),
    };
    match found {
        Ok(Some((text, mime))) => {
            Reply::Result(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] }))
        }
        Ok(None) => Reply::Error(NOT_FOUND, "resource not found".to_string()),
        Err(e) => internal_error(e),
    }
}

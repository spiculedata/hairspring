//! Model Context Protocol server for Anthill.
//!
//! Lets an agent run the loop *write a property -> explore -> read the minimal
//! reproduction -> fix -> explore again* without a human relaying logs. Speaks
//! JSON-RPC 2.0 over stdio (local) or HTTP POST (via `anthill-server`). Hand
//! rolled on purpose: the surface we need is small and this keeps the
//! dependency tree minimal.
mod resources;
mod rpc;
mod tools;
pub mod validate;

use anthill_engine::engine::Engine;
use anthill_service::{Role, Service};
use rpc::{error_response, ok_response, Reply};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// Largest accepted JSON-RPC message.
pub const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Anthill explores a system for property violations and gives you a minimal, replayable \
reproduction. Typical loop: list_targets, then start_run (wait=true) on a target, then get_failure for the steps and \
bundle, fix the code, and start_run again. Use replay_bundle to confirm a saved reproduction still fails (reproduced=true) \
or has been fixed (reproduced=false). Runs are deterministic: the same seed and bundle always give the same result.";

pub struct McpServer {
    svc: Service,
    engine: Engine,
    /// Execute runs inside this process (stdio mode). Otherwise runs are queued for workers.
    inline: bool,
}

impl McpServer {
    pub fn new(svc: Service, engine: Engine, inline_execution: bool) -> McpServer {
        McpServer {
            svc,
            engine,
            inline: inline_execution,
        }
    }

    /// Handles one JSON-RPC message (or batch). Returns `None` when nothing should be sent back.
    pub fn handle_message(&self, body: &str, role: Role) -> Option<String> {
        if body.len() > MAX_MESSAGE_BYTES {
            return Some(error_response(&Value::Null, -32600, "message too large").to_string());
        }
        let parsed: Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(e) => {
                return Some(
                    error_response(&Value::Null, -32700, &format!("parse error: {e}")).to_string(),
                )
            }
        };
        match parsed {
            Value::Array(items) if items.is_empty() => {
                Some(error_response(&Value::Null, -32600, "empty batch").to_string())
            }
            Value::Array(items) => {
                let replies: Vec<Value> = items
                    .iter()
                    .filter_map(|m| self.handle_one(m, role))
                    .collect();
                if replies.is_empty() {
                    None
                } else {
                    Some(Value::Array(replies).to_string())
                }
            }
            single => self.handle_one(&single, role).map(|v| v.to_string()),
        }
    }

    fn handle_one(&self, msg: &Value, role: Role) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            return Some(error_response(
                &Value::Null,
                -32600,
                "invalid request: expected an object",
            ));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return id.map(|id| error_response(&id, -32600, "invalid request: missing method"));
        };
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
        let reply = self.dispatch(method, &params, role);
        // Requests without an id are notifications: never answered.
        let id = id?;
        Some(match reply {
            Reply::Result(v) => ok_response(&id, v),
            Reply::Error(code, message) => error_response(&id, code, &message),
        })
    }

    fn dispatch(&self, method: &str, params: &Value, role: Role) -> Reply {
        match method {
            "initialize" => Reply::Result(self.initialize(params)),
            "ping" => Reply::Result(json!({})),
            "tools/list" => Reply::Result(tools::list()),
            "tools/call" => tools::call(self, params, role),
            "resources/list" => resources::list(self),
            "resources/templates/list" => Reply::Result(resources::templates()),
            "resources/read" => resources::read(self, params),
            m if m.starts_with("notifications/") => Reply::Result(json!({})),
            _ => Reply::Error(-32601, format!("method not found: {method}")),
        }
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let version = SUPPORTED_VERSIONS
            .iter()
            .find(|v| **v == requested)
            .unwrap_or(&SUPPORTED_VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false }, "resources": { "listChanged": false, "subscribe": false } },
            "serverInfo": { "name": "anthill", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        })
    }

    pub(crate) fn service(&self) -> &Service {
        &self.svc
    }

    pub(crate) fn engine(&self) -> &Engine {
        &self.engine
    }

    pub(crate) fn inline(&self) -> bool {
        self.inline
    }
}

/// Newline-delimited JSON-RPC over a reader/writer pair (the MCP stdio transport).
pub fn serve_stdio(
    server: &McpServer,
    role: Role,
    input: impl BufRead,
    mut out: impl Write,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_message(&line, role) {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

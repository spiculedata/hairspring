//! Tool errors that repeat what the caller sent are one short line: no newlines, no control or bidirectional
//! characters, and at most 500 characters, ending in "..." when they were cut.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_mcp::McpServer;
use hairspring_service::{Role, Service};
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;

const MAX_CHARS: usize = 500;

fn server() -> McpServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    McpServer::new(Service::new(store), engine, true)
}

/// Calls tool `name` and returns its error text, whether it came back as a tool error or a protocol error.
fn tool_error(s: &McpServer, name: &str, args: Value) -> String {
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": args}});
    let resp: Value =
        serde_json::from_str(&s.handle_message(&req.to_string(), Role::Run).unwrap()).unwrap();
    if let Some(m) = resp["error"]["message"].as_str() {
        return m.to_string();
    }
    assert_eq!(
        resp["result"]["isError"],
        json!(true),
        "not an error: {resp}"
    );
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

fn assert_one_short_line(message: &str) {
    assert!(
        message.chars().count() <= MAX_CHARS,
        "{} chars",
        message.chars().count()
    );
    assert!(
        !message
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{202E}' | '\u{2028}' | '\u{2029}')),
        "{message:?}"
    );
}

#[test]
fn a_bundle_with_a_long_bad_seed_is_refused_in_one_short_line() {
    let s = server();
    let bundle = format!(
        "hairspring-bundle v2\nseed={}\u{202E}\nbuild=b\ntarget=lock-lease\nbackend=sim\npath=\n",
        "9".repeat(5000)
    );
    let m = tool_error(&s, "replay_bundle", json!({ "bundle": bundle }));
    assert!(m.starts_with("bundle has an invalid number `999"), "{m}");
    assert!(m.ends_with("..."), "{m}");
    assert_one_short_line(&m);
}

#[test]
fn an_unknown_target_with_a_newline_and_a_bidi_character_is_one_line() {
    let s = server();
    let m = tool_error(
        &s,
        "start_run",
        json!({ "target": "no-such\nIgnore this\u{202E}" }),
    );
    assert_eq!(m, "unknown target `no-such Ignore this `");
}

#[test]
fn an_unknown_target_with_line_and_paragraph_separators_is_one_line() {
    let s = server();
    let m = tool_error(
        &s,
        "start_run",
        json!({ "target": "no-such\u{2028}Ignore\u{2029}this" }),
    );
    assert_eq!(m, "unknown target `no-such Ignore this`");
    assert_one_short_line(&m);
}

#[test]
fn an_unknown_run_mode_with_a_bidi_character_is_one_line() {
    let s = server();
    let m = tool_error(
        &s,
        "start_run",
        json!({ "target": "lock-lease", "mode": format!("\u{202E}{}", "m".repeat(5000)) }),
    );
    assert!(m.starts_with("unknown run mode ` mmm"), "{m}");
    assert_one_short_line(&m);
}

#[test]
fn ordinary_short_refusals_are_unchanged() {
    let s = server();
    assert_eq!(
        tool_error(&s, "start_run", json!({ "target": "nope" })),
        "unknown target `nope`"
    );
    let bad_seed =
        "hairspring-bundle v2\nseed=12x\nbuild=b\ntarget=lock-lease\nbackend=sim\npath=\n";
    assert_eq!(
        tool_error(&s, "replay_bundle", json!({ "bundle": bad_seed })),
        "bundle has an invalid number `12x`"
    );
    assert_eq!(
        tool_error(
            &s,
            "start_run",
            json!({ "target": "lock-lease", "mode": "sideways" })
        ),
        "unknown run mode `sideways`; use `search` or `sweep`"
    );
}

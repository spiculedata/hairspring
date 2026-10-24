//! `start_run` with `mode: "sweep"`: the whole trial budget runs and every trial can be listed afterwards.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_mcp::McpServer;
use hairspring_service::{Role, Service};
use hairspring_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;

const INVALID_PARAMS: i64 = -32602;

fn server() -> McpServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    McpServer::new(Service::new(store), engine, true)
}

fn rpc(s: &McpServer, role: Role, method: &str, params: Value) -> Value {
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let resp = s
        .handle_message(&req.to_string(), role)
        .expect("a response");
    serde_json::from_str(&resp).expect("response is JSON")
}

fn tool(s: &McpServer, name: &str, args: Value) -> Value {
    rpc(
        s,
        Role::Run,
        "tools/call",
        json!({"name": name, "arguments": args}),
    )
}

fn structured(resp: &Value) -> &Value {
    assert!(resp["error"].is_null(), "unexpected rpc error: {resp}");
    assert_ne!(resp["result"]["isError"], json!(true), "tool error: {resp}");
    &resp["result"]["structuredContent"]
}

#[test]
fn a_sweep_started_through_mcp_runs_its_whole_budget_and_its_trials_are_listed() {
    let s = server();
    let run = tool(
        &s,
        "start_run",
        json!({"target": "lock-lease", "seed": 7, "mode": "sweep", "max_trials": 4,
               "max_steps": 10000000, "wait": true}),
    );
    let run = structured(&run);
    assert_eq!(run["run"]["state"], "failed");
    assert_eq!(run["run"]["trials"], 4, "{run}");
    assert_eq!(run["run"]["stopped_at_first_failure"], false);
    assert_eq!(
        run["sweep"]["trials"], 4,
        "the summary comes back with the run: {run}"
    );
    let id = run["run"]["id"].as_str().unwrap();
    let again = tool(&s, "get_run", json!({"run_id": id}));
    assert_eq!(structured(&again)["sweep"]["trials"], 4);
    let listed = tool(&s, "list_trials", json!({"run_id": id}));
    let trials = structured(&listed)["trials"].as_array().unwrap().len();
    assert_eq!(trials, 4);
}

#[test]
fn a_run_without_a_mode_stops_at_its_first_failure() {
    let s = server();
    let run = tool(
        &s,
        "start_run",
        json!({"target": "lock-lease", "seed": 7, "wait": true}),
    );
    assert_eq!(structured(&run)["run"]["stopped_at_first_failure"], true);
}

#[test]
fn a_bad_mode_or_trial_count_is_a_parameter_error() {
    let s = server();
    for args in [
        json!({"target": "lock-lease", "mode": "fast"}),
        json!({"target": "lock-lease", "mode": 5}),
        json!({"target": "lock-lease", "max_trials": 0}),
        json!({"target": "lock-lease", "max_trials": 10001}),
        json!({"target": "lock-lease", "max_trials": "ten"}),
    ] {
        let r = tool(&s, "start_run", args.clone());
        assert_eq!(r["error"]["code"], INVALID_PARAMS, "{args}: {r}");
    }
}

#[test]
fn the_start_run_schema_offers_both_modes_and_a_trial_count() {
    let s = server();
    let r = rpc(&s, Role::Read, "tools/list", json!({}));
    let tool = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "start_run")
        .unwrap();
    let props = &tool["inputSchema"]["properties"];
    assert_eq!(props["mode"]["enum"], json!(["search", "sweep"]));
    assert_eq!(props["max_trials"]["maximum"], 10000);
}

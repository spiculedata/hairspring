use anthill_engine::config::Config;
use anthill_engine::demos::default_registry;
use anthill_engine::engine::Engine;
use anthill_mcp::McpServer;
use anthill_service::{Role, Service};
use anthill_store::{SqliteStore, Store};
use serde_json::{json, Value};
use std::sync::Arc;

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

fn tool(s: &McpServer, role: Role, name: &str, args: Value) -> Value {
    rpc(
        s,
        role,
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
fn initialize_negotiates_and_declares_capabilities() {
    let s = server();
    let r = rpc(
        &s,
        Role::Read,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    );
    assert_eq!(r["result"]["serverInfo"]["name"], "anthill");
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert!(
        r["result"]["capabilities"]["tools"].is_object()
            && r["result"]["capabilities"]["resources"].is_object()
    );
    assert!(r["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("start_run"));
    let old = rpc(
        &s,
        Role::Read,
        "initialize",
        json!({"protocolVersion": "1999-01-01"}),
    );
    assert!(
        old["result"]["protocolVersion"]
            .as_str()
            .unwrap()
            .starts_with("20"),
        "falls back to a version we support"
    );
}

#[test]
fn notifications_get_no_response_and_ping_works() {
    let s = server();
    assert!(s
        .handle_message(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            Role::Read
        )
        .is_none());
    assert_eq!(rpc(&s, Role::Read, "ping", json!({}))["result"], json!({}));
}

#[test]
fn tools_list_has_schemas_and_expected_tools() {
    let s = server();
    let r = rpc(&s, Role::Read, "tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in [
        "list_targets",
        "start_run",
        "get_run",
        "list_runs",
        "list_failures",
        "get_failure",
        "replay_bundle",
        "cancel_run",
    ] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    assert!(tools
        .iter()
        .all(|t| t["inputSchema"]["type"] == "object"
            && !t["description"].as_str().unwrap().is_empty()));
}

#[test]
fn full_agent_loop_find_inspect_replay() {
    let s = server();
    let targets = tool(&s, Role::Run, "list_targets", json!({}));
    assert!(structured(&targets)["targets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "lock-lease"));

    let run = tool(
        &s,
        Role::Run,
        "start_run",
        json!({"target": "lock-lease", "seed": 7, "wait": true}),
    );
    let run = structured(&run);
    assert_eq!(run["run"]["state"], "failed");
    let failure_id = run["failures"][0]["id"].as_str().unwrap().to_string();

    let f = tool(
        &s,
        Role::Read,
        "get_failure",
        json!({"failure_id": failure_id}),
    );
    let f = structured(&f);
    let steps = f["steps"].as_array().unwrap();
    assert!(
        !steps.is_empty() && steps.len() == f["record"]["shrunk_len"].as_u64().unwrap() as usize
    );
    assert!(
        f["explanation"]
            .as_str()
            .unwrap()
            .contains("Minimal reproduction"),
        "{}",
        f["explanation"]
    );
    let bundle = f["bundle"].as_str().unwrap().to_string();
    assert!(bundle.starts_with("anthill-bundle v2"));

    let by_text = tool(&s, Role::Read, "replay_bundle", json!({"bundle": bundle}));
    assert_eq!(structured(&by_text)["reproduced"], json!(true));
    let hash = f["record"]["bundle_hash"].as_str().unwrap();
    let by_hash = tool(
        &s,
        Role::Read,
        "replay_bundle",
        json!({"bundle_hash": hash}),
    );
    assert_eq!(structured(&by_hash)["reproduced"], json!(true));

    let got = tool(
        &s,
        Role::Read,
        "get_run",
        json!({"run_id": run["run"]["id"]}),
    );
    assert_eq!(structured(&got)["run"]["target"], "lock-lease");
    assert_eq!(
        structured(&tool(&s, Role::Read, "list_runs", json!({"limit": 5})))["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        structured(&tool(&s, Role::Read, "list_failures", json!({})))["failures"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn passing_target_reports_passed() {
    let s = server();
    let r = tool(
        &s,
        Role::Run,
        "start_run",
        json!({"target": "pubsub-atomic", "seed": 1, "max_steps": 20000, "wait": true}),
    );
    assert_eq!(structured(&r)["run"]["state"], "passed");
}

#[test]
fn read_only_role_cannot_start_or_cancel() {
    let s = server();
    for (name, args) in [
        ("start_run", json!({"target": "lock-lease"})),
        ("cancel_run", json!({"run_id": "run_x"})),
    ] {
        let r = tool(&s, Role::Read, name, args);
        assert_eq!(r["result"]["isError"], json!(true), "{name}: {r}");
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("permission"),
            "{r}"
        );
    }
}

#[test]
fn bad_arguments_are_rejected_with_invalid_params() {
    let s = server();
    for args in [
        json!({}),
        json!({"target": 5}),
        json!({"target": "x".repeat(200)}),
        json!({"target": "lock-lease", "seed": "abc"}),
        json!({"target": "lock-lease", "max_steps": -1}),
    ] {
        let r = tool(&s, Role::Run, "start_run", args.clone());
        assert_eq!(r["error"]["code"], -32602, "{args}: {r}");
    }
    let r = tool(
        &s,
        Role::Read,
        "get_run",
        json!({"run_id": "../../etc/passwd"}),
    );
    assert_eq!(r["error"]["code"], -32602);
}

#[test]
fn domain_errors_are_tool_errors_not_protocol_errors() {
    let s = server();
    let r = tool(
        &s,
        Role::Run,
        "start_run",
        json!({"target": "no-such-target"}),
    );
    assert_eq!(r["result"]["isError"], json!(true));
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("no-such-target"));
    let r = tool(
        &s,
        Role::Read,
        "get_run",
        json!({"run_id": "run_0123456789abcdef"}),
    );
    assert_eq!(r["result"]["isError"], json!(true));
    let r = tool(
        &s,
        Role::Read,
        "replay_bundle",
        json!({"bundle": "garbage"}),
    );
    assert_eq!(r["result"]["isError"], json!(true));
}

#[test]
fn protocol_errors_have_standard_codes() {
    let s = server();
    assert_eq!(
        rpc(&s, Role::Read, "no/such/method", json!({}))["error"]["code"],
        -32601
    );
    assert_eq!(
        rpc(
            &s,
            Role::Read,
            "tools/call",
            json!({"name": "nope", "arguments": {}})
        )["error"]["code"],
        -32602
    );
    let bad: Value =
        serde_json::from_str(&s.handle_message("{not json", Role::Read).unwrap()).unwrap();
    assert_eq!(bad["error"]["code"], -32700);
    let bad: Value = serde_json::from_str(&s.handle_message("[1]", Role::Read).unwrap()).unwrap();
    assert!(bad.is_array() && bad[0]["error"]["code"] == -32600, "{bad}");
    let huge = "x".repeat(3 * 1024 * 1024);
    let bad: Value = serde_json::from_str(&s.handle_message(&huge, Role::Read).unwrap()).unwrap();
    assert_eq!(bad["error"]["code"], -32600);
}

#[test]
fn batches_are_answered_in_order_and_skip_notifications() {
    let s = server();
    let batch = json!([
        {"jsonrpc": "2.0", "id": 1, "method": "ping"},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": "b", "method": "tools/list"}
    ]);
    let resp: Value =
        serde_json::from_str(&s.handle_message(&batch.to_string(), Role::Read).unwrap()).unwrap();
    let arr = resp.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(
        (arr[0]["id"].clone(), arr[1]["id"].clone()),
        (json!(1), json!("b"))
    );
}

#[test]
fn resources_list_templates_and_read() {
    let s = server();
    let run = tool(
        &s,
        Role::Run,
        "start_run",
        json!({"target": "pubsub-buggy", "seed": 5, "wait": true}),
    );
    let run = structured(&run).clone();
    let id = run["run"]["id"].as_str().unwrap();
    let list = rpc(&s, Role::Read, "resources/list", json!({}));
    let uris: Vec<_> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap().to_string())
        .collect();
    assert!(uris.contains(&format!("anthill://runs/{id}")), "{uris:?}");
    assert!(
        rpc(&s, Role::Read, "resources/templates/list", json!({}))["result"]["resourceTemplates"]
            .as_array()
            .unwrap()
            .len()
            >= 3
    );
    let read = rpc(
        &s,
        Role::Read,
        "resources/read",
        json!({"uri": format!("anthill://runs/{id}")}),
    );
    let text = read["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(serde_json::from_str::<Value>(text).unwrap()["run"]["target"] == "pubsub-buggy");
    let fid = run["failures"][0]["id"].as_str().unwrap();
    assert!(rpc(
        &s,
        Role::Read,
        "resources/read",
        json!({"uri": format!("anthill://failures/{fid}")})
    )["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("steps"));
    let hash = run["failures"][0]["bundle_hash"].as_str().unwrap();
    assert!(rpc(
        &s,
        Role::Read,
        "resources/read",
        json!({"uri": format!("anthill://bundles/{hash}")})
    )["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("anthill-bundle"));
    for bad in [
        "anthill://runs/../../etc/passwd",
        "file:///etc/passwd",
        "anthill://nothing/x",
        "anthill://runs/",
    ] {
        assert!(
            rpc(&s, Role::Read, "resources/read", json!({"uri": bad}))["error"].is_object(),
            "{bad} must be rejected"
        );
    }
}

#[test]
fn stdio_loop_processes_lines_and_stops_at_eof() {
    let s = server();
    let input = format!(
        "{}\n\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
    );
    let mut out = Vec::new();
    anthill_mcp::serve_stdio(&s, Role::Admin, std::io::Cursor::new(input), &mut out).unwrap();
    let lines: Vec<_> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["id"], 2);
}

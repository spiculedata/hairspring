//! The optional organisation name: set in the server's configuration, served by `GET /api/health` to callers the server
//! would answer anyway, and absent when it is not set.
use hairspring_engine::config::Config;
use hairspring_engine::demos::default_registry;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{SqliteStore, Store};
use serde_json::Value;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";

fn boot(auth: Auth, organisation: Option<&str>) -> std::io::Result<RunningServer> {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    cfg.organisation = organisation.map(str::to_owned);
    start(cfg, Service::new(store), engine)
}

fn health(addr: SocketAddr, token: Option<&str>) -> Value {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut r = agent.get(format!("http://{addr}/api/health"));
    if let Some(t) = token {
        r = r.header("Authorization", format!("Bearer {t}"));
    }
    let mut resp = r.call().expect("transport ok");
    assert_eq!(resp.status().as_u16(), 200, "health always answers");
    serde_json::from_str(&resp.body_mut().read_to_string().unwrap()).unwrap()
}

fn tokens() -> Auth {
    Auth::from_spec(&format!("{READ}:read")).unwrap()
}

#[test]
fn health_has_no_organisation_when_none_is_set() {
    let server = boot(tokens(), None).unwrap();
    let body = health(server.addr(), Some(READ));
    assert_eq!(body["status"], "ok");
    assert!(body.get("organisation").is_none(), "{body}");
}

#[test]
fn a_set_name_is_served_to_a_caller_with_a_token() {
    let server = boot(tokens(), Some("Acme Ltd")).unwrap();
    let body = health(server.addr(), Some(READ));
    assert_eq!(body["organisation"], "Acme Ltd");
    assert_eq!(body["status"], "ok", "the rest of the answer is unchanged");
    assert!(body["version"].is_string());
}

#[test]
fn a_caller_without_a_valid_token_does_not_learn_the_name() {
    let server = boot(tokens(), Some("Acme Ltd")).unwrap();
    for token in [None, Some("wrong-token-0123456789abcdef"), Some("")] {
        let body = health(server.addr(), token);
        assert_eq!(body["status"], "ok", "{token:?}");
        assert!(body.get("organisation").is_none(), "{token:?}: {body}");
    }
}

#[test]
fn without_tokens_the_name_is_served_as_the_other_endpoints_are() {
    let server = boot(Auth::open(), Some("Acme Ltd")).unwrap();
    assert_eq!(health(server.addr(), None)["organisation"], "Acme Ltd");
}

fn boot_limited(limit: u32, organisation: Option<&str>) -> RunningServer {
    let registry = Arc::new(default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), tokens());
    cfg.embedded_workers = 0;
    cfg.rate_limit_per_min = limit;
    cfg.organisation = organisation.map(str::to_owned);
    start(cfg, Service::new(store), engine).unwrap()
}

#[test]
fn guessing_tokens_through_health_is_limited_and_never_confirms_the_name() {
    let server = boot_limited(2, Some("Acme Ltd"));
    for guess in 0..50 {
        let body = health(server.addr(), Some(&format!("wrong-token-{guess:016}")));
        assert!(body.get("organisation").is_none(), "guess {guess}: {body}");
    }
    let body = health(server.addr(), Some(READ));
    assert_eq!(body["status"], "ok", "health still answers");
    assert!(
        body.get("organisation").is_none(),
        "the real token is not confirmed once the guessing limit is used up: {body}"
    );
}

#[test]
fn tokenless_health_is_never_limited() {
    let server = boot_limited(2, Some("Acme Ltd"));
    for _ in 0..20 {
        assert_eq!(health(server.addr(), None)["status"], "ok");
    }
    assert_eq!(
        health(server.addr(), Some(READ))["organisation"],
        "Acme Ltd",
        "tokenless probes do not use up the budget of a later token check"
    );
}

#[test]
fn a_name_is_trimmed_and_a_blank_name_is_none() {
    let server = boot(tokens(), Some("  Acme Ltd  ")).unwrap();
    assert_eq!(
        health(server.addr(), Some(READ))["organisation"],
        "Acme Ltd"
    );
    let blank = boot(tokens(), Some("   ")).unwrap();
    assert!(health(blank.addr(), Some(READ))
        .get("organisation")
        .is_none());
}

#[test]
fn markup_in_the_name_is_served_as_text() {
    let server = boot(tokens(), Some("<script>alert(1)</script>")).unwrap();
    assert_eq!(
        health(server.addr(), Some(READ))["organisation"],
        "<script>alert(1)</script>"
    );
}

#[test]
fn a_name_that_is_not_valid_stops_the_server_starting() {
    for bad in [
        "x".repeat(81),
        "line\nbreak".to_owned(),
        "bidi\u{202E}override".to_owned(),
    ] {
        match boot(tokens(), Some(&bad)) {
            Err(e) => {
                assert_eq!(e.kind(), ErrorKind::InvalidInput);
                assert!(e.to_string().contains("organisation"), "{e}");
                assert!(!e.to_string().contains("override"), "{e}");
            }
            Ok(_) => panic!("{bad:?} should be refused"),
        }
    }
}

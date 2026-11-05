//! The event stream asks only for the run's own row to learn whether it is over. It must not build the whole run
//! detail (failures, findings, properties, sweep) at connect or at every poll: on a large run that costs far more than
//! the events themselves.
//!
//! The proof does not rely on timing. The run below holds a failure with a stored check whose environment is not
//! valid JSON, so building the run's full detail fails. A stream that only reads the run's state is unaffected.
use hairspring_engine::config::Config;
use hairspring_engine::engine::Engine;
use hairspring_server::{start, Auth, RunningServer, ServerConfig};
use hairspring_service::Service;
use hairspring_store::{
    NewCheck, NewConfirmation, NewFailure, NewRun, RunState, SqliteStore, Store,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

const READ: &str = "read-token-0123456789abcdef";
const FINISH_AFTER: Duration = Duration::from_millis(600);

struct Resp {
    status: u16,
    body: String,
}

fn get(addr: SocketAddr, token: &str, path: &str) -> Resp {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut resp = agent
        .get(format!("http://{addr}{path}"))
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .expect("transport ok");
    Resp {
        status: resp.status().as_u16(),
        body: resp.body_mut().read_to_string().unwrap_or_default(),
    }
}

/// A server holding a running run whose full detail cannot be built.
fn boot_with_unreadable_detail() -> (RunningServer, Arc<dyn Store>, String) {
    let registry = Arc::new(hairspring_engine::demos::default_registry());
    let engine = Engine::new(registry.clone(), Config::from_registry(&registry));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    let run = store
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    store.mark_running(&run.id, 2).unwrap();
    let failure = store
        .add_failure(
            &NewFailure {
                run_id: run.id.clone(),
                target: "t".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: store.put_blob(b"bundle").unwrap(),
                steps_json: "[]".into(),
            },
            3,
        )
        .unwrap();
    store
        .add_confirmation(
            &NewConfirmation {
                id: "conf-1".into(),
                failure_id: failure.id,
                runner: "local-1".into(),
                runner_trusted: true,
                checks: vec![NewCheck {
                    kind: "same_seed_rerun".into(),
                    status: "pass".into(),
                    passed: 1,
                    failed: 0,
                    inconclusive: 0,
                    command: "c".into(),
                    detail_json: "{}".into(),
                    artefacts_json: "[]".into(),
                    environment_json: "this is not json".into(),
                    started_at: 1,
                    finished_at: 2,
                }],
            },
            4,
        )
        .unwrap();
    let svc = Service::new(store.clone());
    assert!(
        svc.get_run(&run.id).is_err(),
        "the fixture must make the full run detail unreadable"
    );
    let auth = Auth::from_spec(&format!("{READ}:read")).unwrap();
    let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), auth);
    cfg.embedded_workers = 0;
    (
        start(cfg, svc, engine).expect("server starts"),
        store,
        run.id,
    )
}

#[test]
fn the_stream_follows_a_run_whose_full_detail_cannot_be_built_and_ends_when_it_finishes() {
    // Arrange
    let (srv, store, id) = boot_with_unreadable_detail();
    store.add_event(&id, "started", "{}", 5).unwrap();
    let finisher = {
        let (store, id) = (store.clone(), id.clone());
        std::thread::spawn(move || {
            std::thread::sleep(FINISH_AFTER);
            store
                .finish_run(&id, RunState::Passed, 1, 1, None, 10)
                .unwrap();
            store.add_event(&id, "finished", "{}", 11).unwrap();
        })
    };

    // Act
    let r = get(srv.addr(), READ, &format!("/api/runs/{id}/events"));
    finisher.join().unwrap();

    // Assert
    assert_eq!(r.status, 200, "{}", r.body);
    let started = r.body.find("event: started").expect("started event");
    let finished = r.body.find("event: finished").expect("finished event");
    assert!(started < finished, "{}", r.body);
}

#[test]
fn an_unknown_run_is_still_a_404() {
    let (srv, _, _) = boot_with_unreadable_detail();
    let r = get(srv.addr(), READ, "/api/runs/run_0000000000000000/events");
    assert_eq!(r.status, 404);
}

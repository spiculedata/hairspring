//! The target listing names each target's backend, and a run records the same name.
use hairspring_engine::config::Config;
use hairspring_engine::engine::Engine;
use hairspring_engine::erased::Registry;
use hairspring_service::{Service, StartRun};
use hairspring_store::{SqliteStore, Store};
use std::sync::Arc;

const CONFIG: &str = "[target.plain]\nkind = \"process\"\ncommand = [\"/bin/true\"]\n\
    [target.named]\nkind = \"process\"\ncommand = [\"/bin/true\"]\nbackend = \"hermit\"\n";

fn setup() -> (Service, Engine) {
    let engine = Engine::new(Arc::new(Registry::new()), Config::parse(CONFIG).unwrap());
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().unwrap());
    (Service::new(store), engine)
}

#[test]
fn a_process_target_is_listed_under_hermit_whether_or_not_it_names_it() {
    let (svc, engine) = setup();
    let listed = svc.list_targets(&engine);
    assert_eq!(listed.len(), 2);
    for target in &listed {
        assert_eq!(target.kind, "process");
        assert_eq!(target.backend, "hermit", "{}", target.name);
    }
}

#[test]
fn capabilities_are_unknown_when_this_engine_has_no_backend_for_the_target() {
    let (svc, engine) = setup();
    assert!(svc
        .list_targets(&engine)
        .iter()
        .all(|t| t.capabilities.is_none()));
}

#[test]
fn a_run_records_the_backend_the_listing_names() {
    let (svc, engine) = setup();
    for name in ["plain", "named"] {
        let run = svc
            .start_run(
                &engine,
                &StartRun {
                    target: name.into(),
                    seed: Some(1),
                    budget: None,
                },
            )
            .unwrap();
        assert_eq!(run.backend, "hermit", "{name}");
    }
}

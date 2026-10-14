mod common;
use anthill_store::{SqliteStore, Store};
use std::sync::Arc;

#[test]
fn sqlite_store_passes_the_conformance_suite() {
    common::run_all(&|| Arc::new(SqliteStore::open_in_memory().unwrap()) as Arc<dyn Store>);
}

#[test]
fn sqlite_store_persists_across_reopen_and_migrates_idempotently() {
    let dir = std::env::temp_dir().join(format!(
        "anthill-store-test-{}-{}",
        std::process::id(),
        anthill_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.db");
    let id = {
        let s = SqliteStore::open(&path).unwrap();
        s.create_run(
            &anthill_store::NewRun {
                target: "t".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            5,
        )
        .unwrap()
        .id
    };
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run(&id).unwrap().unwrap().target, "t");
    std::fs::remove_dir_all(&dir).ok();
}

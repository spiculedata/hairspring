mod common;
use hairspring_store::{SqliteStore, Store};
use std::sync::Arc;

#[test]
fn sqlite_store_passes_the_conformance_suite() {
    common::run_all(&|| Arc::new(SqliteStore::open_in_memory().unwrap()) as Arc<dyn Store>);
}

#[test]
fn sqlite_store_persists_across_reopen_and_migrates_idempotently() {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-store-test-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.db");
    let id = {
        let s = SqliteStore::open(&path).unwrap();
        s.create_run(
            &hairspring_store::NewRun {
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

#[test]
fn random_seeds_are_exactly_representable_in_javascript() {
    // Browsers and LLM tool clients parse JSON numbers as doubles: integers above 2^53 lose precision.
    const JS_SAFE: u64 = 1 << 53;
    let seeds: Vec<u64> = (0..2000).map(|_| hairspring_store::random_seed()).collect();
    assert!(
        seeds.iter().all(|s| *s < JS_SAFE),
        "seed above 2^53 would be corrupted by JSON consumers"
    );
    let distinct: std::collections::HashSet<_> = seeds.iter().collect();
    assert!(distinct.len() > 1990, "still varied: {}", distinct.len());
}

//! Runs the shared conformance suite against PostgreSQL. Skipped unless `HAIRSPRING_TEST_PG_URL` is set:
//!
//!     docker run -d --name pg -e POSTGRES_PASSWORD=x -e POSTGRES_DB=hairspring -p 127.0.0.1:54329:5432 postgres:17-alpine
//!     HAIRSPRING_TEST_PG_URL=postgres://postgres:x@127.0.0.1:54329/hairspring cargo test -p hairspring-store --features postgres
#![cfg(feature = "postgres")]
mod common;
use hairspring_store::{new_id, PgStore, Store};
use std::sync::Arc;

fn url() -> Option<String> {
    let u = std::env::var("HAIRSPRING_TEST_PG_URL").ok();
    if u.is_none() {
        eprintln!("skipped: set HAIRSPRING_TEST_PG_URL to run the PostgreSQL conformance tests");
    }
    u
}

/// A store in its own throwaway schema, so every scenario starts empty.
fn fresh(url: &str) -> Arc<dyn Store> {
    let schema = new_id("t").replace('-', "_");
    Arc::new(PgStore::connect_in_schema(url, &schema, 8).expect("connect"))
}

#[test]
fn postgres_store_passes_the_conformance_suite() {
    let Some(url) = url() else { return };
    common::run_all(&|| fresh(&url));
}

#[test]
fn migrating_twice_and_concurrently_is_safe() {
    let Some(url) = url() else { return };
    let schema = new_id("m").replace('-', "_");
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (url, schema) = (url.clone(), schema.clone());
            std::thread::spawn(move || PgStore::connect_in_schema(&url, &schema, 2).is_ok())
        })
        .collect();
    assert!(
        handles.into_iter().all(|h| h.join().unwrap()),
        "concurrent startup must not race on schema creation"
    );
    let again = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    assert!(again.list_runs(1).unwrap().is_empty());
}

#[test]
fn an_unreachable_database_is_a_clear_error_that_does_not_echo_the_password() {
    let err = PgStore::connect_in_schema("postgres://user:s3cr3t-pass@127.0.0.1:1/none", "x", 1)
        .err()
        .expect("must fail");
    let text = err.to_string();
    assert!(
        !text.contains("s3cr3t-pass"),
        "credentials must never appear in errors: {text}"
    );
}

#[test]
fn schema_names_are_validated_against_injection() {
    let Some(url) = url() else { return };
    assert!(PgStore::connect_in_schema(&url, "x; DROP SCHEMA public CASCADE; --", 1).is_err());
    assert!(PgStore::connect_in_schema(&url, "", 1).is_err());
}

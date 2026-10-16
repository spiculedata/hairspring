//! Helpers for the PostgreSQL tests: the connection URL from the environment, throwaway schemas and raw connections.
#![allow(dead_code)]
use hairspring_store::new_id;
use postgres::{Client, NoTls};

/// The test database URL, or `None` (and a note) when `HAIRSPRING_TEST_PG_URL` is not set.
pub fn url() -> Option<String> {
    let u = std::env::var("HAIRSPRING_TEST_PG_URL").ok();
    if u.is_none() {
        eprintln!("skipped: set HAIRSPRING_TEST_PG_URL to run the PostgreSQL tests");
    }
    u
}

pub fn new_schema(prefix: &str) -> String {
    new_id(prefix).replace('-', "_")
}

pub fn admin(url: &str) -> Client {
    Client::connect(url, NoTls).unwrap()
}

/// A connection whose `search_path` is the schema, for raw SQL against it.
pub fn raw(url: &str, schema: &str) -> Client {
    let mut c = admin(url);
    c.batch_execute(&format!("SET search_path = \"{schema}\""))
        .unwrap();
    c
}

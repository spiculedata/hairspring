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

/// A genuine version 1 schema, created the way the first release created it, with data.
fn make_v1_schema(url: &str, schema: &str) {
    let mut c = postgres::Client::connect(url, postgres::NoTls).unwrap();
    c.batch_execute(&format!(
        "CREATE SCHEMA \"{schema}\"; SET search_path = \"{schema}\";"
    ))
    .unwrap();
    c.batch_execute(common::v1_schema::POSTGRES_V1).unwrap();
    c.batch_execute(
        "CREATE TABLE schema_version (version INTEGER NOT NULL);
         INSERT INTO schema_version(version) VALUES (1);
         INSERT INTO runs(id,target,backend,state,seed,config_json,created_at,steps,trials)
           VALUES ('run_old','legacy','hermit','passed',9,'{}',1000,50,12);
         INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at)
           VALUES ('fail_old','run_old','legacy','m',4,2,'h','[]',1001);
         INSERT INTO workers(id,name,caps_json,registered_at,last_seen) VALUES ('wrk_old','w','{}',1,2);",
    )
    .unwrap();
}

#[test]
fn a_version_one_schema_migrates_in_place_and_old_rows_read_back_with_zero_counts() {
    let Some(url) = url() else { return };
    let schema = new_id("v").replace('-', "_");
    make_v1_schema(&url, &schema);
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let run = s.get_run("run_old").unwrap().unwrap();
    assert_eq!(
        (run.target.as_str(), run.steps, run.trials),
        ("legacy", 50, 12)
    );
    assert_eq!(
        (
            run.inconclusive,
            run.distinct_schedules,
            run.work,
            run.catalog_size
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(run.mode, "search");
    assert!(!run.stopped_at_first_failure && !run.ingest_incomplete);
    assert_eq!(s.get_failure("fail_old").unwrap().unwrap().shrunk_len, 2);
    let workers = s.list_workers().unwrap();
    assert_eq!(
        (workers[0].id.as_str(), workers[0].trusted),
        ("wrk_old", false)
    );
    assert_eq!(s.count_trials("run_old").unwrap(), 0);
    assert!(s.list_run_properties("run_old").unwrap().is_empty());
    // A second open finds version 2 and changes nothing.
    let again = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    assert_eq!(again.get_run("run_old").unwrap().unwrap().trials, 12);
}

#[test]
fn concurrent_startup_on_a_version_one_schema_migrates_exactly_once() {
    let Some(url) = url() else { return };
    let schema = new_id("r").replace('-', "_");
    make_v1_schema(&url, &schema);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (url, schema) = (url.clone(), schema.clone());
            std::thread::spawn(move || PgStore::connect_in_schema(&url, &schema, 2).is_ok())
        })
        .collect();
    assert!(handles.into_iter().all(|h| h.join().unwrap()));
}

#[test]
fn concurrent_startup_on_a_schema_missing_the_worker_secret_column_succeeds() {
    let Some(url) = url() else { return };
    let schema = new_id("c").replace('-', "_");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    c.batch_execute(&format!(
        "ALTER TABLE \"{schema}\".workers DROP COLUMN secret_hash"
    ))
    .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(6));
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let (url, schema, b) = (url.clone(), schema.clone(), barrier.clone());
            std::thread::spawn(move || {
                b.wait();
                PgStore::connect_in_schema(&url, &schema, 2)
                    .err()
                    .map(|e| e.to_string())
            })
        })
        .collect();
    let errors: Vec<String> = handles
        .into_iter()
        .filter_map(|h| h.join().unwrap())
        .collect();
    assert!(errors.is_empty(), "concurrent connects failed: {errors:?}");
}

#[test]
fn a_schema_from_the_future_is_refused_untouched() {
    let Some(url) = url() else { return };
    let schema = new_id("f").replace('-', "_");
    make_v1_schema(&url, &schema);
    let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    c.batch_execute(&format!(
        "UPDATE \"{schema}\".schema_version SET version = 99"
    ))
    .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("99"), "{err}");
    let rows = c
        .query(
            "SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = 'trials'",
            &[&schema],
        )
        .unwrap();
    assert!(rows.is_empty(), "a refused schema is not modified");
}

#[test]
fn a_version_two_schema_from_before_worker_secrets_gains_the_column_when_connected() {
    let Some(url) = url() else { return };
    let schema = new_id("s").replace('-', "_");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    // The earlier version 2 had no secret column.
    c.batch_execute(&format!(
        "ALTER TABLE \"{schema}\".workers DROP COLUMN secret_hash"
    ))
    .unwrap();
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let w = s
        .register_worker_with_secret("remote", "{}", "abc", 1)
        .unwrap();
    assert_eq!(s.worker_secret_hash(&w.id).unwrap().as_deref(), Some("abc"));
}

/// The URL with its user replaced by `user` (no password), for the trust-authenticated test server.
fn url_as(url: &str, user: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("a postgres URL");
    let (_, host) = rest.rsplit_once('@').expect("a URL with a user");
    format!("{scheme}://{user}@{host}")
}

#[test]
fn opening_a_current_schema_needs_no_ddl_rights_on_the_workers_table() {
    let Some(url) = url() else { return };
    let schema = new_id("d").replace('-', "_");
    let role = format!("hs_nodll_{}", &new_id("u").replace('-', "_")[..12]);
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut admin = postgres::Client::connect(&url, postgres::NoTls).unwrap();
    admin
        .batch_execute(&format!(
            "CREATE ROLE \"{role}\" LOGIN;
             GRANT CREATE ON DATABASE hairspring TO \"{role}\";
             GRANT ALL ON SCHEMA \"{schema}\" TO \"{role}\";
             GRANT ALL ON ALL TABLES IN SCHEMA \"{schema}\" TO \"{role}\";
             GRANT ALL ON ALL SEQUENCES IN SCHEMA \"{schema}\" TO \"{role}\";"
        ))
        .unwrap();
    // The role owns nothing, so any `ALTER TABLE workers` it issued would be refused.
    let opened = PgStore::connect_in_schema(&url_as(&url, &role), &schema, 1).map(|_| ());
    admin
        .batch_execute(&format!("DROP OWNED BY \"{role}\"; DROP ROLE \"{role}\""))
        .unwrap();
    assert!(opened.is_ok(), "open as a non-owner failed: {opened:?}");
}

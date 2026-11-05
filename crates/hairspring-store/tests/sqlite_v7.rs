//! Schema version 7 on SQLite: two indexes and nothing else. `failures_created` orders the failures list without a sort,
//! and the partial index `run_properties_violation` lets the bounded property filter read only a run's violations. The
//! migration rewrites no row, and every list reads the same with or without the indexes.
mod common;
use common::v7_lists::{every_list, fill};
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const CURRENT_VERSION: i64 = 7;
const V7_INDEXES: [&str; 2] = ["failures_created", "run_properties_violation"];

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-v7-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.sqlite")
}

fn cleanup(path: &Path) {
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

fn user_version(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

/// SQLite bumps this counter on every schema change, so an open that runs no DDL leaves it alone.
fn schema_counter(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap()
}

fn index_sql(conn: &Connection, name: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='index' AND name=?",
        [name],
        |r| r.get(0),
    )
    .ok()
}

fn ddl_of(conn: &Connection, table: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name=?",
        [table],
        |r| r.get(0),
    )
    .unwrap()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// Turns a current database into what a version 6 build leaves behind: the same tables, without the version 7 indexes.
fn make_v6(path: &Path) {
    Connection::open(path)
        .unwrap()
        .execute_batch(
            "DROP INDEX failures_created; DROP INDEX run_properties_violation; PRAGMA user_version = 6;",
        )
        .unwrap();
}

fn property(prefix: &str) -> FailureFilter {
    FailureFilter {
        property_prefix: Some(prefix.into()),
        ..FailureFilter::default()
    }
}

#[test]
fn a_fresh_database_is_current_with_both_indexes() {
    let path = temp_db("fresh");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    assert_eq!(
        index_sql(&conn, "failures_created").as_deref(),
        Some("CREATE INDEX failures_created ON failures(created_at)")
    );
    assert_eq!(
        index_sql(&conn, "run_properties_violation").as_deref(),
        Some("CREATE INDEX run_properties_violation ON run_properties(run_id, idx) WHERE status = 'violation'")
    );
    cleanup(&path);
}

#[test]
fn a_version_six_database_with_data_migrates_in_place_and_every_list_reads_the_same() {
    let path = temp_db("lists");
    let s = SqliteStore::open(&path).unwrap();
    let fx = fill(&s);
    let with_indexes = every_list(&s, &fx);
    make_v6(&path);
    // The same store, now planning without the indexes, as a version 6 database would.
    let without_indexes = every_list(&s, &fx);
    drop(s);
    let before = Connection::open(&path).unwrap();
    let tables = [
        "runs",
        "failures",
        "run_properties",
        "run_catalog",
        "trials",
    ];
    let ddl: Vec<String> = tables.iter().map(|t| ddl_of(&before, t)).collect();
    let rows: Vec<i64> = tables.iter().map(|t| count(&before, t)).collect();
    drop(before);

    let s = SqliteStore::open(&path).unwrap();
    let migrated = every_list(&s, &fx);
    drop(s);

    let after = Connection::open(&path).unwrap();
    assert_eq!(user_version(&after), CURRENT_VERSION);
    for index in V7_INDEXES {
        assert!(index_sql(&after, index).is_some(), "{index} was created");
    }
    assert_eq!(
        tables.iter().map(|t| ddl_of(&after, t)).collect::<Vec<_>>(),
        ddl,
        "no table changes"
    );
    assert_eq!(
        tables.iter().map(|t| count(&after, t)).collect::<Vec<_>>(),
        rows,
        "no row is added or removed"
    );
    assert!(
        with_indexes.iter().any(|(_, ids)| !ids.is_empty()),
        "the fixture lists something"
    );
    assert_eq!(without_indexes, with_indexes);
    assert_eq!(migrated, with_indexes);
    cleanup(&path);
}

#[test]
fn opening_a_migrated_database_again_runs_no_ddl() {
    let path = temp_db("again");
    drop(SqliteStore::open(&path).unwrap());
    make_v6(&path);
    drop(SqliteStore::open(&path).unwrap());
    let counter = schema_counter(&Connection::open(&path).unwrap());
    for _ in 0..3 {
        drop(SqliteStore::open(&path).unwrap());
    }
    let conn = Connection::open(&path).unwrap();
    assert_eq!(schema_counter(&conn), counter);
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    cleanup(&path);
}

#[test]
fn a_version_six_database_that_already_has_one_index_still_migrates() {
    let path = temp_db("partial");
    drop(SqliteStore::open(&path).unwrap());
    make_v6(&path);
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE INDEX failures_created ON failures(created_at)")
        .unwrap();
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    for index in V7_INDEXES {
        assert!(index_sql(&conn, index).is_some(), "{index}");
    }
    cleanup(&path);
}

#[test]
fn a_current_database_that_lost_a_version_seven_index_is_reported_not_repaired() {
    for index in V7_INDEXES {
        let path = temp_db("lost");
        drop(SqliteStore::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("DROP INDEX {index}")).unwrap();
        let err = SqliteStore::open(&path).err().expect("must refuse");
        assert!(err.to_string().contains(index), "{err}");
        assert!(index_sql(&conn, index).is_none(), "{index} not recreated");
        cleanup(&path);
    }
}

#[test]
fn a_version_six_database_missing_a_version_five_index_is_refused_not_repaired() {
    let path = temp_db("v6-lost");
    drop(SqliteStore::open(&path).unwrap());
    make_v6(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP INDEX failure_checks_failure")
        .unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("failure_checks_failure"), "{err}");
    assert_eq!(user_version(&conn), 6, "left at version 6");
    assert!(
        index_sql(&conn, "failures_created").is_none(),
        "no half upgrade"
    );
    cleanup(&path);
}

#[test]
fn the_failures_list_reads_the_created_index_backwards_and_sorts_nothing() {
    let path = temp_db("plan-list");
    let s = SqliteStore::open(&path).unwrap();
    for (offset, limit) in [(0, 51), (1000, 501), (100_000, 51)] {
        let plan = s
            .explain_failures_page(&FailureFilter::default(), offset, limit)
            .unwrap();
        assert!(
            plan.iter()
                .any(|p| p.contains("SCAN failures USING INDEX failures_created")),
            "{plan:?}"
        );
        assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
    }
    make_v6(&path);
    let plan = s
        .explain_failures_page(&FailureFilter::default(), 0, 51)
        .unwrap();
    assert!(
        plan.iter().any(|p| p.contains("TEMP B-TREE")),
        "without the index the list is sorted whole: {plan:?}"
    );
    drop(s);
    cleanup(&path);
}

#[test]
fn the_property_filter_reads_the_window_violations_and_the_matched_runs_failures_only() {
    let path = temp_db("plan-property");
    let s = SqliteStore::open(&path).unwrap();
    let plan = s.explain_failures_page(&property("db."), 0, 51).unwrap();
    for wanted in [
        "SCAN runs USING INDEX runs_created",
        "USING COVERING INDEX run_properties_violation (run_id=?)",
        "SEARCH failures USING INDEX failures_run (run_id=?)",
    ] {
        assert!(
            plan.iter().any(|p| p.contains(wanted)),
            "{wanted}: {plan:?}"
        );
    }
    assert!(
        !plan.iter().any(|p| p.starts_with("SCAN failures")),
        "never a walk over every failure: {plan:?}"
    );
    let named = FailureFilter {
        run_id: Some("run_0000000000000000".into()),
        ..property("db.")
    };
    let plan = s.explain_failures_page(&named, 0, 51).unwrap();
    assert!(
        plan.iter()
            .any(|p| p.contains("run_properties_violation (run_id=?)")),
        "{plan:?}"
    );
    assert!(!plan.iter().any(|p| p.starts_with("SCAN")), "{plan:?}");
    drop(s);
    cleanup(&path);
}

#[test]
fn downgrading_a_version_seven_database_drops_its_indexes() {
    let path = temp_db("down");
    drop(SqliteStore::open(&path).unwrap());
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!(
        (report.from_version, report.to_version),
        (CURRENT_VERSION, 2)
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 2);
    for index in V7_INDEXES {
        assert!(index_sql(&conn, index).is_none(), "{index} dropped");
    }
    drop(conn);
    // And it migrates forward again to the same indexes.
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    for index in V7_INDEXES {
        assert!(index_sql(&conn, index).is_some(), "{index} back");
    }
    cleanup(&path);
}

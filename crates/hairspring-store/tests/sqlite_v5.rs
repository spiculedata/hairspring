//! Schema version 5 on SQLite: the stored results of confirming a failure. The migration only adds a table, so a version
//! 4 database keeps every row it has, and opening a current database changes nothing.
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const CURRENT_VERSION: i64 = 5;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-v5-{name}-{}-{}",
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

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

fn ddl_of(conn: &Connection, table: &str) -> String {
    conn.query_row("SELECT sql FROM sqlite_master WHERE name=?", [table], |r| {
        r.get(0)
    })
    .unwrap()
}

/// A genuine version 4 database holding one run and one failure, and nothing of version 5.
fn make_v4_database(path: &Path) -> (String, String) {
    let s = SqliteStore::open(path).unwrap();
    let run = s
        .create_run(
            &NewRun {
                target: "legacy".into(),
                backend: "hermit".into(),
                seed: 9,
                config_json: "{}".into(),
            },
            1000,
        )
        .unwrap();
    let bundle = s.put_blob(b"bundle").unwrap();
    let failure = s
        .add_failure(
            &NewFailure {
                run_id: run.id.clone(),
                target: "legacy".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            1001,
        )
        .unwrap();
    drop(s);
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("DROP TABLE failure_checks; PRAGMA user_version = 4;")
        .unwrap();
    (run.id, failure.id)
}

fn one_confirmation(failure_id: &str) -> NewConfirmation {
    NewConfirmation {
        id: "conf_a".into(),
        failure_id: failure_id.into(),
        runner: "local-1".into(),
        runner_trusted: true,
        checks: vec![NewCheck {
            kind: "same_seed_rerun".into(),
            status: "pass".into(),
            passed: 3,
            failed: 0,
            inconclusive: 0,
            command: "hermit run".into(),
            detail_json: "{}".into(),
            artefacts_json: "[]".into(),
            environment_json: "{}".into(),
            started_at: 1,
            finished_at: 2,
        }],
    }
}

#[test]
fn a_fresh_database_is_current_with_the_checks_table() {
    let path = temp_db("fresh");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    assert!(table_exists(&conn, "failure_checks"));
    cleanup(&path);
}

#[test]
fn a_version_four_database_migrates_in_place_and_keeps_its_rows() {
    let path = temp_db("v4");
    let (run_id, failure_id) = make_v4_database(&path);
    let before = Connection::open(&path).unwrap();
    let (runs, failures, trials) = (
        ddl_of(&before, "runs"),
        ddl_of(&before, "failures"),
        ddl_of(&before, "trials"),
    );
    drop(before);

    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run(&run_id).unwrap().unwrap().target, "legacy");
    assert!(s.get_failure(&failure_id).unwrap().is_some());
    assert!(
        s.list_failure_checks(&failure_id).unwrap().is_empty(),
        "an old failure reads back with no checks"
    );
    assert_eq!(
        s.add_confirmation(&one_confirmation(&failure_id), 5)
            .unwrap(),
        ConfirmationOutcome::Stored
    );
    drop(s);

    let after = Connection::open(&path).unwrap();
    assert_eq!(user_version(&after), CURRENT_VERSION);
    assert!(table_exists(&after, "failure_checks"));
    assert_eq!(ddl_of(&after, "runs"), runs);
    assert_eq!(ddl_of(&after, "failures"), failures);
    assert_eq!(ddl_of(&after, "trials"), trials);
    cleanup(&path);
}

#[test]
fn opening_a_current_database_runs_no_ddl() {
    let path = temp_db("idempotent");
    drop(SqliteStore::open(&path).unwrap());
    let counter = schema_counter(&Connection::open(&path).unwrap());
    for _ in 0..3 {
        drop(SqliteStore::open(&path).unwrap());
    }
    assert_eq!(schema_counter(&Connection::open(&path).unwrap()), counter);
    cleanup(&path);
}

#[test]
fn a_database_newer_than_this_build_is_refused_untouched() {
    let path = temp_db("v6");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", CURRENT_VERSION + 1)
        .unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains('6'), "{err}");
    assert_eq!(user_version(&conn), CURRENT_VERSION + 1);
    assert!(table_exists(&conn, "failure_checks"), "left alone");
    cleanup(&path);
}

#[test]
fn a_current_database_that_lost_the_table_or_its_index_is_reported_not_repaired() {
    let path = temp_db("lost-table");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE failure_checks").unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("failure_checks"), "{err}");
    assert!(!table_exists(&conn, "failure_checks"), "not recreated");
    cleanup(&path);

    let path = temp_db("lost-index");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP INDEX failure_checks_failure")
        .unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("failure_checks_failure"), "{err}");
    cleanup(&path);
}

#[test]
fn a_version_four_database_missing_a_version_four_table_is_refused_not_repaired() {
    let path = temp_db("v4-lost");
    make_v4_database(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE run_coverage").unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("run_coverage"), "{err}");
    assert_eq!(user_version(&conn), 4, "left at version 4");
    assert!(!table_exists(&conn, "failure_checks"), "no half upgrade");
    cleanup(&path);
}

#[test]
fn downgrading_a_version_five_database_drops_the_checks_and_the_version_four_tables() {
    let path = temp_db("down-from-5");
    let s = SqliteStore::open(&path).unwrap();
    let run = s
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = s.put_blob(b"b").unwrap();
    let failure = s
        .add_failure(
            &NewFailure {
                run_id: run.id,
                target: "t".into(),
                message: "m".into(),
                original_len: 1,
                shrunk_len: 1,
                bundle_hash: bundle,
                steps_json: "[]".into(),
            },
            2,
        )
        .unwrap();
    s.add_confirmation(&one_confirmation(&failure.id), 3)
        .unwrap();
    drop(s);

    let dry = downgrade_sqlite(
        &path,
        &DowngradeOptions {
            dry_run: true,
            ..DowngradeOptions::default()
        },
    )
    .unwrap();
    assert_eq!((dry.from_version, dry.applied), (5, false));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        user_version(&conn),
        CURRENT_VERSION,
        "a dry run changes nothing"
    );
    assert!(table_exists(&conn, "failure_checks"));
    drop(conn);

    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (5, 2));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 2);
    for t in ["failure_checks", "run_digest_use", "run_coverage"] {
        assert!(!table_exists(&conn, t), "{t} is dropped");
    }
    drop(conn);

    // Upgrading again restores the tables, empty.
    let s = SqliteStore::open(&path).unwrap();
    assert!(s.list_failure_checks(&failure.id).unwrap().is_empty());
    cleanup(&path);
}

#[test]
fn a_version_four_database_still_downgrades_to_two() {
    let path = temp_db("down-from-4");
    make_v4_database(&path);
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (4, 2));
    assert_eq!(user_version(&Connection::open(&path).unwrap()), 2);
    cleanup(&path);
}

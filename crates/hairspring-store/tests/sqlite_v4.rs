//! Schema version 4 on SQLite: the per-run digest allowance and the stored coverage series. The migration only adds
//! tables, so a version 3 database keeps every row it has.
mod common;
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const NEW_TABLES: [&str; 2] = ["run_digest_use", "run_coverage"];

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-v4-{name}-{}-{}",
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

/// A genuine version 3 database: one run, and none of the version 4 tables.
fn make_v3_database(path: &Path) -> String {
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
    drop(s);
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "DROP TABLE run_digest_use; DROP TABLE run_coverage; PRAGMA user_version = 3;",
    )
    .unwrap();
    run.id
}

/// The version this build creates: version 5 only adds a table to version 4.
const CURRENT_VERSION: i64 = 5;

/// A genuine version 4 database: nothing of version 5.
fn make_v4_database(path: &Path) {
    drop(SqliteStore::open(path).unwrap());
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("DROP TABLE failure_checks; PRAGMA user_version = 4;")
        .unwrap();
}

#[test]
fn a_fresh_database_has_the_version_four_tables() {
    let path = temp_db("fresh");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    for t in NEW_TABLES {
        assert!(table_exists(&conn, t), "{t}");
    }
    cleanup(&path);
}

#[test]
fn a_version_three_database_migrates_in_place_and_keeps_its_rows() {
    let path = temp_db("v3");
    let run_id = make_v3_database(&path);
    let before = Connection::open(&path).unwrap();
    let (runs_ddl, trials_ddl) = (ddl_of(&before, "runs"), ddl_of(&before, "trials"));
    drop(before);

    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run(&run_id).unwrap().unwrap().target, "legacy");
    assert_eq!(s.run_digest_use(&run_id).unwrap(), DigestUse::default());
    assert_eq!(s.run_coverage_json(&run_id).unwrap(), None);
    drop(s);

    let after = Connection::open(&path).unwrap();
    assert_eq!(user_version(&after), CURRENT_VERSION);
    for t in NEW_TABLES {
        assert!(table_exists(&after, t), "{t}");
    }
    assert_eq!(ddl_of(&after, "runs"), runs_ddl, "runs is not altered");
    assert_eq!(
        ddl_of(&after, "trials"),
        trials_ddl,
        "trials is not altered"
    );
    cleanup(&path);
}

#[test]
fn a_migrated_database_takes_ingest_and_keeps_the_allowance() {
    let path = temp_db("v3-ingest");
    let run_id = make_v3_database(&path);
    let s = SqliteStore::open(&path).unwrap();
    s.enqueue_job(&run_id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w1", 600, 10).unwrap().unwrap();
    s.mark_running(&run_id, 10).unwrap();
    let batch = common::explorer::batch_of_digests(&[4, 8], 10);
    s.ingest_trials(&job.id, "w1", true, &batch, 20).unwrap();
    assert_eq!(
        s.run_digest_use(&run_id).unwrap(),
        DigestUse {
            bytes: 6,
            capped_trials: 1
        }
    );
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
    for t in NEW_TABLES {
        assert!(table_exists(&conn, t), "{t} left alone");
    }
    cleanup(&path);
}

#[test]
fn a_version_four_database_that_lost_a_table_is_reported_not_repaired() {
    for lost in NEW_TABLES {
        let path = temp_db(&format!("lost-{lost}"));
        drop(SqliteStore::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("DROP TABLE {lost}")).unwrap();
        let err = SqliteStore::open(&path).err().expect("must refuse");
        assert!(err.to_string().contains(lost), "{err}");
        assert!(!table_exists(&conn, lost), "not recreated");
        cleanup(&path);
    }
}

#[test]
fn a_version_three_database_missing_a_version_three_table_is_refused_not_repaired() {
    let path = temp_db("v3-lost");
    make_v3_database(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE tree_branches").unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("tree_branches"), "{err}");
    assert_eq!(user_version(&conn), 3, "left at version 3");
    assert!(!table_exists(&conn, "tree_branches"), "not recreated");
    assert!(!table_exists(&conn, "run_digest_use"), "no half upgrade");
    cleanup(&path);
}

#[test]
fn a_version_three_database_still_downgrades_to_two() {
    let path = temp_db("down-from-3");
    make_v3_database(&path);
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (3, 2));
    assert!(report.applied);
    assert_eq!(user_version(&Connection::open(&path).unwrap()), 2);
    cleanup(&path);
}

#[test]
fn downgrading_a_version_four_database_drops_the_new_tables() {
    let path = temp_db("down-from-4");
    make_v4_database(&path);
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (4, 2));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 2);
    for t in NEW_TABLES {
        assert!(!table_exists(&conn, t), "{t} is dropped");
    }
    cleanup(&path);
}

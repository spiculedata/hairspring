//! Schema version 8 on SQLite: the `run_inputs` table and the index `runs_target_created`, and nothing else. The
//! migration rewrites no row, runs stored earlier read as "no inputs recorded" (never as a guess), a table or index of
//! the same name but another definition is refused with the version left at 7, and a current database that lost either
//! is reported, not repaired.
mod common;
use hairspring_store::*;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CURRENT_VERSION: i64 = 8;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-v8-{name}-{}-{}",
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

fn object_sql(conn: &Connection, kind: &str, name: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type=? AND name=?",
        [kind, name],
        |r| r.get(0),
    )
    .ok()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// Turns a current database into what a version 7 build leaves behind: no `run_inputs`, no `runs_target_created`.
fn make_v7(path: &Path) {
    Connection::open(path)
        .unwrap()
        .execute_batch(
            "DROP INDEX runs_target_created; DROP TABLE run_inputs; PRAGMA user_version = 7;",
        )
        .unwrap();
}

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn new_run(s: &SqliteStore, target: &str, at: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: target.into(),
            backend: "sim".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        at,
    )
    .unwrap()
}

fn inputs(run: &str) -> NewRunInputs {
    NewRunInputs {
        run_id: run.into(),
        scheme: "sim/1".into(),
        fields: BTreeMap::from([("hairspring_sha".to_string(), digest('a'))]),
        missing: Vec::new(),
        reporter_id: "local-1".into(),
        reporter_trusted: true,
        attempt: 1,
    }
}

#[test]
fn a_fresh_database_is_current_with_the_table_and_the_index() {
    let path = temp_db("fresh");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    assert!(object_sql(&conn, "table", "run_inputs").is_some());
    assert_eq!(
        object_sql(&conn, "index", "runs_target_created").as_deref(),
        Some("CREATE INDEX runs_target_created ON runs(target, created_at)")
    );
    cleanup(&path);
}

#[test]
fn a_version_seven_database_with_runs_migrates_in_place_and_old_runs_have_no_inputs() {
    let path = temp_db("data");
    let s = SqliteStore::open(&path).unwrap();
    let old = [new_run(&s, "a", 10), new_run(&s, "b", 20)];
    drop(s);
    make_v7(&path);
    let before = Connection::open(&path).unwrap();
    let runs_ddl = object_sql(&before, "table", "runs");
    let rows = count(&before, "runs");
    drop(before);

    let s = SqliteStore::open(&path).unwrap();
    for run in &old {
        assert_eq!(s.get_run_inputs(&run.id).unwrap(), None, "never invented");
        assert!(s.get_run(&run.id).unwrap().is_some());
    }
    // The migrated database records inputs for an old run and for a new one.
    assert_eq!(
        s.put_run_inputs(&inputs(&old[0].id), 99).unwrap(),
        RunInputsOutcome::Stored
    );
    drop(s);

    let after = Connection::open(&path).unwrap();
    assert_eq!(user_version(&after), CURRENT_VERSION);
    assert_eq!(
        object_sql(&after, "table", "runs"),
        runs_ddl,
        "no column added"
    );
    assert_eq!(count(&after, "runs"), rows);
    assert_eq!(count(&after, "run_inputs"), 1);
    cleanup(&path);
}

#[test]
fn opening_a_migrated_database_again_runs_no_ddl() {
    let path = temp_db("again");
    drop(SqliteStore::open(&path).unwrap());
    make_v7(&path);
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

/// `IF NOT EXISTS` goes by name. A table or index of the right name with another definition must not pass for the
/// version 8 object, whether it was there before the migration or replaced one afterwards.
#[test]
fn a_table_or_index_of_the_same_name_but_another_definition_is_refused() {
    let tables = [
        // A column missing, an extra column, another type, another nullability, no key, another key, no reference.
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL, extra TEXT)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json BLOB NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT NOT NULL REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY, scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES failures(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL, CHECK (attempt > 0))",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL, UNIQUE (fields_json))",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE, scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id) ON UPDATE CASCADE, scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL) WITHOUT ROWID",
        "CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL) STRICT",
        "CREATE TABLE run_inputs (scheme TEXT PRIMARY KEY REFERENCES runs(id), run_id TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted INTEGER NOT NULL, attempt INTEGER NOT NULL, created_at INTEGER NOT NULL)",
    ];
    for sql in tables {
        let path = temp_db("wrong-table");
        drop(SqliteStore::open(&path).unwrap());
        make_v7(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        let err = SqliteStore::open(&path)
            .err()
            .unwrap_or_else(|| panic!("{sql}: must refuse"));
        assert!(
            err.to_string()
                .contains("table run_inputs exists but has a different definition"),
            "{sql}: {err}"
        );
        assert_eq!(user_version(&conn), 7, "{sql}: no half upgrade");
        assert!(
            object_sql(&conn, "index", "runs_target_created").is_none(),
            "{sql}: the index was rolled back with the rest"
        );
        drop(conn);
        cleanup(&path);
    }
    let indexes = [
        "CREATE INDEX runs_target_created ON runs(created_at)",
        "CREATE INDEX runs_target_created ON runs(created_at, target)",
        "CREATE INDEX runs_target_created ON runs(target)",
        "CREATE UNIQUE INDEX runs_target_created ON runs(target, created_at)",
        "CREATE INDEX runs_target_created ON runs(target, created_at) WHERE state = 'passed'",
        "CREATE INDEX runs_target_created ON runs(target, created_at DESC)",
        "CREATE INDEX runs_target_created ON runs(target COLLATE NOCASE, created_at)",
        "CREATE INDEX runs_target_created ON failures(target, created_at)",
    ];
    for sql in indexes {
        let path = temp_db("wrong-index");
        drop(SqliteStore::open(&path).unwrap());
        make_v7(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(sql).unwrap();
        let err = SqliteStore::open(&path)
            .err()
            .unwrap_or_else(|| panic!("{sql}: must refuse"));
        assert!(
            err.to_string().contains(
                "index runs_target_created exists but is invalid or has a different definition: drop it"
            ),
            "{sql}: {err}"
        );
        assert_eq!(user_version(&conn), 7, "{sql}: no half upgrade");
        assert!(
            object_sql(&conn, "table", "run_inputs").is_none(),
            "{sql}: the table was rolled back with the rest"
        );
        drop(conn);
        cleanup(&path);

        // Replaced after the migration: refused on the next open too.
        let path = temp_db("replaced-index");
        drop(SqliteStore::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("DROP INDEX runs_target_created; {sql}"))
            .unwrap();
        let err = SqliteStore::open(&path)
            .err()
            .unwrap_or_else(|| panic!("{sql}: must refuse"));
        assert!(
            err.to_string().contains("different definition"),
            "{sql}: {err}"
        );
        drop(conn);
        cleanup(&path);
    }
}

#[test]
fn the_same_table_and_index_built_beforehand_are_kept() {
    let path = temp_db("prebuilt");
    drop(SqliteStore::open(&path).unwrap());
    make_v7(&path);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "create table run_inputs ( run_id text primary key references runs ( id ), scheme text not null,
               fields_json text not null, missing_json text not null, reporter_id text not null,
               reporter_trusted integer not null, attempt integer not null, created_at integer not null );
             create index runs_target_created on runs ( target , created_at )",
        )
        .unwrap();
    drop(SqliteStore::open(&path).unwrap());
    assert_eq!(
        user_version(&Connection::open(&path).unwrap()),
        CURRENT_VERSION
    );
    cleanup(&path);
}

#[test]
fn a_current_database_that_lost_a_version_eight_object_is_reported_not_repaired() {
    for (drop_sql, kind, name) in [
        ("DROP TABLE run_inputs", "table", "run_inputs"),
        (
            "DROP INDEX runs_target_created",
            "index",
            "runs_target_created",
        ),
    ] {
        let path = temp_db("lost");
        drop(SqliteStore::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(drop_sql).unwrap();
        let err = SqliteStore::open(&path).err().expect("must refuse");
        assert!(err.to_string().contains(name), "{err}");
        assert!(
            object_sql(&conn, kind, name).is_none(),
            "{name} not recreated"
        );
        cleanup(&path);
    }
}

#[test]
fn a_version_seven_database_missing_a_version_seven_index_is_refused_not_repaired() {
    let path = temp_db("v7-lost");
    drop(SqliteStore::open(&path).unwrap());
    make_v7(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP INDEX failures_created").unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("failures_created"), "{err}");
    assert_eq!(user_version(&conn), 7, "left at version 7");
    assert!(object_sql(&conn, "table", "run_inputs").is_none());
    cleanup(&path);
}

#[test]
fn the_index_serves_the_runs_of_one_target_newest_first_without_a_sort() {
    let path = temp_db("plan");
    let s = SqliteStore::open(&path).unwrap();
    for (n, target) in ["a", "b", "a", "c", "a"].into_iter().enumerate() {
        new_run(&s, target, 100 + n as i64);
    }
    drop(s);
    let conn = Connection::open(&path).unwrap();
    let plan: Vec<String> = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT id FROM runs WHERE target = ? ORDER BY created_at DESC, rowid DESC LIMIT 11",
        )
        .unwrap()
        .query_map(["a"], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<_, rusqlite::Error>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|p| p.contains("USING INDEX runs_target_created (target=?)")),
        "{plan:?}"
    );
    assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
    cleanup(&path);
}

#[test]
fn downgrading_a_version_eight_database_drops_its_table_and_index() {
    let path = temp_db("down");
    let s = SqliteStore::open(&path).unwrap();
    let run = new_run(&s, "t", 1);
    s.put_run_inputs(&inputs(&run.id), 2).unwrap();
    drop(s);
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!(
        (report.from_version, report.to_version),
        (CURRENT_VERSION, 2)
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 2);
    assert!(object_sql(&conn, "table", "run_inputs").is_none());
    assert!(object_sql(&conn, "index", "runs_target_created").is_none());
    assert_eq!(count(&conn, "runs"), 1, "the run itself stays");
    drop(conn);
    // It migrates forward again; the recorded inputs went with the table, so the run reads as not recorded.
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run_inputs(&run.id).unwrap(), None);
    drop(s);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), CURRENT_VERSION);
    cleanup(&path);
}

#[test]
fn downgrading_a_version_seven_database_still_works() {
    let path = temp_db("down7");
    drop(SqliteStore::open(&path).unwrap());
    make_v7(&path);
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (7, 2));
    cleanup(&path);
}

/// A recorded input is a digest. What a caller offers in its place is refused, so a value (a path, an environment
/// entry) cannot reach the database file by this route.
#[test]
fn a_planted_value_is_refused_and_never_reaches_the_file() {
    let path = temp_db("plant");
    let s = SqliteStore::open(&path).unwrap();
    let run = new_run(&s, "t", 1);
    let secret = "TOPSECRET-value-9876";
    let planted = NewRunInputs {
        fields: BTreeMap::from([("guest_env".to_string(), secret.to_string())]),
        ..inputs(&run.id)
    };
    assert!(matches!(
        s.put_run_inputs(&planted, 2).unwrap(),
        RunInputsOutcome::Rejected(RunInputsRejection::Invalid(_))
    ));
    drop(s);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").ok();
    drop(conn);
    for file in std::fs::read_dir(path.parent().unwrap()).unwrap() {
        let bytes = std::fs::read(file.unwrap().path()).unwrap();
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "the value reached a database file"
        );
    }
    cleanup(&path);
}

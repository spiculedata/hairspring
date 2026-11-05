//! Schema version 6 on SQLite: `runs.mode` holds the mode the run was started in. New runs store it from their
//! `config_json`, and the migration backfills runs created while the column was never written.
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const CURRENT_VERSION: i64 = 7;
const SWEEP_CONFIG: &str = r#"{"target":"t","seed":1,"budget":null,"mode":"sweep"}"#;
const SEARCH_CONFIG: &str = r#"{"target":"t","seed":1,"budget":null,"mode":"search"}"#;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-v6-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.sqlite")
}

fn cleanup(path: &Path) {
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

fn new_run(config_json: &str) -> NewRun {
    NewRun {
        target: "t".into(),
        backend: "sim".into(),
        seed: 1,
        config_json: config_json.into(),
    }
}

fn mode_in(conn: &Connection, id: &str) -> String {
    conn.query_row("SELECT mode FROM runs WHERE id=?", [id], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_new_run_stores_the_mode_named_in_its_config() {
    let path = temp_db("new");
    let s = SqliteStore::open(&path).unwrap();
    let sweep = s.create_run(&new_run(SWEEP_CONFIG), 1).unwrap();
    let search = s.create_run(&new_run(SEARCH_CONFIG), 2).unwrap();
    let bare = s.create_run(&new_run("{}"), 3).unwrap();
    assert_eq!(sweep.mode, "sweep");
    assert_eq!(search.mode, "search");
    assert_eq!(bare.mode, "search");
    assert_eq!(s.get_run(&sweep.id).unwrap().unwrap().mode, "sweep");
    cleanup(&path);
}

#[test]
fn a_config_that_is_not_json_or_names_another_mode_is_stored_as_a_search() {
    let path = temp_db("odd");
    let s = SqliteStore::open(&path).unwrap();
    for config in [
        "not json",
        "{\"mode\":",
        "[]",
        "null",
        r#"{"mode":3}"#,
        r#"{"mode":"SWEEP"}"#,
    ] {
        let run = s.create_run(&new_run(config), 1).unwrap();
        assert_eq!(run.mode, "search", "{config}");
    }
    cleanup(&path);
}

#[test]
fn a_version_five_database_backfills_the_mode_of_its_sweeps_only() {
    let path = temp_db("backfill");
    let s = SqliteStore::open(&path).unwrap();
    let ids: Vec<String> = [
        SWEEP_CONFIG,
        SEARCH_CONFIG,
        "not json",
        "{\"mode\":\"sweep\"",
    ]
    .iter()
    .map(|c| s.create_run(&new_run(c), 1).unwrap().id)
    .collect();
    let branch = s.create_run(&new_run(SWEEP_CONFIG), 2).unwrap().id;
    drop(s);
    // What a version 5 build left behind: every mode is the column default, except one set to something else.
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("UPDATE runs SET mode='search'; PRAGMA user_version = 5;")
        .unwrap();
    conn.execute("UPDATE runs SET mode='branch' WHERE id=?", [&branch])
        .unwrap();
    drop(conn);

    drop(SqliteStore::open(&path).unwrap());

    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, CURRENT_VERSION);
    let modes: Vec<String> = ids.iter().map(|id| mode_in(&conn, id)).collect();
    assert_eq!(modes, ["sweep", "search", "search", "search"]);
    assert_eq!(
        mode_in(&conn, &branch),
        "branch",
        "a mode that is not the default is left alone"
    );
    cleanup(&path);
}

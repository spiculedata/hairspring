//! Schema version 6 on PostgreSQL: `runs.mode` holds the mode the run was started in. Skipped unless
//! `HAIRSPRING_TEST_PG_URL` is set (see `postgres.rs`). New runs store it from their `config_json`, and the migration
//! backfills runs created while the column was never written.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use hairspring_store::*;
use postgres::Client;

const CURRENT_VERSION: i32 = 6;
const SWEEP_CONFIG: &str = r#"{"target":"t","seed":1,"budget":null,"mode":"sweep"}"#;
const SEARCH_CONFIG: &str = r#"{"target":"t","seed":1,"budget":null,"mode":"search"}"#;

fn new_run(config_json: &str) -> NewRun {
    NewRun {
        target: "t".into(),
        backend: "sim".into(),
        seed: 1,
        config_json: config_json.into(),
    }
}

fn recorded_version(c: &mut Client, schema: &str) -> i32 {
    c.query_one(
        &format!("SELECT version FROM \"{schema}\".schema_version"),
        &[],
    )
    .unwrap()
    .get(0)
}

fn mode_in(c: &mut Client, schema: &str, id: &str) -> String {
    c.query_one(
        &format!("SELECT mode FROM \"{schema}\".runs WHERE id = $1"),
        &[&id],
    )
    .unwrap()
    .get(0)
}

#[test]
fn a_new_run_stores_the_mode_named_in_its_config() {
    let Some(url) = url() else { return };
    let s = PgStore::connect_in_schema(&url, &new_schema("v6n"), 1).unwrap();
    let sweep = s.create_run(&new_run(SWEEP_CONFIG), 1).unwrap();
    let search = s.create_run(&new_run(SEARCH_CONFIG), 2).unwrap();
    let bare = s.create_run(&new_run("{}"), 3).unwrap();
    assert_eq!(sweep.mode, "sweep");
    assert_eq!(search.mode, "search");
    assert_eq!(bare.mode, "search");
    assert_eq!(s.get_run(&sweep.id).unwrap().unwrap().mode, "sweep");
}

#[test]
fn a_config_that_is_not_json_or_names_another_mode_is_stored_as_a_search() {
    let Some(url) = url() else { return };
    let s = PgStore::connect_in_schema(&url, &new_schema("v6o"), 1).unwrap();
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
}

#[test]
fn a_version_five_schema_backfills_the_mode_of_its_sweeps_only() {
    let Some(url) = url() else { return };
    let schema = new_schema("v6b");
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
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
    let mut c = admin(&url);
    c.batch_execute(&format!(
        "UPDATE \"{schema}\".runs SET mode = 'search';
         UPDATE \"{schema}\".schema_version SET version = 5"
    ))
    .unwrap();
    c.execute(
        &format!("UPDATE \"{schema}\".runs SET mode = 'branch' WHERE id = $1"),
        &[&branch],
    )
    .unwrap();

    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());

    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    let modes: Vec<String> = ids.iter().map(|id| mode_in(&mut c, &schema, id)).collect();
    assert_eq!(modes, ["sweep", "search", "search", "search"]);
    assert_eq!(
        mode_in(&mut c, &schema, &branch),
        "branch",
        "a mode that is not the default is left alone"
    );
}

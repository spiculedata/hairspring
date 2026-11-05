//! Schema version 8 on PostgreSQL: the `run_inputs` table and the index `runs_target_created`, and nothing else.
//! Skipped unless `HAIRSPRING_TEST_PG_URL` is set (see `postgres.rs`). The migration rewrites no row, runs stored
//! earlier read as "no inputs recorded", a table or index of the same name but another definition is refused with the
//! version left at 7, and creating the index needs the role that owns the schema's tables.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use hairspring_store::*;
use postgres::Client;
use std::collections::BTreeMap;

const CURRENT_VERSION: i32 = 8;

fn recorded_version(c: &mut Client, schema: &str) -> i32 {
    c.query_one(
        &format!("SELECT version FROM \"{schema}\".schema_version"),
        &[],
    )
    .unwrap()
    .get(0)
}

fn index_def(c: &mut Client, schema: &str, index: &str) -> Option<String> {
    c.query_opt(
        "SELECT indexdef FROM pg_indexes WHERE schemaname = $1 AND indexname = $2",
        &[&schema, &index],
    )
    .unwrap()
    .map(|r| r.get(0))
}

fn table_exists(c: &mut Client, schema: &str, table: &str) -> bool {
    c.query_one(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2)",
        &[&schema, &table],
    )
    .unwrap()
    .get(0)
}

fn columns_of(c: &mut Client, schema: &str, table: &str) -> Vec<String> {
    c.query(
        "SELECT column_name || ' ' || data_type FROM information_schema.columns
          WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
        &[&schema, &table],
    )
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

fn count(c: &mut Client, schema: &str, table: &str) -> i64 {
    c.query_one(&format!("SELECT COUNT(*) FROM \"{schema}\".{table}"), &[])
        .unwrap()
        .get(0)
}

/// Turns a current schema into what a version 7 build leaves behind: no `run_inputs`, no `runs_target_created`.
fn make_v7(url: &str, schema: &str) {
    admin(url)
        .batch_execute(&format!(
            "DROP INDEX \"{schema}\".runs_target_created;
             DROP TABLE \"{schema}\".run_inputs;
             UPDATE \"{schema}\".schema_version SET version = 7"
        ))
        .unwrap();
}

/// The URL with its user replaced by `user` (no password), for the trust-authenticated test server.
fn url_as(url: &str, user: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("a postgres URL");
    let (_, host) = rest.rsplit_once('@').expect("a URL with a user");
    format!("{scheme}://{user}@{host}")
}

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn new_run(s: &PgStore, target: &str, at: i64) -> RunRecord {
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
    }
}

#[test]
fn a_fresh_schema_is_current_with_the_table_and_the_index() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8f");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    assert_eq!(
        columns_of(&mut c, &schema, "run_inputs"),
        [
            "run_id text",
            "scheme text",
            "fields_json text",
            "missing_json text",
            "reporter_id text",
            "reporter_trusted boolean",
            "created_at bigint"
        ]
    );
    let index = index_def(&mut c, &schema, "runs_target_created").unwrap();
    assert!(
        index.ends_with("USING btree (target, created_at, seq)"),
        "{index}"
    );
}

#[test]
fn a_version_seven_schema_with_runs_migrates_in_place_and_old_runs_have_no_inputs() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8m");
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let old = [new_run(&s, "a", 10), new_run(&s, "b", 20)];
    drop(s);
    make_v7(&url, &schema);
    let mut c = admin(&url);
    let columns = columns_of(&mut c, &schema, "runs");
    let rows = count(&mut c, &schema, "runs");

    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    for run in &old {
        assert_eq!(s.get_run_inputs(&run.id).unwrap(), None, "never invented");
    }
    assert_eq!(
        s.put_run_inputs(&inputs(&old[0].id), 99).unwrap(),
        RunInputsOutcome::Stored
    );
    drop(s);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    assert_eq!(
        columns_of(&mut c, &schema, "runs"),
        columns,
        "no column added"
    );
    assert_eq!(count(&mut c, &schema, "runs"), rows);
    assert_eq!(count(&mut c, &schema, "run_inputs"), 1);
    assert!(index_def(&mut c, &schema, "runs_target_created").is_some());
}

#[test]
fn opening_a_current_schema_again_changes_nothing() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8a");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v7(&url, &schema);
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    let oid = |c: &mut Client| -> u32 {
        c.query_one(
            "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE n.nspname = $1 AND c.relname = 'runs_target_created'",
            &[&schema],
        )
        .unwrap()
        .get(0)
    };
    let before = oid(&mut c);
    for _ in 0..3 {
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    }
    assert_eq!(oid(&mut c), before, "the index was not rebuilt");
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
}

/// `IF NOT EXISTS` goes by name. A table or index of the right name with another definition must not pass for the
/// version 8 object: the open is refused and the schema, table and index included, stays as a version 7 build left it.
#[test]
fn a_table_or_index_of_the_same_name_but_another_definition_is_refused() {
    let Some(url) = url() else { return };
    let columns = "scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, \
                   reporter_id TEXT NOT NULL, reporter_trusted BOOLEAN NOT NULL, created_at BIGINT NOT NULL";
    let tables = [
        // A column missing, an extra one, another type, another nullability, no key, a key on two columns, no reference.
        "run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL, fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL, reporter_trusted BOOLEAN NOT NULL".to_string(),
        format!("run_id TEXT PRIMARY KEY REFERENCES runs(id), {columns}, extra TEXT"),
        format!("run_id TEXT PRIMARY KEY REFERENCES runs(id), {}", columns.replace("fields_json TEXT", "fields_json VARCHAR")),
        format!("run_id TEXT PRIMARY KEY REFERENCES runs(id), {}", columns.replace("scheme TEXT NOT NULL", "scheme TEXT")),
        format!("run_id TEXT NOT NULL REFERENCES runs(id), {columns}"),
        format!("run_id TEXT NOT NULL REFERENCES runs(id), {columns}, PRIMARY KEY (run_id, scheme)"),
        format!("run_id TEXT PRIMARY KEY, {columns}"),
        format!("run_id TEXT PRIMARY KEY REFERENCES failures(id), {columns}"),
    ];
    for def in tables {
        let schema = new_schema("v8t");
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
        make_v7(&url, &schema);
        let mut c = admin(&url);
        c.batch_execute(&format!(
            "SET search_path = \"{schema}\"; CREATE TABLE run_inputs ({def})"
        ))
        .unwrap();
        let err = PgStore::connect_in_schema(&url, &schema, 1)
            .err()
            .unwrap_or_else(|| panic!("{def}: must refuse"));
        assert!(
            err.to_string()
                .contains("table run_inputs exists but has a different definition"),
            "{def}: {err}"
        );
        assert_eq!(
            recorded_version(&mut c, &schema),
            7,
            "{def}: no half upgrade"
        );
        assert!(
            index_def(&mut c, &schema, "runs_target_created").is_none(),
            "{def}: the index was rolled back with the rest"
        );
    }
    let indexes = [
        "CREATE INDEX runs_target_created ON runs(created_at, seq)",
        "CREATE INDEX runs_target_created ON runs(target, created_at)",
        "CREATE INDEX runs_target_created ON runs(target, seq, created_at)",
        "CREATE UNIQUE INDEX runs_target_created ON runs(target, created_at, seq)",
        "CREATE INDEX runs_target_created ON runs(target, created_at, seq) WHERE state = 'passed'",
        "CREATE INDEX runs_target_created ON runs(target, created_at DESC, seq)",
        "CREATE INDEX runs_target_created ON failures(created_at, seq)",
    ];
    for sql in indexes {
        let schema = new_schema("v8i");
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
        make_v7(&url, &schema);
        let mut c = admin(&url);
        c.batch_execute(&format!("SET search_path = \"{schema}\"; {sql}"))
            .unwrap();
        let err = PgStore::connect_in_schema(&url, &schema, 1)
            .err()
            .unwrap_or_else(|| panic!("{sql}: must refuse"));
        assert!(
            err.to_string().contains(
                "index runs_target_created exists but is invalid or has a different definition: drop it"
            ),
            "{sql}: {err}"
        );
        assert_eq!(
            recorded_version(&mut c, &schema),
            7,
            "{sql}: no half upgrade"
        );
        assert!(
            !table_exists(&mut c, &schema, "run_inputs"),
            "{sql}: the table was rolled back with the rest"
        );
    }
}

/// An index a failed `CREATE INDEX CONCURRENTLY` left invalid is refused too, not taken for the real one.
#[test]
fn an_invalid_index_of_the_right_name_is_refused() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8v");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v7(&url, &schema);
    let mut c = admin(&url);
    c.batch_execute(&format!(
        "SET search_path = \"{schema}\";
         CREATE INDEX runs_target_created ON runs(target, created_at, seq);
         UPDATE pg_index SET indisvalid = FALSE
          WHERE indexrelid = (SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                               WHERE n.nspname = '{schema}' AND c.relname = 'runs_target_created')"
    ))
    .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(
        err.to_string()
            .contains("invalid or has a different definition"),
        "{err}"
    );
    assert_eq!(recorded_version(&mut c, &schema), 7);
}

#[test]
fn the_same_table_and_index_built_beforehand_are_kept() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8k");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v7(&url, &schema);
    let mut c = admin(&url);
    c.batch_execute(&format!(
        "SET search_path = \"{schema}\";
         CREATE TABLE run_inputs (run_id TEXT PRIMARY KEY REFERENCES runs(id), scheme TEXT NOT NULL,
           fields_json TEXT NOT NULL, missing_json TEXT NOT NULL, reporter_id TEXT NOT NULL,
           reporter_trusted BOOLEAN NOT NULL, created_at BIGINT NOT NULL);
         CREATE INDEX runs_target_created ON runs(target, created_at, seq)"
    ))
    .unwrap();
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
}

#[test]
fn a_current_schema_that_lost_a_version_eight_object_is_reported_not_repaired() {
    let Some(url) = url() else { return };
    for (drop_sql, name) in [
        ("DROP TABLE \"{s}\".run_inputs", "run_inputs"),
        (
            "DROP INDEX \"{s}\".runs_target_created",
            "runs_target_created",
        ),
    ] {
        let schema = new_schema("v8l");
        drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
        let mut c = admin(&url);
        c.batch_execute(&drop_sql.replace("{s}", &schema)).unwrap();
        let err = PgStore::connect_in_schema(&url, &schema, 1)
            .err()
            .expect("must refuse");
        assert!(err.to_string().contains(name), "{err}");
        assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
        assert!(
            index_def(&mut c, &schema, name).is_none() && !table_exists(&mut c, &schema, name),
            "{name} not recreated"
        );
    }
}

/// Creating the index needs ownership of `runs`. A role that only reads and writes rows can open a current schema (no
/// DDL is issued) and record inputs, but cannot be the one that migrates a version 7 schema: the open fails, the schema
/// stays at version 7 with no `run_inputs` table left behind, and the owner's open then migrates it.
#[test]
fn only_the_owner_can_migrate_a_version_seven_schema_and_any_role_then_records_inputs() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8o");
    let role = format!("hs_v8_{}", &new_id("u").replace('-', "_")[..12]);
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    let run = new_run(&s, "t", 1);
    drop(s);
    make_v7(&url, &schema);
    let mut a = admin(&url);
    a.batch_execute(&format!(
        "CREATE ROLE \"{role}\" LOGIN;
         GRANT USAGE, CREATE ON SCHEMA \"{schema}\" TO \"{role}\";
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA \"{schema}\" TO \"{role}\";
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA \"{schema}\" TO \"{role}\";"
    ))
    .unwrap();
    let as_role = url_as(&url, &role);
    let refused = PgStore::connect_in_schema(&as_role, &schema, 1)
        .err()
        .map(|e| e.to_string());
    let version_after_refusal = recorded_version(&mut a, &schema);
    let table_after_refusal = table_exists(&mut a, &schema, "run_inputs");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    // The tables the owner created are the owner's: the role is granted what it needs, as for the other tables.
    a.batch_execute(&format!(
        "GRANT SELECT, INSERT ON \"{schema}\".run_inputs TO \"{role}\""
    ))
    .unwrap();
    let recorded = PgStore::connect_in_schema(&as_role, &schema, 1)
        .map_err(|e| e.to_string())
        .and_then(|s| {
            s.put_run_inputs(&inputs(&run.id), 5)
                .map_err(|e| e.to_string())
        });
    a.batch_execute(&format!("DROP OWNED BY \"{role}\"; DROP ROLE \"{role}\""))
        .unwrap();
    let refused = refused.expect("a non-owner cannot create the index");
    assert!(refused.contains("owner"), "{refused}");
    assert_eq!(version_after_refusal, 7, "nothing half done");
    assert!(!table_after_refusal, "the table was rolled back");
    assert_eq!(recorded, Ok(RunInputsOutcome::Stored));
    assert_eq!(recorded_version(&mut a, &schema), CURRENT_VERSION);
}

/// 3000 runs over 30 targets, then `ANALYZE`, so the planner's choice is the one a real store gets.
fn bulk(url: &str, schema: &str) {
    raw(url, schema)
        .batch_execute(
            "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at)
               SELECT 'run_' || lpad(to_hex(n), 16, '0'), 't' || (n % 30), 'sim', 'failed', n, '{}', n / 3
               FROM generate_series(1, 3000) n;
             ANALYZE runs;",
        )
        .unwrap();
}

#[test]
fn the_index_serves_the_runs_of_one_target_newest_first_without_a_sort() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8p");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    bulk(&url, &schema);
    let mut c = raw(&url, &schema);
    let plan: Vec<String> = c
        .query(
            "EXPLAIN SELECT id FROM runs WHERE target = 't7' ORDER BY created_at DESC, seq DESC LIMIT 11",
            &[],
        )
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    let text = plan.join("\n");
    assert!(text.contains("runs_target_created"), "{text}");
    assert!(!text.contains("Sort"), "{text}");
}

#[test]
fn downgrading_a_version_eight_schema_drops_its_table_and_index() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8d");
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    let run = new_run(&s, "t", 1);
    s.put_run_inputs(&inputs(&run.id), 2).unwrap();
    drop(s);
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!(
        (report.from_version, report.to_version),
        (CURRENT_VERSION as i64, 2)
    );
    let mut c = admin(&url);
    assert!(!table_exists(&mut c, &schema, "run_inputs"));
    assert!(index_def(&mut c, &schema, "runs_target_created").is_none());
    assert_eq!(count(&mut c, &schema, "runs"), 1, "the run itself stays");
    let s = PgStore::connect_in_schema(&url, &schema, 1).unwrap();
    assert_eq!(s.get_run_inputs(&run.id).unwrap(), None);
    drop(s);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
}

#[test]
fn downgrading_a_version_seven_schema_still_works() {
    let Some(url) = url() else { return };
    let schema = new_schema("v8d7");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    make_v7(&url, &schema);
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert_eq!((report.from_version, report.to_version), (7, 2));
}

//! Schema version 3 on PostgreSQL: migration, open path, constraints and downgrade. Skipped unless
//! `HAIRSPRING_TEST_PG_URL` is set (see `postgres.rs`). Queries and plans are in `postgres_trees.rs`.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use common::tree_support::*;
use hairspring_store::*;
use postgres::Client;
use std::sync::Arc;

fn recorded_version(c: &mut Client, schema: &str) -> i32 {
    c.query_one(
        &format!("SELECT version FROM \"{schema}\".schema_version"),
        &[],
    )
    .unwrap()
    .get(0)
}

/// A genuine version 2 schema with data in the tables version 3 must leave alone.
fn make_v2_schema(url: &str, schema: &str) {
    let mut c = admin(url);
    c.batch_execute(&format!(
        "CREATE SCHEMA \"{schema}\"; SET search_path = \"{schema}\";"
    ))
    .unwrap();
    c.batch_execute(&common::v2_schema::postgres_v2()).unwrap();
    c.batch_execute(
        "CREATE TABLE schema_version (version INTEGER NOT NULL);
         INSERT INTO schema_version(version) VALUES (2);
         INSERT INTO runs(id,target,backend,state,seed,config_json,created_at,steps,trials,mode)
           VALUES ('run_old','legacy','hermit','passed',9,'{}',1000,50,1,'search');
         INSERT INTO trials(run_id,seq,seed,verdict,reason,timed_out,degraded,perturbing,report_missing,rated,
             claimable,syscalls,work_ops,elapsed_ms,worker_id,worker_trusted,attempt,kept,digest)
           VALUES ('run_old',0,5,'pass','',false,false,false,false,true,true,10,1,2,'wrk_old',true,1,'none','\\x01000000');
         INSERT INTO workers(id,name,caps_json,registered_at,last_seen,trusted,secret_hash)
           VALUES ('wrk_old','w','{}',1,2,false,'h');",
    )
    .unwrap();
}

fn table_dump(c: &mut Client, table: &str) -> Vec<String> {
    c.query(
        &format!("SELECT (t.*)::text FROM {table} t ORDER BY 1"),
        &[],
    )
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// Columns, indexes and constraints of a schema, with the schema's own name masked.
fn schema_dump(c: &mut Client, schema: &str) -> Vec<String> {
    let mask = |s: String| s.replace(schema, "S");
    let mut out = Vec::new();
    for r in c
        .query(
            "SELECT table_name || '.' || column_name || ' ' || data_type || ' ' || is_nullable || ' ' || COALESCE(column_default, '')
               FROM information_schema.columns WHERE table_schema = $1 ORDER BY table_name, ordinal_position",
            &[&schema],
        )
        .unwrap()
    {
        out.push(mask(r.get(0)));
    }
    for r in c
        .query(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = $1 ORDER BY indexname",
            &[&schema],
        )
        .unwrap()
    {
        out.push(mask(r.get(0)));
    }
    for r in c
        .query(
            "SELECT conrelid::regclass::text || ' ' || conname || ' ' || pg_get_constraintdef(oid) FROM pg_constraint
              WHERE connamespace = (SELECT oid FROM pg_namespace WHERE nspname = $1) AND conname NOT LIKE '%_not_null' ORDER BY 1",
            &[&schema],
        )
        .unwrap()
    {
        out.push(mask(r.get(0)));
    }
    out
}

fn table_exists(c: &mut Client, schema: &str, table: &str) -> bool {
    c.query_one(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2)",
        &[&schema, &table],
    )
    .unwrap()
    .get(0)
}

const NEW_TABLES: [&str; 4] = ["run_identity", "trees", "trial_lineage", "tree_branches"];
/// The schema version this build creates and migrates to (versions 4 and 5 only add tables to version 3).
const CURRENT_VERSION: i32 = 5;

/// The PostgreSQL tests return early without a URL, which looks like a pass. Setting `HAIRSPRING_REQUIRE_PG=1` turns
/// a missing URL into a failure, and with a URL this proves a server answers.
#[test]
fn the_postgres_tests_really_connect() {
    let Some(url) = url() else {
        assert!(
            std::env::var("HAIRSPRING_REQUIRE_PG").is_err(),
            "HAIRSPRING_REQUIRE_PG is set but HAIRSPRING_TEST_PG_URL is not: the PostgreSQL tests would pass unrun"
        );
        return;
    };
    let version: String = admin(&url)
        .query_one("SELECT version()", &[])
        .unwrap()
        .get(0);
    eprintln!("connected to: {version}");
    assert!(version.starts_with("PostgreSQL"), "{version}");
}

#[test]
fn a_fresh_schema_is_current_and_has_the_tree_tables() {
    let Some(url) = url() else { return };
    let schema = new_schema("f");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    for t in NEW_TABLES {
        assert!(table_exists(&mut c, &schema, t), "{t}");
    }
}

#[test]
fn a_version_two_schema_migrates_to_three_in_place_and_leaves_trials_and_runs_alone() {
    let Some(url) = url() else { return };
    let schema = new_schema("m");
    make_v2_schema(&url, &schema);
    let mut c = raw(&url, &schema);
    let kept = [
        "runs",
        "trials",
        "workers",
        "failures",
        "blobs",
        "run_catalog",
        "run_properties",
    ];
    let before: Vec<_> = kept.iter().map(|t| table_dump(&mut c, t)).collect();
    let columns_before: Vec<String> = schema_dump(&mut c, &schema)
        .into_iter()
        .filter(|l| l.starts_with("runs.") || l.starts_with("trials."))
        .collect();

    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    assert_eq!(s.get_run("run_old").unwrap().unwrap().trials, 1);
    assert_eq!(s.tree_of_run("run_old").unwrap(), None);

    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    for (t, was) in kept.iter().zip(before) {
        assert_eq!(table_dump(&mut c, t), was, "{t} is untouched");
    }
    let columns_after: Vec<String> = schema_dump(&mut c, &schema)
        .into_iter()
        .filter(|l| l.starts_with("runs.") || l.starts_with("trials."))
        .collect();
    assert_eq!(
        columns_after, columns_before,
        "no column was added to runs or trials"
    );
    for t in NEW_TABLES {
        assert!(table_dump(&mut c, t).is_empty(), "{t} starts empty");
    }
}

#[test]
fn a_fresh_schema_and_a_migrated_one_are_identical() {
    let Some(url) = url() else { return };
    let (fresh, migrated) = (new_schema("a"), new_schema("b"));
    drop(PgStore::connect_in_schema(&url, &fresh, 1).unwrap());
    make_v2_schema(&url, &migrated);
    drop(PgStore::connect_in_schema(&url, &migrated, 1).unwrap());
    let mut c = admin(&url);
    let (a, b) = (schema_dump(&mut c, &fresh), schema_dump(&mut c, &migrated));
    assert_eq!(a, b);
    assert!(
        a.iter()
            .any(|l| l.contains("trial_lineage_check")
                || l.contains("CHECK ((parent_node IS NULL OR")),
        "{a:?}"
    );
}

/// The URL with its user replaced by `user` (no password), for the trust-authenticated test server.
fn url_as(url: &str, user: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("a postgres URL");
    let (_, host) = rest.rsplit_once('@').expect("a URL with a user");
    format!("{scheme}://{user}@{host}")
}

#[test]
fn opening_a_current_schema_issues_no_ddl_at_all() {
    let Some(url) = url() else { return };
    let schema = new_schema("d");
    let role = format!("hs_v3_{}", &new_id("u").replace('-', "_")[..12]);
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut a = admin(&url);
    // The role may read and write rows and nothing else: no CREATE on the schema, none on the database, and it owns
    // nothing. Any CREATE or ALTER the open issued, even an idempotent one, would be refused.
    a.batch_execute(&format!(
        "CREATE ROLE \"{role}\" LOGIN;
         GRANT USAGE ON SCHEMA \"{schema}\" TO \"{role}\";
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA \"{schema}\" TO \"{role}\";
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA \"{schema}\" TO \"{role}\";"
    ))
    .unwrap();
    let opened = PgStore::connect_in_schema(&url_as(&url, &role), &schema, 1);
    let usable = opened
        .as_ref()
        .map(|s| s.list_runs(1).map(|_| ()).map_err(|e| e.to_string()))
        .map_err(|e| e.to_string());
    drop(opened);
    a.batch_execute(&format!("DROP OWNED BY \"{role}\"; DROP ROLE \"{role}\""))
        .unwrap();
    assert_eq!(
        usable,
        Ok(Ok(())),
        "a role with row rights only must be able to open a version 3 schema"
    );
}

#[test]
fn a_schema_missing_a_version_three_table_is_refused_not_repaired() {
    let Some(url) = url() else { return };
    let schema = new_schema("t");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    c.batch_execute(&format!("DROP TABLE \"{schema}\".tree_branches"))
        .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("tree_branches"), "{err}");
    assert!(!table_exists(&mut c, &schema, "tree_branches"));
}

#[test]
fn a_schema_missing_a_version_three_index_is_refused_not_repaired() {
    let Some(url) = url() else { return };
    let schema = new_schema("i");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    c.batch_execute(&format!("DROP INDEX \"{schema}\".lineage_children"))
        .unwrap();
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains("lineage_children"), "{err}");
    let present: bool = c
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = $1 AND indexname = 'lineage_children')",
            &[&schema],
        )
        .unwrap()
        .get(0);
    assert!(!present, "the index was not recreated");
}

#[test]
fn a_schema_newer_than_this_build_is_refused_untouched() {
    let Some(url) = url() else { return };
    let schema = new_schema("n");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    let newer = CURRENT_VERSION + 1;
    c.batch_execute(&format!(
        "UPDATE \"{schema}\".schema_version SET version = {newer}"
    ))
    .unwrap();
    let dump = schema_dump(&mut c, &schema);
    let err = PgStore::connect_in_schema(&url, &schema, 1)
        .err()
        .expect("must refuse");
    assert!(err.to_string().contains(&newer.to_string()), "{err}");
    assert_eq!(recorded_version(&mut c, &schema), newer);
    assert_eq!(schema_dump(&mut c, &schema), dump);
}

#[test]
fn concurrent_first_opens_of_a_version_two_schema_migrate_it_exactly_once() {
    let Some(url) = url() else { return };
    for round in 0..5 {
        let schema = new_schema(&format!("c{round}"));
        make_v2_schema(&url, &schema);
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
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
        assert!(errors.is_empty(), "round {round}: {errors:?}");
        assert_eq!(recorded_version(&mut admin(&url), &schema), CURRENT_VERSION);
    }
}

const CHILD_URL_ENV: &str = "HAIRSPRING_TEST_V3_CHILD_PG_URL";
const CHILD_SCHEMA_ENV: &str = "HAIRSPRING_TEST_V3_CHILD_PG_SCHEMA";
const CHILD_START_ENV: &str = "HAIRSPRING_TEST_V3_CHILD_PG_START_MS";

/// Not a test in its own right: the parent test re-runs this binary with the env vars set and this connects to the
/// shared schema at the agreed instant. Without them it does nothing.
#[test]
fn child_process_connects_to_the_schema_named_in_the_environment() {
    let (Ok(url), Ok(schema), Ok(start)) = (
        std::env::var(CHILD_URL_ENV),
        std::env::var(CHILD_SCHEMA_ENV),
        std::env::var(CHILD_START_ENV),
    ) else {
        return;
    };
    let start: u128 = start.parse().unwrap();
    while std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        < start
    {
        std::hint::spin_loop();
    }
    if let Err(e) = PgStore::connect_in_schema(&url, &schema, 2) {
        panic!("child connect failed: {e}");
    }
}

#[test]
fn several_processes_opening_a_version_two_schema_at_once_all_succeed() {
    let Some(url) = url() else { return };
    let schema = new_schema("p");
    make_v2_schema(&url, &schema);
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 500;
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = (0..6)
        .map(|_| {
            std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "child_process_connects_to_the_schema_named_in_the_environment",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_URL_ENV, &url)
                .env(CHILD_SCHEMA_ENV, &schema)
                .env(CHILD_START_ENV, start.to_string())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let failures: Vec<String> = children
        .into_iter()
        .map(|c| c.wait_with_output().unwrap())
        .filter(|o| !o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .collect();
    assert!(failures.is_empty(), "child processes failed: {failures:?}");
    assert_eq!(recorded_version(&mut admin(&url), &schema), CURRENT_VERSION);
}

#[test]
fn the_parent_check_refuses_a_parent_that_is_not_earlier() {
    let Some(url) = url() else { return };
    let schema = new_schema("k");
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let run = new_run(&s, "sim");
    ingest_verdicts(&s, &run.id, &passes(3));
    let tree = make_tree(&s, &run, None);
    let mut c = raw(&url, &schema);
    let insert = |c: &mut Client, node: i64, seq: i64, parent: Option<i64>| {
        c.execute(
            "INSERT INTO trial_lineage(tree_id,node,run_id,seq,parent_node,depth,path,verdict,operator,cut_kind,cut_at,
               origin,reporter_id,reporter_trusted,verified,created_at)
             VALUES ($1,$2,$3,$4,$5,0,'\\x00'::bytea,'pass','root','none',0,'server','w',true,'no',1)",
            &[&tree.id, &node, &run.id, &seq, &parent],
        )
    };
    insert(&mut c, 0, 0, None).expect("a root is accepted");
    for (node, seq, parent) in [(1, 1, 1), (1, 1, 5)] {
        let err = insert(&mut c, node, seq, Some(parent)).unwrap_err();
        let message = err
            .as_db_error()
            .map(|d| d.message().to_string())
            .unwrap_or_default();
        assert!(
            message.contains("check constraint"),
            "parent {parent} for node {node}: {err:?}"
        );
    }
    insert(&mut c, 1, 2, Some(0)).expect("a valid child is accepted");
}

fn mark_branch(url: &str, schema: &str) -> impl Fn(&str) {
    let (url, schema) = (url.to_string(), schema.to_string());
    move |run: &str| {
        raw(&url, &schema)
            .execute("UPDATE runs SET mode = 'branch' WHERE id = $1", &[&run])
            .unwrap();
    }
}

fn dump_v2_tables(c: &mut Client) -> Vec<Vec<String>> {
    [
        "runs", "trials", "failures", "events", "jobs", "workers", "blobs",
    ]
    .iter()
    .map(|t| table_dump(c, t))
    .collect()
}

#[test]
fn downgrading_keeps_branch_runs_as_detached_runs_and_the_round_trip_works() {
    let Some(url) = url() else { return };
    let schema = new_schema("dk");
    make_v2_schema(&url, &schema);
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let f = common::tree_downgrade::build(&s, &mark_branch(&url, &schema));
    drop(s);

    let dry = downgrade_postgres(
        &url,
        &schema,
        &DowngradeOptions {
            drop_branch_runs: false,
            dry_run: true,
        },
    )
    .unwrap();
    assert_eq!(
        (dry.from_version, dry.applied),
        (i64::from(CURRENT_VERSION), false)
    );
    assert_eq!(dry.branch_runs, vec![f.branch_run.clone()]);
    let mut c = raw(&url, &schema);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);

    let (runs_before, trials_before) = (table_dump(&mut c, "runs"), table_dump(&mut c, "trials"));
    let report = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert!(report.dropped_blobs >= 1, "{report:?}");
    assert_eq!(
        dry.dropped_blobs, report.dropped_blobs,
        "a dry run reports the real blob count"
    );
    assert_eq!(
        (report.from_version, report.to_version, report.applied),
        (i64::from(CURRENT_VERSION), 2, true)
    );
    assert_eq!(report.branch_runs, vec![f.branch_run.clone()]);
    assert!(report.dropped_runs.is_empty());

    assert_eq!(recorded_version(&mut c, &schema), 2);
    for t in NEW_TABLES {
        assert!(!table_exists(&mut c, &schema, t), "{t} is dropped");
    }
    assert_eq!(
        (table_dump(&mut c, "runs"), table_dump(&mut c, "trials")),
        (runs_before, trials_before)
    );
    let mode: String = c
        .query_one("SELECT mode FROM runs WHERE id = $1", &[&f.branch_run])
        .unwrap()
        .get(0);
    assert_eq!(mode, "branch");
    let trials: i64 = c
        .query_one(
            "SELECT COUNT(*) FROM trials WHERE run_id = $1",
            &[&f.branch_run],
        )
        .unwrap()
        .get(0);
    assert_eq!(trials, 2);
    let blob_count = |c: &mut Client, hash: &str| -> i64 {
        c.query_one("SELECT COUNT(*) FROM blobs WHERE hash = $1", &[&hash])
            .unwrap()
            .get(0)
    };
    assert_eq!(
        blob_count(&mut c, &f.edge_blob),
        0,
        "the blob only the lineage used is collected"
    );
    assert_eq!(blob_count(&mut c, &f.shared_blob), 1);
    assert_eq!(blob_count(&mut c, &f.branch_blob), 1);
    let v2_state = dump_v2_tables(&mut c);

    let again = downgrade_postgres(&url, &schema, &DowngradeOptions::default()).unwrap();
    assert!(!again.applied && again.from_version == 2);

    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    assert_eq!(s.get_run(&f.branch_run).unwrap().unwrap().mode, "branch");
    assert_eq!(s.count_trials(&f.branch_run).unwrap(), 2);
    assert_eq!(s.get_tree(&f.tree).unwrap(), None);
    assert_eq!(recorded_version(&mut c, &schema), CURRENT_VERSION);
    assert_eq!(dump_v2_tables(&mut c), v2_state);
}

#[test]
fn downgrading_with_drop_branch_runs_deletes_them_and_their_orphan_blobs() {
    let Some(url) = url() else { return };
    let schema = new_schema("dd");
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let f = common::tree_downgrade::build(&s, &mark_branch(&url, &schema));
    drop(s);
    let report = downgrade_postgres(
        &url,
        &schema,
        &DowngradeOptions {
            drop_branch_runs: true,
            dry_run: false,
        },
    )
    .unwrap();
    assert_eq!(report.branch_runs, vec![f.branch_run.clone()]);
    assert_eq!(report.dropped_runs, vec![f.branch_run.clone()]);
    assert!(report.dropped_blobs >= 2, "{report:?}");

    let mut c = raw(&url, &schema);
    assert_eq!(recorded_version(&mut c, &schema), 2);
    let count =
        |c: &mut Client, sql: &str, id: &str| -> i64 { c.query_one(sql, &[&id]).unwrap().get(0) };
    for table in [
        "runs WHERE id",
        "trials WHERE run_id",
        "failures WHERE run_id",
        "events WHERE run_id",
        "jobs WHERE run_id",
    ] {
        assert_eq!(
            count(
                &mut c,
                &format!("SELECT COUNT(*) FROM {table} = $1"),
                &f.branch_run
            ),
            0,
            "{table}"
        );
    }
    for run in [&f.root_run, &f.plain_run] {
        assert_eq!(
            count(&mut c, "SELECT COUNT(*) FROM runs WHERE id = $1", run),
            1,
            "{run} stays"
        );
    }
    assert_eq!(
        count(
            &mut c,
            "SELECT COUNT(*) FROM blobs WHERE hash = $1",
            &f.branch_blob
        ),
        0
    );
    assert_eq!(
        count(
            &mut c,
            "SELECT COUNT(*) FROM blobs WHERE hash = $1",
            &f.edge_blob
        ),
        0
    );
    assert_eq!(
        count(
            &mut c,
            "SELECT COUNT(*) FROM blobs WHERE hash = $1",
            &f.shared_blob
        ),
        1
    );
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    assert!(s.get_run(&f.branch_run).unwrap().is_none());
    assert_eq!(s.count_trials(&f.plain_run).unwrap(), 2);
}

#[test]
fn the_downgrade_refuses_versions_it_does_not_know() {
    let Some(url) = url() else { return };
    let schema = new_schema("dr");
    drop(PgStore::connect_in_schema(&url, &schema, 1).unwrap());
    let mut c = admin(&url);
    for v in [CURRENT_VERSION + 1, 1] {
        c.batch_execute(&format!(
            "UPDATE \"{schema}\".schema_version SET version = {v}"
        ))
        .unwrap();
        let dump = schema_dump(&mut c, &schema);
        let err =
            downgrade_postgres(&url, &schema, &DowngradeOptions::default()).expect_err("refused");
        assert!(err.to_string().contains(&v.to_string()), "{err}");
        assert_eq!(schema_dump(&mut c, &schema), dump);
    }
    assert!(
        downgrade_postgres(&url, "x; DROP SCHEMA public", &DowngradeOptions::default()).is_err()
    );
}

#[test]
#[ignore = "hairspring gc does not exist yet (RX D6); this is its specification"]
fn gc_spec_a_tree_is_kept_while_any_of_its_runs_is_kept() {
    let url = url().expect("HAIRSPRING_TEST_PG_URL");
    let s = PgStore::connect_in_schema(&url, &new_schema("g1"), 2).unwrap();
    common::tree_gc_spec::a_tree_is_kept_while_any_of_its_runs_is_kept(
        &s,
        &common::tree_gc_spec::gc_is_not_built,
    );
}

#[test]
#[ignore = "hairspring gc does not exist yet (RX D6); this is its specification"]
fn gc_spec_a_tree_is_deleted_in_one_unit_when_every_run_has_expired() {
    let url = url().expect("HAIRSPRING_TEST_PG_URL");
    let s = PgStore::connect_in_schema(&url, &new_schema("g2"), 2).unwrap();
    common::tree_gc_spec::a_tree_is_deleted_in_one_unit_when_every_run_has_expired(
        &s,
        &common::tree_gc_spec::gc_is_not_built,
    );
}

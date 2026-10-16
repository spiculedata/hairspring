//! Schema version 3 on SQLite: migration, open path, constraints and downgrade. Queries and plans are in
//! `sqlite_trees.rs`.
mod common;
use common::tree_support::*;
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-v3-{name}-{}-{}",
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

/// A genuine version 2 database with data in the tables version 3 must leave alone.
fn make_v2_database(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(&common::v2_schema::sqlite_v2()).unwrap();
    conn.execute_batch(
        "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at,steps,trials,mode)
           VALUES ('run_old','legacy','hermit','passed',9,'{}',1000,50,1,'search');
         INSERT INTO trials(run_id,seq,seed,verdict,reason,timed_out,degraded,perturbing,report_missing,rated,
             claimable,syscalls,work_ops,elapsed_ms,worker_id,worker_trusted,attempt,kept,digest)
           VALUES ('run_old',0,5,'pass','',0,0,0,0,1,1,10,1,2,'wrk_old',1,1,'none',x'01000000');
         INSERT INTO workers(id,name,caps_json,registered_at,last_seen,trusted,secret_hash)
           VALUES ('wrk_old','w','{}',1,2,0,'h');
         PRAGMA user_version = 2;",
    )
    .unwrap();
}

fn schema_dump(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT type, name, tbl_name, COALESCE(sql,'') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| {
        Ok(format!(
            "{}|{}|{}|{}",
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?
        ))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

fn table_dump(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1, 2"))
        .unwrap();
    let cols = stmt.column_count();
    stmt.query_map([], |r| {
        Ok((0..cols)
            .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
            .collect::<Vec<_>>()
            .join("|"))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
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

const NEW_TABLES: [&str; 4] = ["run_identity", "trees", "trial_lineage", "tree_branches"];

#[test]
fn a_fresh_database_is_at_version_three_with_the_tree_tables() {
    let path = temp_db("fresh");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 3);
    for t in NEW_TABLES {
        assert!(table_exists(&conn, t), "{t}");
    }
    cleanup(&path);
}

#[test]
fn a_version_two_database_migrates_to_three_in_place_and_leaves_trials_and_runs_alone() {
    let path = temp_db("v2");
    make_v2_database(&path);
    let before = Connection::open(&path).unwrap();
    let kept = [
        "runs",
        "trials",
        "workers",
        "failures",
        "blobs",
        "run_catalog",
        "run_properties",
    ];
    let dumps: Vec<_> = kept.iter().map(|t| table_dump(&before, t)).collect();
    let runs_ddl = schema_dump(&before)
        .into_iter()
        .filter(|l| l.contains("|runs|") || l.contains("|trials|"))
        .collect::<Vec<_>>();
    drop(before);

    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run("run_old").unwrap().unwrap().trials, 1);
    assert_eq!(s.tree_of_run("run_old").unwrap(), None);
    drop(s);

    let after = Connection::open(&path).unwrap();
    assert_eq!(user_version(&after), 3);
    for (t, was) in kept.iter().zip(dumps) {
        assert_eq!(table_dump(&after, t), was, "{t} is untouched");
    }
    let runs_ddl_after = schema_dump(&after)
        .into_iter()
        .filter(|l| l.contains("|runs|") || l.contains("|trials|"))
        .collect::<Vec<_>>();
    assert_eq!(
        runs_ddl_after, runs_ddl,
        "no column was added to runs or trials"
    );
    for t in NEW_TABLES {
        assert!(table_exists(&after, t), "{t}");
        assert!(table_dump(&after, t).is_empty(), "{t} starts empty");
    }
    cleanup(&path);
}

#[test]
fn a_fresh_schema_and_a_migrated_one_are_identical() {
    let fresh = temp_db("same-fresh");
    drop(SqliteStore::open(&fresh).unwrap());
    let migrated = temp_db("same-migrated");
    make_v2_database(&migrated);
    drop(SqliteStore::open(&migrated).unwrap());
    let a = schema_dump(&Connection::open(&fresh).unwrap());
    let b = schema_dump(&Connection::open(&migrated).unwrap());
    assert_eq!(a, b);
    cleanup(&fresh);
    cleanup(&migrated);
}

#[test]
fn a_version_two_database_without_worker_secrets_gains_the_column_on_the_way_to_three() {
    let path = temp_db("v2-no-secret");
    make_v2_database(&path);
    Connection::open(&path)
        .unwrap()
        .execute_batch("ALTER TABLE workers DROP COLUMN secret_hash")
        .unwrap();
    let s = SqliteStore::open(&path).unwrap();
    let w = s.register_worker_with_secret("r", "{}", "abc", 1).unwrap();
    assert_eq!(s.worker_secret_hash(&w.id).unwrap().as_deref(), Some("abc"));
    cleanup(&path);
}

fn schema_cookie(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn opening_a_current_database_runs_no_ddl() {
    let path = temp_db("noddl");
    drop(SqliteStore::open(&path).unwrap());
    let probe = Connection::open(&path).unwrap();
    let cookie = schema_cookie(&probe);
    let dump = schema_dump(&probe);
    for _ in 0..3 {
        drop(SqliteStore::open(&path).unwrap());
    }
    // The schema cookie moves with every CREATE, ALTER and DROP: it staying put shows none was issued.
    assert_eq!(schema_cookie(&probe), cookie);
    assert_eq!(schema_dump(&probe), dump);
    cleanup(&path);
}

#[test]
fn a_version_three_database_missing_a_table_is_refused_not_repaired() {
    let path = temp_db("missing");
    drop(SqliteStore::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE tree_branches")
        .unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("tree_branches"), "{err}");
    assert!(!table_exists(
        &Connection::open(&path).unwrap(),
        "tree_branches"
    ));
    cleanup(&path);
}

#[test]
fn a_version_three_database_missing_an_index_is_refused_not_repaired() {
    let path = temp_db("missing-index");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP INDEX lineage_children").unwrap();
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("lineage_children"), "{err}");
    let present: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='lineage_children'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(present, 0, "the index was not recreated");
    cleanup(&path);
}

#[test]
fn a_database_newer_than_three_is_refused_untouched() {
    let path = temp_db("v4");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 4).unwrap();
    let dump = schema_dump(&conn);
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains('4'), "{err}");
    assert_eq!(user_version(&conn), 4);
    assert_eq!(schema_dump(&conn), dump);
    cleanup(&path);
}

#[test]
fn the_path_column_refuses_text_and_a_later_parent() {
    let path = temp_db("checks");
    let s = SqliteStore::open(&path).unwrap();
    let run = new_run(&s, "sim");
    ingest_verdicts(&s, &run.id, &passes(3));
    let tree = make_tree(&s, &run, None);
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    let insert = |node: i64, seq: i64, parent: Option<i64>, path: rusqlite::types::Value| {
        conn.execute(
            "INSERT INTO trial_lineage(tree_id,node,run_id,seq,parent_node,depth,path,verdict,operator,cut_kind,cut_at,
               origin,reporter_id,reporter_trusted,verified,created_at)
             VALUES (?,?,?,?,?,0,?,'pass','root','none',0,'server','w',1,'no',1)",
            rusqlite::params![tree.id, node, run.id, seq, parent, path],
        )
    };
    let text = rusqlite::types::Value::Text("0".into());
    let blob = rusqlite::types::Value::Blob(vec![0]);
    let err = insert(0, 0, None, text).unwrap_err().to_string();
    assert!(err.contains("CHECK"), "TEXT path refused: {err}");
    insert(0, 0, None, blob.clone()).expect("a BLOB path is accepted");
    let err = insert(1, 1, Some(1), blob.clone()).unwrap_err().to_string();
    assert!(err.contains("CHECK"), "parent equal to node refused: {err}");
    let err = insert(1, 1, Some(5), blob.clone()).unwrap_err().to_string();
    assert!(err.contains("CHECK"), "parent after node refused: {err}");
    let err = insert(2, 2, Some(9), blob.clone()).unwrap_err().to_string();
    assert!(err.contains("CHECK") || err.contains("FOREIGN"), "{err}");
    insert(1, 2, Some(0), blob).expect("a valid child is accepted");
    cleanup(&path);
}

fn dump_v2_tables(conn: &Connection) -> Vec<Vec<String>> {
    [
        "runs", "trials", "failures", "events", "jobs", "workers", "blobs",
    ]
    .iter()
    .map(|t| table_dump(conn, t))
    .collect()
}

fn mark_branch(path: &Path) -> impl Fn(&str) + '_ {
    move |run: &str| {
        Connection::open(path)
            .unwrap()
            .execute("UPDATE runs SET mode='branch' WHERE id=?", [run])
            .unwrap();
    }
}

#[test]
fn downgrading_keeps_branch_runs_as_detached_runs_and_the_round_trip_works() {
    let path = temp_db("down-keep");
    make_v2_database(&path);
    let s = SqliteStore::open(&path).unwrap();
    let f = common::tree_downgrade::build(&s, &mark_branch(&path));
    drop(s);

    // A dry run reports and changes nothing.
    let dry = downgrade_sqlite(
        &path,
        &DowngradeOptions {
            drop_branch_runs: false,
            dry_run: true,
        },
    )
    .unwrap();
    assert_eq!((dry.from_version, dry.applied), (3, false));
    assert_eq!(dry.branch_runs, vec![f.branch_run.clone()]);
    assert_eq!(user_version(&Connection::open(&path).unwrap()), 3);

    let v3_runs = {
        let c = Connection::open(&path).unwrap();
        (table_dump(&c, "runs"), table_dump(&c, "trials"))
    };
    let report = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert!(report.dropped_blobs >= 1, "{report:?}");
    assert_eq!(
        dry.dropped_blobs, report.dropped_blobs,
        "a dry run reports the real blob count"
    );
    assert_eq!(
        (report.from_version, report.to_version, report.applied),
        (3, 2, true)
    );
    assert_eq!(report.branch_runs, vec![f.branch_run.clone()]);
    assert!(report.dropped_runs.is_empty());

    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 2);
    for t in NEW_TABLES {
        assert!(!table_exists(&conn, t), "{t} is dropped");
    }
    assert_eq!(
        (table_dump(&conn, "runs"), table_dump(&conn, "trials")),
        v3_runs,
        "runs and trials are untouched"
    );
    // What version 2 code reads: the branch run is an ordinary run whose mode is a plain string.
    let mode: String = conn
        .query_row("SELECT mode FROM runs WHERE id=?", [&f.branch_run], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(mode, "branch");
    let trials: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM trials WHERE run_id=?",
            [&f.branch_run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(trials, 2);
    let edge_left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM blobs WHERE hash=?",
            [&f.edge_blob],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edge_left, 0, "the blob only the lineage used is collected");
    let kept: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM blobs WHERE hash IN (?,?)",
            [&f.shared_blob, &f.branch_blob],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kept, 2, "blobs the remaining rows use stay");
    let before_v2_again = dump_v2_tables(&conn);
    drop(conn);

    // Downgrading a version 2 database is a no-op.
    let again = downgrade_sqlite(&path, &DowngradeOptions::default()).unwrap();
    assert!(!again.applied && again.from_version == 2);

    // v2 -> v3 again: the branch run reads back, the tree tables are empty, nothing else moved.
    let s = SqliteStore::open(&path).unwrap();
    let b = s.get_run(&f.branch_run).unwrap().unwrap();
    assert_eq!(b.mode, "branch");
    assert_eq!(s.count_trials(&f.branch_run).unwrap(), 2);
    assert_eq!(s.tree_of_run(&f.root_run).unwrap(), None);
    assert_eq!(s.get_tree(&f.tree).unwrap(), None);
    drop(s);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 3);
    assert_eq!(dump_v2_tables(&conn), before_v2_again);
    cleanup(&path);
}

#[test]
fn downgrading_with_drop_branch_runs_deletes_them_and_their_orphan_blobs() {
    let path = temp_db("down-drop");
    let s = SqliteStore::open(&path).unwrap();
    let f = common::tree_downgrade::build(&s, &mark_branch(&path));
    drop(s);
    let opts = DowngradeOptions {
        drop_branch_runs: true,
        dry_run: false,
    };
    let report = downgrade_sqlite(&path, &opts).unwrap();
    assert_eq!(report.branch_runs, vec![f.branch_run.clone()]);
    assert_eq!(report.dropped_runs, vec![f.branch_run.clone()]);

    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 2);
    let count = |sql: &str, id: &str| -> i64 { conn.query_row(sql, [id], |r| r.get(0)).unwrap() };
    for table in [
        "runs WHERE id",
        "trials WHERE run_id",
        "failures WHERE run_id",
        "events WHERE run_id",
        "jobs WHERE run_id",
    ] {
        assert_eq!(
            count(&format!("SELECT COUNT(*) FROM {table}=?"), &f.branch_run),
            0,
            "{table}"
        );
    }
    for run in [&f.root_run, &f.plain_run] {
        assert_eq!(
            count("SELECT COUNT(*) FROM runs WHERE id=?", run),
            1,
            "{run} stays"
        );
    }
    assert_eq!(
        count("SELECT COUNT(*) FROM blobs WHERE hash=?", &f.branch_blob),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM blobs WHERE hash=?", &f.edge_blob),
        0
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM blobs WHERE hash=?", &f.shared_blob),
        1,
        "still used by the plain run"
    );
    assert!(report.dropped_blobs >= 2, "{report:?}");
    // The result opens as a healthy version 3 database again.
    let s = SqliteStore::open(&path).unwrap();
    assert!(s.get_run(&f.branch_run).unwrap().is_none());
    assert_eq!(s.count_trials(&f.plain_run).unwrap(), 2);
    cleanup(&path);
}

#[test]
fn the_downgrade_refuses_versions_it_does_not_know() {
    let path = temp_db("down-refuse");
    drop(SqliteStore::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    for v in [4, 1] {
        conn.pragma_update(None, "user_version", v).unwrap();
        let dump = schema_dump(&conn);
        let err = downgrade_sqlite(&path, &DowngradeOptions::default()).expect_err("refused");
        assert!(err.to_string().contains(&v.to_string()), "{err}");
        assert_eq!(schema_dump(&conn), dump);
    }
    cleanup(&path);
}

fn make_v2_for_open_stress(path: &Path) {
    make_v2_database(path);
}

fn open_together(path: &Path, threads: usize) -> Vec<String> {
    let barrier = Arc::new(std::sync::Barrier::new(threads));
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let (p, b) = (path.to_path_buf(), barrier.clone());
            std::thread::spawn(move || {
                b.wait();
                SqliteStore::open(&p).err().map(|e| e.to_string())
            })
        })
        .collect();
    handles
        .into_iter()
        .filter_map(|h| h.join().unwrap())
        .collect()
}

#[test]
fn many_threads_opening_version_two_databases_repeatedly_all_succeed() {
    for round in 0..25 {
        let old = temp_db(&format!("stress-v2-{round}"));
        make_v2_for_open_stress(&old);
        let errors = open_together(&old, 12);
        assert!(errors.is_empty(), "round {round}: {errors:?}");
        let s = SqliteStore::open(&old).unwrap();
        assert_eq!(s.get_run("run_old").unwrap().unwrap().trials, 1);
        assert_eq!(user_version(&Connection::open(&old).unwrap()), 3);
        cleanup(&old);
    }
}

const CHILD_PATH_ENV: &str = "HAIRSPRING_TEST_V3_CHILD_PATH";
const CHILD_START_ENV: &str = "HAIRSPRING_TEST_V3_CHILD_START_MS";

/// Not a test in its own right: the parent test re-runs this binary with the env vars set and this opens the shared
/// database at the agreed instant. Without them it does nothing.
#[test]
fn child_process_opens_the_database_named_in_the_environment() {
    let (Ok(path), Ok(start)) = (
        std::env::var(CHILD_PATH_ENV),
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
    if let Err(e) = SqliteStore::open(Path::new(&path)) {
        panic!("child open failed: {e}");
    }
}

#[test]
fn several_processes_opening_a_version_two_database_at_once_all_succeed() {
    const CHILDREN: usize = 6;
    let path = temp_db("multiprocess-v2");
    make_v2_database(&path);
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 400;
    let exe = std::env::current_exe().unwrap();
    let children: Vec<_> = (0..CHILDREN)
        .map(|_| {
            std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "child_process_opens_the_database_named_in_the_environment",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD_PATH_ENV, &path)
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
    assert_eq!(user_version(&Connection::open(&path).unwrap()), 3);
    assert_eq!(
        SqliteStore::open(&path)
            .unwrap()
            .get_run("run_old")
            .unwrap()
            .unwrap()
            .trials,
        1
    );
    cleanup(&path);
}

#[test]
#[ignore = "hairspring gc does not exist yet (RX D6); this is its specification"]
fn gc_spec_a_tree_is_kept_while_any_of_its_runs_is_kept() {
    let s = SqliteStore::open_in_memory().unwrap();
    common::tree_gc_spec::a_tree_is_kept_while_any_of_its_runs_is_kept(
        &s,
        &common::tree_gc_spec::gc_is_not_built,
    );
}

#[test]
#[ignore = "hairspring gc does not exist yet (RX D6); this is its specification"]
fn gc_spec_a_tree_is_deleted_in_one_unit_when_every_run_has_expired() {
    let s = SqliteStore::open_in_memory().unwrap();
    common::tree_gc_spec::a_tree_is_deleted_in_one_unit_when_every_run_has_expired(
        &s,
        &common::tree_gc_spec::gc_is_not_built,
    );
}

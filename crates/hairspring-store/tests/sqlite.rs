mod common;
use hairspring_store::{SqliteStore, Store};
use std::sync::Arc;

#[test]
fn sqlite_store_passes_the_conformance_suite() {
    common::run_all(&|| Arc::new(SqliteStore::open_in_memory().unwrap()) as Arc<dyn Store>);
}

#[test]
fn sqlite_store_persists_across_reopen_and_migrates_idempotently() {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-store-test-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.db");
    let id = {
        let s = SqliteStore::open(&path).unwrap();
        s.create_run(
            &hairspring_store::NewRun {
                target: "t".into(),
                backend: "sim".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            5,
        )
        .unwrap()
        .id
    };
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run(&id).unwrap().unwrap().target, "t");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn random_seeds_are_exactly_representable_in_javascript() {
    // Browsers and LLM tool clients parse JSON numbers as doubles: integers above 2^53 lose precision.
    const JS_SAFE: u64 = 1 << 53;
    let seeds: Vec<u64> = (0..2000).map(|_| hairspring_store::random_seed()).collect();
    assert!(
        seeds.iter().all(|s| *s < JS_SAFE),
        "seed above 2^53 would be corrupted by JSON consumers"
    );
    let distinct: std::collections::HashSet<_> = seeds.iter().collect();
    assert!(distinct.len() > 1990, "still varied: {}", distinct.len());
}

/// A genuine version 1 database: the schema the first release created, with data in every table that matters.
fn make_v1_database(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(common::v1_schema::SQLITE_V1).unwrap();
    conn.execute_batch(
        "INSERT INTO runs(id,target,backend,state,seed,config_json,created_at,steps,trials)
           VALUES ('run_old','legacy','hermit','passed',9,'{}',1000,50,12);
         INSERT INTO failures(id,run_id,target,message,original_len,shrunk_len,bundle_hash,steps_json,created_at)
           VALUES ('fail_old','run_old','legacy','m',4,2,'h','[]',1001);
         INSERT INTO workers(id,name,caps_json,registered_at,last_seen) VALUES ('wrk_old','w','{}',1,2);
         PRAGMA user_version = 1;",
    )
    .unwrap();
}

fn temp_db(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-migrate-{name}-{}-{}",
        std::process::id(),
        hairspring_store::now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("old.db")
}

#[test]
fn a_version_one_database_migrates_in_place_and_old_rows_read_back_with_zero_counts() {
    let path = temp_db("v1");
    make_v1_database(&path);
    let s = SqliteStore::open(&path).unwrap();

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
    assert!(run.property_summary_json.is_none());
    assert_eq!(s.get_failure("fail_old").unwrap().unwrap().shrunk_len, 2);
    let workers = s.list_workers().unwrap();
    assert_eq!(
        (workers[0].id.as_str(), workers[0].trusted),
        ("wrk_old", false)
    );
    assert_eq!(s.count_trials("run_old").unwrap(), 0);
    assert!(s.get_catalog("run_old").unwrap().is_empty());
    assert!(s.list_run_properties("run_old").unwrap().is_empty());

    // The migrated database takes new data like a fresh one.
    let new = s
        .create_run(
            &hairspring_store::NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 1,
                config_json: "{}".into(),
            },
            5,
        )
        .unwrap();
    assert_eq!(new.mode, "search");
    drop(s);

    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

#[test]
fn migrating_twice_changes_nothing() {
    let path = temp_db("twice");
    make_v1_database(&path);
    drop(SqliteStore::open(&path).unwrap());
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run("run_old").unwrap().unwrap().trials, 12);
}

#[test]
fn several_threads_with_their_own_connections_opening_a_version_one_database_at_once_all_succeed() {
    let path = temp_db("race");
    make_v1_database(&path);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let p = path.clone();
            std::thread::spawn(move || SqliteStore::open(&p).err().map(|e| e.to_string()))
        })
        .collect();
    let errors: Vec<String> = handles
        .into_iter()
        .filter_map(|h| h.join().unwrap())
        .collect();
    assert!(errors.is_empty(), "concurrent opens failed: {errors:?}");
}

/// Opens `path` from `threads` threads released together; returns every error message.
fn open_together(path: &std::path::Path, threads: usize) -> Vec<String> {
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
fn many_threads_opening_old_and_new_databases_repeatedly_all_succeed() {
    for round in 0..25 {
        let old = temp_db(&format!("stress-old-{round}"));
        make_v1_database(&old);
        let fresh = temp_db(&format!("stress-new-{round}"));
        for path in [&old, &fresh] {
            let errors = open_together(path, 12);
            assert!(errors.is_empty(), "round {round}: {errors:?}");
            let store = SqliteStore::open(path).unwrap();
            assert!(store.list_workers().is_ok());
        }
    }
}

const CHILD_PATH_ENV: &str = "HAIRSPRING_TEST_OPEN_CHILD_PATH";
const CHILD_START_ENV: &str = "HAIRSPRING_TEST_OPEN_CHILD_START_MS";

/// Not a test in its own right: when the parent test re-runs this binary with the env vars set, this is the child
/// process that opens the shared database at the agreed instant. Without them it does nothing.
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
    if let Err(e) = SqliteStore::open(std::path::Path::new(&path)) {
        panic!("child open failed: {e}");
    }
}

#[test]
fn several_processes_opening_a_version_one_database_at_once_all_succeed() {
    const CHILDREN: usize = 6;
    let path = temp_db("multiprocess");
    make_v1_database(&path);
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
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.get_run("run_old").unwrap().unwrap().trials, 12);
}

#[test]
fn an_opened_file_database_is_in_wal_mode() {
    let path = temp_db("walmode");
    drop(SqliteStore::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn a_database_from_the_future_is_refused_untouched() {
    let path = temp_db("future");
    make_v1_database(&path);
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
    }
    let err = SqliteStore::open(&path).err().expect("must refuse");
    assert!(err.to_string().contains("99"), "{err}");
    let conn = rusqlite::Connection::open(&path).unwrap();
    let has_trials: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='trials'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(has_trials, 0, "a refused database is not modified");
}

#[test]
fn a_version_two_database_from_before_worker_secrets_gains_the_column_when_opened() {
    let path = temp_db("v2-no-secret");
    drop(SqliteStore::open(&path).unwrap());
    {
        // The earlier version 2 had no secret column. (Version 3 adds nothing to `workers`, so a database is made to
        // look like that earlier build by dropping the column and marking it version 2.)
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("ALTER TABLE workers DROP COLUMN secret_hash")
            .unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
    }
    let s = SqliteStore::open(&path).unwrap();
    let w = s
        .register_worker_with_secret("remote", "{}", "abc", 1)
        .unwrap();
    assert_eq!(s.worker_secret_hash(&w.id).unwrap().as_deref(), Some("abc"));
    // Opening again, with the column present, changes nothing.
    drop(s);
    let again = SqliteStore::open(&path).unwrap();
    assert_eq!(again.list_workers().unwrap().len(), 1);
}

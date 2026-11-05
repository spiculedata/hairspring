//! Retention on SQLite racing an ingest from another connection, as another process would. SQLite has one writer at a
//! time and retention takes the write lock when it begins, so it must wait for the ingest to commit and then see the
//! trial that refers to the shared blob.
mod common;
use common::tree_support::*;
use hairspring_store::*;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Long enough for retention to have reached the write lock if it were not going to wait for it; well inside
/// [`BUSY_TIMEOUT`], so the wait ends in success.
const INGEST_HOLDS: Duration = Duration::from_millis(500);

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-retention-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.sqlite")
}

#[test]
fn a_blob_a_trial_is_being_stored_against_is_not_removed() {
    let path = temp_db("blob");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let old = new_run(&*s, "sim");
    let mut b = batch(&passes(1), 0);
    b.trials[0].output = Some(b"output both runs produce".to_vec());
    let job = ingest_batch(&*s, &old.id, &b);
    assert!(s.complete_job(&job.id, "w").unwrap());
    finish(&*s, &old.id);
    let hash = sha256_hex(b"output both runs produce");
    let live = new_run(&*s, "sim");

    // What the ingest of `live` does, stopped before it commits: the blob insert, then a trial naming the blob.
    let ingest = Connection::open(&path).unwrap();
    ingest
        .execute_batch(&format!(
            "BEGIN IMMEDIATE; \
             INSERT OR IGNORE INTO blobs(hash,data) VALUES ('{hash}', CAST('output both runs produce' AS BLOB)); \
             CREATE TEMP TABLE copy AS SELECT * FROM trials WHERE run_id='{}'; \
             UPDATE copy SET run_id='{}'; INSERT INTO trials SELECT * FROM copy;",
            old.id, live.id
        ))
        .unwrap();

    let gc = {
        let s = Arc::clone(&s);
        let runs = vec![old.id.clone()];
        std::thread::spawn(move || s.retention_delete(&runs, i64::MAX, false).unwrap())
    };
    std::thread::sleep(INGEST_HOLDS);
    assert!(
        !gc.is_finished(),
        "retention cannot finish while another writer holds the database"
    );
    ingest.execute_batch("COMMIT").unwrap();
    let report = gc.join().unwrap();

    assert_eq!(report.error, None);
    assert_eq!(report.run_ids, vec![old.id.clone()]);
    let trial = s.list_trials(&live.id, None, 10).unwrap().remove(0);
    assert_eq!(trial.output_ref.as_deref(), Some(hash.as_str()));
    assert!(s.get_blob(&hash).unwrap().is_some());
    assert_eq!(report.blobs, 0, "{report:?}");
    drop(s);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// A dry run takes no write lock: it finishes, with the right counts, while another connection holds the database's
/// write lock (it would otherwise wait out the busy timeout and fail), and that writer can still commit.
#[test]
fn a_dry_run_reads_while_another_connection_holds_the_write_lock() {
    let path = temp_db("dry");
    let s = SqliteStore::open(&path).unwrap();
    let old = new_run(&s, "sim");
    let mut b = batch(&passes(2), 0);
    b.trials[0].output = Some(b"old output".to_vec());
    let job = ingest_batch(&s, &old.id, &b);
    assert!(s.complete_job(&job.id, "w").unwrap());
    finish(&s, &old.id);

    let writer = Connection::open(&path).unwrap();
    writer
        .execute_batch(
            "BEGIN IMMEDIATE; INSERT INTO blobs(hash,data) VALUES ('written meanwhile', CAST('x' AS BLOB));",
        )
        .unwrap();
    let started = std::time::Instant::now();
    let report = s
        .retention_delete(std::slice::from_ref(&old.id), i64::MAX, true)
        .unwrap();
    assert!(
        started.elapsed() < BUSY_TIMEOUT,
        "the dry run did not wait for the writer"
    );
    writer.execute_batch("COMMIT").unwrap();

    assert_eq!(report.run_ids, vec![old.id.clone()]);
    assert_eq!((report.trials, report.blobs), (2, 1));
    assert!(s.get_run(&old.id).unwrap().is_some(), "nothing was removed");
    assert!(s.get_blob("written meanwhile").unwrap().is_some());
    drop(s);
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

/// Two gc commands on one database take turns: the second cannot take the retention lock until the first lets go,
/// whichever connection or process asks.
#[test]
fn only_one_holder_of_the_retention_lock_at_a_time() {
    let path = temp_db("lock");
    let first = SqliteStore::open(&path).unwrap();
    let second = SqliteStore::open(&path).unwrap();
    let held = first.retention_lock().unwrap().expect("free at first");
    assert!(second.retention_lock().unwrap().is_none());
    assert!(first.retention_lock().unwrap().is_none(), "not re-entrant");
    let other = temp_db("lock-other");
    let elsewhere = SqliteStore::open(&other).unwrap();
    assert!(
        elsewhere.retention_lock().unwrap().is_some(),
        "another database is not affected"
    );
    drop(held);
    assert!(second.retention_lock().unwrap().is_some());
    let memory = SqliteStore::open_in_memory().unwrap();
    let a = memory.retention_lock().unwrap();
    assert!(a.is_some(), "an in-memory database has nothing to share");
    drop((first, second, elsewhere));
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
    std::fs::remove_dir_all(other.parent().unwrap()).ok();
}

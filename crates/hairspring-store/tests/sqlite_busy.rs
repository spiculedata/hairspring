//! Two connections to one SQLite file, as two processes would be: a write that has to wait for another writer must
//! wait and succeed, and one that waits too long must end in a clear error rather than a hang.
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long the other writer holds the file in the test that expects the wait to succeed.
const OTHER_WRITER_HOLDS: Duration = Duration::from_millis(400);
/// The longest the other writer ever holds the file, so a store call that never returns fails the test instead of
/// hanging it.
const OTHER_WRITER_CAP: Duration = Duration::from_secs(40);
/// The longest a store call may wait before the test calls it a hang. SQLite counts the sleeps it asks for, not the
/// time that passes, so a loaded machine waits longer than [`BUSY_TIMEOUT`]; this leaves room for that.
const HANG_LIMIT: Duration = Duration::from_secs(30);

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-busy-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.sqlite")
}

fn cleanup(path: &Path) {
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

fn failure(s: &SqliteStore) -> FailureRecord {
    let run = s
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "hermit".into(),
                seed: 7,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let bundle = s.put_blob(b"bundle").unwrap();
    s.add_failure(
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
    .unwrap()
}

fn confirmation(failure_id: &str) -> NewConfirmation {
    NewConfirmation {
        id: "conf_busy".into(),
        failure_id: failure_id.into(),
        runner: "local-1".into(),
        runner_trusted: true,
        checks: vec![NewCheck {
            kind: "same_seed_rerun".into(),
            status: "pass".into(),
            passed: 1,
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

/// Another process taking the write lock: writes a row, signals that it holds the lock, then commits when told to
/// release it (the sender is dropped or sent to) or after `hold`, whichever comes first.
fn other_writer(
    path: &Path,
    hold: Duration,
) -> (thread::JoinHandle<()>, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (tx, rx) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let path = path.to_path_buf();
    let handle = thread::spawn(move || {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS other_writer(x);")
            .unwrap();
        tx.send(()).unwrap();
        released.recv_timeout(hold).ok();
        conn.execute_batch("COMMIT").unwrap();
    });
    (handle, rx, release)
}

#[test]
fn a_write_that_waits_for_another_writer_succeeds_once_it_commits() {
    let path = temp_db("wait");
    let store = SqliteStore::open(&path).unwrap();
    let f = failure(&store);
    let (writer, locked, _release) = other_writer(&path, OTHER_WRITER_HOLDS);
    locked.recv().unwrap();

    let outcome = store.add_confirmation(&confirmation(&f.id), 500);

    writer.join().unwrap();
    assert_eq!(outcome.unwrap(), ConfirmationOutcome::Stored);
    cleanup(&path);
}

#[test]
fn a_write_that_waits_longer_than_the_busy_timeout_fails_with_a_clear_error() {
    let path = temp_db("timeout");
    let store = SqliteStore::open(&path).unwrap();
    let f = failure(&store);
    let (writer, locked, release) = other_writer(&path, OTHER_WRITER_CAP);
    locked.recv().unwrap();
    let started = Instant::now();

    let outcome = store.add_confirmation(&confirmation(&f.id), 500);

    let waited = started.elapsed();
    release.send(()).ok();
    writer.join().unwrap();
    let err = outcome.expect_err("the file stayed locked");
    assert!(err.to_string().contains("locked"), "{err}");
    assert!(waited >= BUSY_TIMEOUT / 2, "gave up after {waited:?}");
    assert!(waited < HANG_LIMIT, "hung for {waited:?}");
    cleanup(&path);
}

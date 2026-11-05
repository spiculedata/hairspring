//! Helpers shared by the read-pool and WAL-limit tests of the SQLite store.
#![allow(dead_code)]
use hairspring_store::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// The longest a step may take before the test calls it a hang.
pub const HANG_LIMIT: Duration = Duration::from_secs(30);

pub fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hairspring-read-pool-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("db.sqlite")
}

pub fn cleanup(path: &Path) {
    std::fs::remove_dir_all(path.parent().unwrap()).ok();
}

pub fn new_run(s: &dyn Store, created_at: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: "t".into(),
            backend: "hermit".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        created_at,
    )
    .unwrap()
}

pub fn run_count(c: &Connection) -> i64 {
    c.query_row("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
        .unwrap()
}

/// Holds `n` read connections at once, each inside a read call, until the returned sender is dropped. Returns once all
/// `n` are inside their calls, which only happens if `n` reads can be in progress together.
pub fn hold_readers(
    s: &Arc<SqliteStore>,
    n: usize,
) -> (Vec<thread::JoinHandle<()>>, mpsc::Sender<()>) {
    let (inside_tx, inside_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
    let handles = (0..n)
        .map(|_| {
            let s = Arc::clone(s);
            let inside_tx = inside_tx.clone();
            let release_rx = Arc::clone(&release_rx);
            thread::spawn(move || {
                s.read_with(|c| {
                    run_count(c);
                    inside_tx.send(()).unwrap();
                    // Wait until released: every holder stops when the sender is dropped.
                    let _ = release_rx.lock().unwrap().recv();
                })
                .unwrap();
            })
        })
        .collect();
    for i in 0..n {
        inside_rx
            .recv_timeout(HANG_LIMIT)
            .unwrap_or_else(|_| panic!("only {i} of {n} reads were in progress together"));
    }
    (handles, release_tx)
}

pub fn release(handles: Vec<thread::JoinHandle<()>>, release_tx: mpsc::Sender<()>) {
    drop(release_tx);
    for h in handles {
        h.join().unwrap();
    }
}

/// Waits, without sleeping, until `done` holds or the hang limit passes.
pub fn spin_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + HANG_LIMIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::yield_now();
    }
}

/// A sibling file of the database: `-wal` or `-shm`.
pub fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The size of the database's WAL file, 0 when there is none.
pub fn wal_len(path: &Path) -> u64 {
    std::fs::metadata(sibling(path, "-wal")).map_or(0, |m| m.len())
}

/// Whether `call` runs on a read connection: with every read connection held, a read queues, while anything on the
/// writer finishes. Returns that and `call`'s result, once the held connections are released.
pub fn runs_on_a_reader<R: Send + 'static>(
    s: &Arc<SqliteStore>,
    call: impl FnOnce(&SqliteStore) -> R + Send + 'static,
) -> (bool, R) {
    let (handles, release_tx) = hold_readers(s, s.read_connections());
    let (done_tx, done_rx) = mpsc::channel();
    let s2 = Arc::clone(s);
    let caller = thread::spawn(move || done_tx.send(call(&s2)).unwrap());
    let mut finished = None;
    spin_until("the call to queue or finish", || {
        if let Ok(r) = done_rx.try_recv() {
            finished = Some(r);
            return true;
        }
        s.readers_waiting() == 1
    });
    let on_reader = finished.is_none();
    release(handles, release_tx);
    let out = finished.unwrap_or_else(|| done_rx.recv_timeout(HANG_LIMIT).unwrap());
    caller.join().unwrap();
    (on_reader, out)
}

//! The WAL limit of a file-backed SQLite store: with reads overlapping without a gap, the WAL file stays bounded
//! because the writer pauses the read connections, waits for them (a bounded time) and checkpoints; a read held past
//! the wait makes it skip and try again later; and closing the store checkpoints.
//!
//! Interleavings are arranged with channels and the pool's pause, never with sleeps; the time limits only turn a hang
//! into a failure.
mod read_pool_support;
use hairspring_store::*;
use read_pool_support::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

/// A 2 KB event payload, as in the measurements in docs/architecture.md.
fn payload() -> String {
    format!("{{\"pad\":\"{}\"}}", "x".repeat(2000))
}

#[test]
fn the_writer_sets_the_journal_size_limit() {
    let path = temp_db("journal-limit");
    let s = SqliteStore::open(&path).unwrap();
    let limit: i64 = s.with_connection(|c| {
        c.query_row("PRAGMA journal_size_limit", [], |r| r.get(0))
            .unwrap()
    });
    assert_eq!(limit, JOURNAL_SIZE_LIMIT);
    const { assert!(WAL_CHECKPOINT_THRESHOLD as i64 >= JOURNAL_SIZE_LIMIT) };
    drop(s);
    cleanup(&path);
}

/// Readers that always overlap: each read call, once it has read, stays in progress until another reader is inside
/// its own call (or `HANDOVER` passes, which only happens while the pool is paused), so from the first read until the
/// writer stops there is never a moment with no read in progress, and SQLite on its own never starts the WAL again.
#[test]
fn the_wal_stays_bounded_while_reads_overlap_without_a_gap() {
    const READERS: usize = 3;
    const WRITES: usize = 3000;
    const THRESHOLD: u64 = 1024 * 1024;
    const BOUND: u64 = 4 * THRESHOLD;
    const HANDOVER: Duration = Duration::from_millis(20);
    const GC_EVERY: usize = 300;
    let path = temp_db("bounded");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    s.set_wal_checkpoint_threshold(THRESHOLD);
    let run = new_run(&*s, 1);
    let old = new_run(&*s, 2);
    assert!(s
        .finish_run(&old.id, RunState::Passed, 1, 1, None, 3)
        .unwrap());
    let inside = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    for _ in 0..READERS {
        let (s, inside, stop, id) = (
            Arc::clone(&s),
            Arc::clone(&inside),
            Arc::clone(&stop),
            run.id.clone(),
        );
        readers.push(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                s.read_with(|c| {
                    inside.fetch_add(1, Ordering::SeqCst);
                    run_count(c);
                    let until = Instant::now() + HANDOVER;
                    while inside.load(Ordering::SeqCst) < 2 && Instant::now() < until {
                        thread::yield_now();
                    }
                    inside.fetch_sub(1, Ordering::SeqCst);
                })
                .unwrap();
                s.list_events(&id, 0, 50).unwrap();
                s.retention_scan(100).unwrap();
            }
        }));
    }
    // Retention holds the writer for a report after every GC_EVERY writes, while the reads go on.
    let written = Arc::new(AtomicUsize::new(0));
    let gc = {
        let (s, written, old) = (Arc::clone(&s), Arc::clone(&written), old.id.clone());
        thread::spawn(move || {
            for n in 1..=WRITES / GC_EVERY {
                spin_until("the writer to go on", || {
                    written.load(Ordering::SeqCst) >= n * GC_EVERY
                });
                let report = s
                    .retention_delete(std::slice::from_ref(&old), 100, true)
                    .unwrap();
                assert!(report.error.is_none(), "{:?}", report.error);
            }
        })
    };
    spin_until("the reads to overlap", || inside.load(Ordering::SeqCst) > 0);
    let payload = payload();
    let mut largest = 0;
    for i in 0..WRITES {
        s.add_event(&run.id, "progress", &payload, i as i64)
            .unwrap();
        largest = largest.max(wal_len(&path));
        written.store(i + 1, Ordering::SeqCst);
    }
    join_within(gc);
    stop.store(true, Ordering::SeqCst);
    for r in readers {
        join_within(r);
    }
    let (done, _skipped) = s.wal_checkpoints();
    assert!(done > 0, "the writer never checkpointed");
    assert!(
        largest < BOUND,
        "the WAL reached {largest} bytes, over {BOUND} (threshold {THRESHOLD}; {done} checkpoints)"
    );
    assert_eq!(s.list_events(&run.id, 0, 10_000).unwrap().len(), WRITES);
    drop(Arc::into_inner(s).unwrap());
    cleanup(&path);
}

#[test]
fn a_checkpoint_waits_for_the_reads_in_progress_and_holds_new_ones_until_it_is_done() {
    let path = temp_db("drain");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let run = new_run(&*s, 1);
    s.add_event(&run.id, "progress", &payload(), 2).unwrap();
    let (handles, release_tx) = hold_readers(&s, 1);
    let checkpoint = {
        let s2 = Arc::clone(&s);
        thread::spawn(move || s2.checkpoint_wal(HANG_LIMIT))
    };
    spin_until("the reads to pause", || s.reads_paused());
    // Idle connections are left, but a read that arrives during the pause waits for it to end.
    let (done_tx, done_rx) = mpsc::channel();
    let late = {
        let s2 = Arc::clone(&s);
        thread::spawn(move || done_tx.send(s2.list_runs(10).unwrap().len()).unwrap())
    };
    spin_until("the late read to queue", || {
        assert!(done_rx.try_recv().is_err(), "a read ran during the pause");
        s.readers_waiting() == 1
    });
    assert!(wal_len(&path) > 0);
    release(handles, release_tx);
    assert_eq!(join_within(checkpoint), WalCheckpoint::Done);
    assert_eq!(done_rx.recv_timeout(HANG_LIMIT).unwrap(), 1);
    join_within(late);
    assert_eq!(wal_len(&path), 0, "the checkpoint left the WAL file");
    assert!(!s.reads_paused());
    cleanup(&path);
}

#[test]
fn a_checkpoint_gives_up_when_a_read_runs_past_the_wait_and_reads_resume() {
    let path = temp_db("give-up");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let run = new_run(&*s, 1);
    let (handles, release_tx) = hold_readers(&s, 1);
    assert_eq!(
        s.checkpoint_wal(Duration::from_millis(50)),
        WalCheckpoint::ReadsStillRunning
    );
    assert!(!s.reads_paused());
    // The held read is still in progress; other reads and writes carry on.
    assert_eq!(s.get_run(&run.id).unwrap().unwrap().id, run.id);
    new_run(&*s, 2);
    release(handles, release_tx);
    assert_eq!(s.list_runs(10).unwrap().len(), 2);
    cleanup(&path);
}

#[test]
fn after_a_skipped_checkpoint_the_writer_waits_before_trying_again() {
    let path = temp_db("retry");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let run = new_run(&*s, 1);
    s.set_wal_checkpoint_threshold(0);
    let (handles, release_tx) = hold_readers(&s, 1);
    let started = Instant::now();
    for i in 0..10 {
        s.add_event(&run.id, "progress", "{}", i).unwrap();
    }
    // The first write waited for the held read and skipped; the others came before the retry time.
    assert!(
        started.elapsed() < WAL_CHECKPOINT_RETRY,
        "the writes were too slow to tell"
    );
    assert_eq!(s.wal_checkpoints(), (0, 1));
    release(handles, release_tx);
    cleanup(&path);
}

#[test]
fn the_writer_does_not_checkpoint_inside_a_transaction() {
    let path = temp_db("in-tx");
    let s = SqliteStore::open(&path).unwrap();
    s.set_wal_checkpoint_threshold(0);
    s.with_connection(|c| c.execute_batch("BEGIN IMMEDIATE").unwrap());
    assert_eq!(s.wal_checkpoints(), (0, 0));
    s.with_connection(|c| c.execute_batch("COMMIT").unwrap());
    assert_eq!(s.wal_checkpoints(), (1, 0));
    drop(s);
    cleanup(&path);
}

#[test]
fn closing_the_store_checkpoints_even_while_another_connection_keeps_the_wal() {
    let path = temp_db("close-checkpoint");
    let s = SqliteStore::open(&path).unwrap();
    let run = new_run(&s, 1);
    // Another connection, idle: SQLite's own close then leaves the WAL alone, since the store is not the last.
    let other = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(run_count(&other), 1);
    s.add_event(&run.id, "progress", &payload(), 2).unwrap();
    assert!(wal_len(&path) > 0);
    drop(s);
    assert!(sibling(&path, "-wal").exists());
    assert_eq!(wal_len(&path), 0, "closing the store did not checkpoint");
    assert_eq!(run_count(&other), 1);
    drop(other);
    cleanup(&path);
}

#[test]
fn a_checkpoint_blocked_by_a_read_in_another_process_gives_up_at_once() {
    let path = temp_db("other-reader");
    let s = SqliteStore::open(&path).unwrap();
    let run = new_run(&s, 1);
    // A read transaction on a connection the store does not own, as another process would hold.
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("BEGIN").unwrap();
    assert_eq!(run_count(&other), 1);
    s.add_event(&run.id, "progress", &payload(), 2).unwrap();
    let started = Instant::now();
    assert_eq!(s.checkpoint_wal(HANG_LIMIT), WalCheckpoint::Blocked);
    assert!(
        started.elapsed() < BUSY_TIMEOUT / 2,
        "the checkpoint waited {:?} for the other reader",
        started.elapsed()
    );
    // The writer has its busy timeout back.
    let timeout: i64 = s.with_connection(|c| {
        c.query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap()
    });
    assert_eq!(timeout, BUSY_TIMEOUT.as_millis() as i64);
    other.execute_batch("COMMIT").unwrap();
    assert_eq!(s.checkpoint_wal(HANG_LIMIT), WalCheckpoint::Done);
    drop(other);
    drop(s);
    cleanup(&path);
}

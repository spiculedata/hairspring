//! The read connections of a file-backed SQLite store: reads run side by side on their own connections, each read call
//! sees one committed snapshot that includes every write that had returned before it began, a read connection can
//! never write, a panic during a read loses no connection, and an in-memory store keeps its single connection.
//!
//! Nothing here sleeps to arrange an interleaving. A read that must stay in progress waits on a channel inside
//! [`SqliteStore::read_with`]; the time limits below only turn a hang into a failure.
mod read_pool_support;
use hairspring_store::*;
use read_pool_support::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

#[test]
fn a_file_store_has_the_default_number_of_read_connections() {
    let path = temp_db("default");
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.read_connections(), DEFAULT_READ_CONNECTIONS);
    const { assert!(DEFAULT_READ_CONNECTIONS > 1) };
    drop(s);
    let s = SqliteStore::open_with_read_connections(&path, 2).unwrap();
    assert_eq!(s.read_connections(), 2);
    drop(s);
    cleanup(&path);
}

#[test]
fn a_slow_read_does_not_hold_up_another_read() {
    let path = temp_db("slow");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let run = new_run(&*s, 1);
    let (handles, release_tx) = hold_readers(&s, 1);

    // The slow read is still in progress; a second read must finish without waiting for it.
    let (done_tx, done_rx) = mpsc::channel();
    let s2 = Arc::clone(&s);
    let id = run.id.clone();
    let reader = thread::spawn(move || {
        let got = s2.get_run(&id).unwrap().map(|r| r.id);
        let listed = s2.list_runs(10).unwrap().len();
        done_tx.send((got, listed)).unwrap();
    });
    let (got, listed) = done_rx
        .recv_timeout(HANG_LIMIT)
        .expect("a read waited for another read to finish");
    assert_eq!((got, listed), (Some(run.id), 1));
    join_within(reader);
    release(handles, release_tx);
    cleanup(&path);
}

#[test]
fn every_read_connection_can_be_in_use_at_once_and_the_next_read_queues() {
    let path = temp_db("exhaust");
    let readers = 3;
    let s = Arc::new(SqliteStore::open_with_read_connections(&path, readers).unwrap());
    new_run(&*s, 1);
    let (handles, release_tx) = hold_readers(&s, readers);

    // Every connection is busy: the next read waits in the queue rather than opening another connection.
    let (done_tx, done_rx) = mpsc::channel();
    let s2 = Arc::clone(&s);
    let queued = thread::spawn(move || done_tx.send(s2.list_runs(10).unwrap().len()).unwrap());
    spin_until("the read to queue", || {
        assert!(
            done_rx.try_recv().is_err(),
            "a read finished while every read connection was in use"
        );
        s.readers_waiting() == 1
    });
    // A write never waits for the read connections.
    new_run(&*s, 2);
    release(handles, release_tx);
    assert_eq!(done_rx.recv_timeout(HANG_LIMIT).unwrap(), 2);
    join_within(queued);
    assert_eq!(s.readers_waiting(), 0);
    cleanup(&path);
}

#[test]
fn queued_reads_are_served_in_arrival_order() {
    let path = temp_db("fifo");
    let s = Arc::new(SqliteStore::open_with_read_connections(&path, 1).unwrap());
    let (handles, release_tx) = hold_readers(&s, 1);
    let (order_tx, order_rx) = mpsc::channel();
    let mut queued = Vec::new();
    for i in 0..4 {
        let s2 = Arc::clone(&s);
        let order_tx = order_tx.clone();
        queued.push(thread::spawn(move || {
            s2.read_with(|_| order_tx.send(i).unwrap()).unwrap();
        }));
        spin_until("the read to queue", || s.readers_waiting() == i + 1);
    }
    release(handles, release_tx);
    let order: Vec<usize> = (0..4)
        .map(|_| order_rx.recv_timeout(HANG_LIMIT).unwrap())
        .collect();
    assert_eq!(order, vec![0, 1, 2, 3]);
    for q in queued {
        join_within(q);
    }
    cleanup(&path);
}

#[test]
fn a_read_sees_every_write_that_returned_before_it() {
    let path = temp_db("ryw");
    let s = SqliteStore::open(&path).unwrap();
    // Put every read connection through a read first, so none of them is fresh.
    for _ in 0..DEFAULT_READ_CONNECTIONS * 2 {
        s.list_runs(1).unwrap();
    }
    let run = new_run(&s, 1);
    assert_eq!(s.get_run(&run.id).unwrap().unwrap().id, run.id);
    for i in 1..=200u64 {
        s.update_progress(&run.id, i, i).unwrap();
        assert_eq!(s.get_run(&run.id).unwrap().unwrap().steps, i);
        let id = s.add_event(&run.id, "progress", "{}", i as i64).unwrap();
        assert_eq!(
            s.list_events(&run.id, 0, 1000).unwrap().last().unwrap().id,
            id
        );
    }
    cleanup(&path);
}

#[test]
fn a_read_connection_opened_before_a_write_sees_it_in_its_next_read() {
    let path = temp_db("ryw-held");
    let s = Arc::new(SqliteStore::open_with_read_connections(&path, 1).unwrap());
    // The one read connection reads before the write, inside a call that is still in progress when the write returns.
    let (before_tx, before_rx) = mpsc::channel();
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let (after_tx, after_rx) = mpsc::channel();
    let s2 = Arc::clone(&s);
    let held = thread::spawn(move || {
        s2.read_with(|c| {
            before_tx.send(run_count(c)).unwrap();
            go_rx.recv().unwrap();
            // Still the same read call: the snapshot it began with.
            after_tx.send(run_count(c)).unwrap();
        })
        .unwrap();
    });
    assert_eq!(before_rx.recv_timeout(HANG_LIMIT).unwrap(), 0);
    let run = new_run(&*s, 1);
    go_tx.send(()).unwrap();
    assert_eq!(
        after_rx.recv_timeout(HANG_LIMIT).unwrap(),
        0,
        "one read call must see one snapshot"
    );
    join_within(held);
    // The same connection, in a new read call, sees the write.
    assert_eq!(s.get_run(&run.id).unwrap().unwrap().id, run.id);
    assert_eq!(s.read_with(run_count).unwrap(), 1);
    cleanup(&path);
}

#[test]
fn a_read_connection_cannot_write() {
    let path = temp_db("query-only");
    let s = SqliteStore::open(&path).unwrap();
    let err = s
        .read_with(|c| {
            c.execute(
                "INSERT INTO workers(id,name,caps_json,registered_at,last_seen) VALUES ('w','w','{}',1,1)",
                [],
            )
        })
        .unwrap()
        .expect_err("a read connection wrote");
    assert_eq!(
        err.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ReadOnly),
        "{err}"
    );
    assert_eq!(s.worker_count().unwrap(), 0);
    // The settings every read connection carries.
    let settings = s
        .read_with(|c| {
            let get = |p: &str| -> i64 {
                c.query_row(&format!("PRAGMA {p}"), [], |r| r.get(0))
                    .unwrap()
            };
            (get("query_only"), get("foreign_keys"), get("busy_timeout"))
        })
        .unwrap();
    assert_eq!(settings, (1, 1, BUSY_TIMEOUT.as_millis() as i64));
    cleanup(&path);
}

#[test]
fn an_in_memory_store_keeps_its_single_connection() {
    let s = SqliteStore::open_in_memory().unwrap();
    assert_eq!(s.read_connections(), 0);
    let run = new_run(&s, 1);
    assert_eq!(s.get_run(&run.id).unwrap().unwrap().id, run.id);
    // Reads go to the one connection, which can write.
    assert_eq!(s.read_with(run_count).unwrap(), 1);
    let path = temp_db("none");
    let s = SqliteStore::open_with_read_connections(&path, 0).unwrap();
    assert_eq!(s.read_connections(), 0);
    let run = new_run(&s, 1);
    assert_eq!(s.get_run(&run.id).unwrap().unwrap().id, run.id);
    drop(s);
    cleanup(&path);
}

#[test]
fn a_panic_during_a_read_loses_no_read_connection() {
    let path = temp_db("panic");
    let readers = 2;
    let s = Arc::new(SqliteStore::open_with_read_connections(&path, readers).unwrap());
    new_run(&*s, 1);
    for _ in 0..readers * 2 {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.read_with(|c| {
                run_count(c);
                panic!("a read that panics");
            })
        }));
        assert!(outcome.is_err());
    }
    // Every connection is back, out of its read transaction, and usable.
    let (handles, release_tx) = hold_readers(&s, readers);
    new_run(&*s, 2);
    release(handles, release_tx);
    assert_eq!(s.list_runs(10).unwrap().len(), 2);
    assert_eq!(s.readers_waiting(), 0);
    cleanup(&path);
}

#[test]
fn the_conformance_reads_and_writes_mixed_with_gc_never_surface_busy_or_stale_reads() {
    const WRITES: u64 = 150;
    const READERS: usize = 6;
    let path = temp_db("stress");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    // Old finished runs for retention to report on and delete while everything else goes on.
    let old: Vec<String> = (0..20)
        .map(|i| {
            let r = new_run(&*s, 10 + i);
            assert!(s
                .finish_run(&r.id, RunState::Passed, 1, 1, None, 20 + i)
                .unwrap());
            r.id
        })
        .collect();
    let live = new_run(&*s, 1_000_000);
    // The progress the writer has seen returned; a reader that starts after reading it must see at least that much.
    let published = Arc::new(AtomicU64::new(0));
    let start = Arc::new(Barrier::new(READERS + 2));
    let mut handles = Vec::new();
    {
        let (s, published, start, live) = (
            Arc::clone(&s),
            Arc::clone(&published),
            Arc::clone(&start),
            live.id.clone(),
        );
        handles.push(thread::spawn(move || {
            start.wait();
            for i in 1..=WRITES {
                s.update_progress(&live, i, i).unwrap();
                s.add_event(&live, "progress", "{}", i as i64).unwrap();
                new_run(&*s, 1_000_000 + i as i64);
                published.store(i, Ordering::SeqCst);
            }
        }));
    }
    for _ in 0..READERS {
        let (s, published, start, live) = (
            Arc::clone(&s),
            Arc::clone(&published),
            Arc::clone(&start),
            live.id.clone(),
        );
        handles.push(thread::spawn(move || {
            start.wait();
            loop {
                let seen = published.load(Ordering::SeqCst);
                let run = s.get_run(&live).unwrap().unwrap();
                assert!(run.steps >= seen, "stale read: {} < {seen}", run.steps);
                let events = s.list_events(&live, 0, 10_000).unwrap().len() as u64;
                assert!(events >= seen, "stale read: {events} events < {seen}");
                assert!(s.list_runs(1000).unwrap().len() as u64 >= seen);
                s.retention_scan(100).unwrap();
                if seen == WRITES {
                    break;
                }
            }
        }));
    }
    {
        let (s, start, old) = (Arc::clone(&s), Arc::clone(&start), old.clone());
        handles.push(thread::spawn(move || {
            start.wait();
            let lock = s
                .retention_lock()
                .unwrap()
                .expect("the retention lock is free");
            for _ in 0..5 {
                let report = s.retention_delete(&old, 100, true).unwrap();
                assert!(report.error.is_none(), "{:?}", report.error);
            }
            let report = s.retention_delete(&old, 100, false).unwrap();
            assert!(report.error.is_none(), "{:?}", report.error);
            assert!(report.applied);
            drop(lock);
        }));
    }
    for h in handles {
        join_within(h);
    }
    for id in &old {
        assert!(s.get_run(id).unwrap().is_none(), "retention kept {id}");
    }
    assert_eq!(s.get_run(&live.id).unwrap().unwrap().steps, WRITES);
    cleanup(&path);
}

#[test]
fn a_queued_read_is_woken_when_the_read_ahead_of_it_takes_one_of_two_returned_connections() {
    // Two connections come back while two reads wait. Whichever waiter wakes first, the second must not sleep while a
    // connection sits idle: the first wakes it after taking its own. The pause makes both come back before either
    // waiter can take one, and the first waiter then holds its connection until the second is inside its read, so
    // without that wake-up the second sleeps until the time limit. The order the two wake in is up to the scheduler,
    // so the round is repeated.
    const ROUNDS: usize = 100;
    let path = temp_db("wake-next");
    let s = Arc::new(SqliteStore::open_with_read_connections(&path, 2).unwrap());
    for round in 0..ROUNDS {
        let (handles, release_tx) = hold_readers(&s, 2);
        let (second_in_tx, second_in_rx) = mpsc::channel::<()>();
        let first = {
            let s2 = Arc::clone(&s);
            thread::spawn(move || {
                s2.read_with(|_| second_in_rx.recv_timeout(Duration::from_secs(5)).is_ok())
                    .unwrap()
            })
        };
        spin_until("the first read to queue", || s.readers_waiting() == 1);
        let second = {
            let s2 = Arc::clone(&s);
            thread::spawn(move || s2.read_with(|_| second_in_tx.send(()).unwrap()).unwrap())
        };
        spin_until("the second read to queue", || s.readers_waiting() == 2);
        let pause = {
            let s2 = Arc::clone(&s);
            thread::spawn(move || s2.checkpoint_wal(HANG_LIMIT))
        };
        spin_until("the reads to pause", || s.reads_paused());
        release(handles, release_tx);
        assert_eq!(join_within(pause), WalCheckpoint::Done);
        assert!(
            join_within(first),
            "round {round}: a read slept while a connection was idle"
        );
        join_within(second);
    }
    cleanup(&path);
}

#[test]
fn closing_the_store_after_every_read_connection_was_used_removes_the_wal_files() {
    let path = temp_db("close");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let (handles, release_tx) = hold_readers(&s, DEFAULT_READ_CONNECTIONS);
    new_run(&*s, 1);
    release(handles, release_tx);
    assert_eq!(s.list_runs(10).unwrap().len(), 1);
    assert!(sibling(&path, "-wal").exists() && sibling(&path, "-shm").exists());
    drop(Arc::into_inner(s).unwrap());
    assert!(!sibling(&path, "-wal").exists(), "the WAL file is left");
    assert!(
        !sibling(&path, "-shm").exists(),
        "the shared-memory file is left"
    );
    // Everything written is in the database file itself.
    let s = SqliteStore::open(&path).unwrap();
    assert_eq!(s.list_runs(10).unwrap().len(), 1);
    drop(s);
    cleanup(&path);
}

#[test]
fn reading_checks_runs_on_a_read_connection_and_a_write_does_not() {
    let path = temp_db("which");
    let s = Arc::new(SqliteStore::open(&path).unwrap());
    let (on_reader, checks) = runs_on_a_reader(&s, |s| s.list_failure_checks("fail_none").unwrap());
    assert!(on_reader, "list_failure_checks ran on the writer");
    assert!(checks.is_empty());
    let (on_reader, batch) = runs_on_a_reader(&s, |s| {
        s.list_checks_of_failures(&["fail_a", "fail_b"]).unwrap()
    });
    assert!(on_reader, "list_checks_of_failures ran on the writer");
    assert_eq!(batch.len(), 2);
    let (on_reader, _) = runs_on_a_reader(&s, |s| s.register_worker("w", "{}", 1).unwrap());
    assert!(!on_reader, "a write waited for a read connection");
    cleanup(&path);
}

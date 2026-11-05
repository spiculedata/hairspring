//! Turning off SQLite's memory statistics at start-up. This file holds one test, so its process has no other thread
//! using SQLite when the test begins, which is what the function requires.
use hairspring_store::*;

#[test]
fn memory_statistics_turn_off_before_sqlite_starts_and_cannot_change_after() {
    // SAFETY: this is the only test in this binary and nothing has used SQLite yet.
    assert!(unsafe { disable_sqlite_memory_statistics() });
    let s = SqliteStore::open_in_memory().unwrap();
    s.worker_count().unwrap();
    // With the statistics off SQLite counts nothing.
    // SAFETY: a read of a counter SQLite keeps; it takes no argument and is safe from any thread.
    assert_eq!(unsafe { rusqlite::ffi::sqlite3_memory_used() }, 0);
    // Once SQLite has started the setting is fixed, and asking again changes nothing.
    // SAFETY: the store above is used only on this thread, and nothing else is running.
    assert!(!unsafe { disable_sqlite_memory_statistics() });
}

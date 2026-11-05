//! Keeping the WAL file of a file-backed SQLite store bounded.
//!
//! SQLite starts the WAL file again from the beginning only when every frame in it has been copied to the database and
//! no read is using it. With read connections working side by side, reads can overlap without a gap for as long as the
//! load lasts, so the automatic checkpoint copies frames but the file never starts again and grows with every write.
//!
//! - **Threshold.** After each use of the writer, the store looks at the WAL file's size. Once it reaches
//!   [`WAL_CHECKPOINT_THRESHOLD`], the writer first runs a `PASSIVE` checkpoint (which copies what it can while reads go
//!   on), then pauses the read connections, waits until every lent one is back (at most [`WAL_DRAIN_WAIT`]), runs a
//!   `TRUNCATE` checkpoint and lends again.
//! - **Why `TRUNCATE`.** With no read in progress `RESTART` and `TRUNCATE` both copy the rest and start the WAL again,
//!   but `RESTART` leaves the file at its largest size until the next commit cuts it to the journal size limit;
//!   `TRUNCATE` sets it to zero bytes at once, for one `ftruncate` per threshold crossing.
//! - **Bounded.** A read that arrives during the pause waits at most [`WAL_DRAIN_WAIT`] plus the `TRUNCATE`
//!   checkpoint, which after the `PASSIVE` pass copies only the frames written since the reads in progress began. If
//!   the lent connections are not back within [`WAL_DRAIN_WAIT`], the checkpoint is skipped, reads resume, and the next
//!   attempt waits at least [`WAL_CHECKPOINT_RETRY`].
//! - **Never busy.** The `TRUNCATE` checkpoint runs with no busy handler, so a read held by another process makes it
//!   report "blocked" at once rather than wait; that counts as a skip. Nothing here returns an error to a caller: the WAL
//!   stays valid whatever happens, and the next attempt tries again.
//! - **Journal size limit.** The writer also sets `PRAGMA journal_size_limit` to [`JOURNAL_SIZE_LIMIT`], so after any
//!   reset (automatic or not) the first commit cuts a larger file back to that size.
//! - **Close.** Dropping the store closes the read connections, then runs a `TRUNCATE` checkpoint on the writer before
//!   it closes.
use crate::sqlite::{SqliteStore, BUSY_TIMEOUT};
use rusqlite::Connection;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The WAL file size at which the writer pauses reads to checkpoint and start the file again.
pub const WAL_CHECKPOINT_THRESHOLD: u64 = 16 * 1024 * 1024;

/// The longest the writer waits for reads in progress to finish before it checkpoints; reads that arrive meanwhile wait
/// in the queue for at most this long plus the checkpoint itself.
pub const WAL_DRAIN_WAIT: Duration = Duration::from_millis(250);

/// After a skipped checkpoint, the least time before the writer tries again.
pub const WAL_CHECKPOINT_RETRY: Duration = Duration::from_secs(2);

/// `PRAGMA journal_size_limit` on the writer: after the WAL starts again, the first commit cuts a larger file back to
/// this size. About the size the automatic checkpoint (1000 pages of 4 KiB) lets the WAL reach.
pub const JOURNAL_SIZE_LIMIT: i64 = 4 * 1024 * 1024;

/// What one attempt to checkpoint and start the WAL again came to. For tests; not part of the supported interface.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalCheckpoint {
    /// The WAL was copied to the database and set to zero bytes.
    Done,
    /// Reads in progress did not finish within the wait; nothing was checkpointed.
    ReadsStillRunning,
    /// SQLite could not finish the checkpoint (another process reading or writing, or an error).
    Blocked,
}

/// The WAL file a store watches, and how its checkpoints went.
pub(crate) struct WalLimit {
    wal_path: PathBuf,
    threshold: AtomicU64,
    /// No attempt before this instant; set after a skip.
    retry_at: Mutex<Option<Instant>>,
    done: AtomicU64,
    skipped: AtomicU64,
    /// The longest the read connections have stayed paused for one attempt, in microseconds.
    longest_pause_us: AtomicU64,
}

impl WalLimit {
    pub(crate) fn new(db_path: &Path) -> WalLimit {
        let mut wal = db_path.as_os_str().to_owned();
        wal.push("-wal");
        WalLimit {
            wal_path: PathBuf::from(wal),
            threshold: AtomicU64::new(WAL_CHECKPOINT_THRESHOLD),
            retry_at: Mutex::new(None),
            done: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            longest_pause_us: AtomicU64::new(0),
        }
    }

    fn over_threshold(&self) -> bool {
        // A missing file (or one that cannot be read) is no reason to checkpoint.
        std::fs::metadata(&self.wal_path)
            .is_ok_and(|m| m.len() >= self.threshold.load(Ordering::Relaxed))
    }

    fn due(&self) -> bool {
        self.retry_at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none_or(|at| Instant::now() >= at)
    }

    fn record(&self, outcome: WalCheckpoint, pause: Duration) {
        self.longest_pause_us
            .fetch_max(pause.as_micros() as u64, Ordering::Relaxed);
        let mut retry_at = self.retry_at.lock().unwrap_or_else(|e| e.into_inner());
        if outcome == WalCheckpoint::Done {
            self.done.fetch_add(1, Ordering::Relaxed);
            *retry_at = None;
        } else {
            self.skipped.fetch_add(1, Ordering::Relaxed);
            *retry_at = Some(Instant::now() + WAL_CHECKPOINT_RETRY);
        }
    }
}

/// The writer's connection, held behind its mutex. When it is given back, the store checks the WAL's size.
pub(crate) struct Writer<'a> {
    store: &'a SqliteStore,
    guard: MutexGuard<'a, Connection>,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(store: &'a SqliteStore, guard: MutexGuard<'a, Connection>) -> Writer<'a> {
        Writer { store, guard }
    }
}

impl Deref for Writer<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.guard
    }
}

impl DerefMut for Writer<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.guard
    }
}

impl Drop for Writer<'_> {
    fn drop(&mut self) {
        // While unwinding, give the connection back as it is; the next write checks again.
        if !std::thread::panicking() {
            self.store.limit_wal(&self.guard);
        }
    }
}

impl SqliteStore {
    /// Checkpoints and starts the WAL again when it has reached the threshold, unless a recent attempt was skipped or
    /// the writer is inside a transaction. Called with the writer's mutex held.
    fn limit_wal(&self, conn: &Connection) {
        let Some(wal) = &self.wal else { return };
        if conn.is_autocommit() && wal.over_threshold() && wal.due() {
            let (outcome, pause) = self.checkpoint_wal_on(conn, WAL_DRAIN_WAIT);
            wal.record(outcome, pause);
        }
    }

    /// One attempt: a `PASSIVE` pass while reads go on, then, with the read connections paused and drained (waiting at
    /// most `wait`), a `TRUNCATE` checkpoint. Returns the outcome and how long reads were paused.
    fn checkpoint_wal_on(&self, conn: &Connection, wait: Duration) -> (WalCheckpoint, Duration) {
        // Copies what the reads in progress allow, so the pause below has little left to copy. A failure here only
        // leaves more for the TRUNCATE checkpoint, which reports its own.
        let _ = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
        let truncate = || match truncate_wal(conn) {
            Ok(true) => WalCheckpoint::Done,
            Ok(false) | Err(_) => WalCheckpoint::Blocked,
        };
        let paused = Instant::now();
        let outcome = match &self.readers {
            None => truncate(),
            Some(pool) => pool
                .with_reads_drained(wait, truncate)
                .unwrap_or(WalCheckpoint::ReadsStillRunning),
        };
        (outcome, paused.elapsed())
    }

    /// Runs one checkpoint attempt now, whatever the WAL's size, waiting at most `wait` for reads in progress. For
    /// tests; not part of the supported interface.
    #[doc(hidden)]
    pub fn checkpoint_wal(&self, wait: Duration) -> WalCheckpoint {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        self.checkpoint_wal_on(&conn, wait).0
    }

    /// The longest the writer has paused the read connections for one checkpoint attempt of the WAL limit. For
    /// measurements; not part of the supported interface.
    #[doc(hidden)]
    pub fn longest_wal_pause(&self) -> Duration {
        self.wal.as_ref().map_or(Duration::ZERO, |w| {
            Duration::from_micros(w.longest_pause_us.load(Ordering::Relaxed))
        })
    }

    /// Changes the WAL size at which the writer checkpoints. For tests; not part of the supported interface.
    #[doc(hidden)]
    pub fn set_wal_checkpoint_threshold(&self, bytes: u64) {
        if let Some(wal) = &self.wal {
            wal.threshold.store(bytes, Ordering::Relaxed);
        }
    }

    /// Checkpoints the WAL limit has run and skipped since the store opened. For tests; not part of the supported
    /// interface.
    #[doc(hidden)]
    pub fn wal_checkpoints(&self) -> (u64, u64) {
        self.wal.as_ref().map_or((0, 0), |w| {
            (
                w.done.load(Ordering::Relaxed),
                w.skipped.load(Ordering::Relaxed),
            )
        })
    }

    /// The close-time checkpoint: with the read connections already closed, copy the WAL and set it to zero bytes.
    pub(crate) fn checkpoint_on_close(&mut self) {
        if self.wal.is_none() {
            return;
        }
        let conn = self.conn.get_mut().unwrap_or_else(|e| e.into_inner());
        // Nothing can be reported from a drop. A checkpoint that cannot finish leaves a valid WAL, which SQLite's own
        // close (or the next open) checkpoints.
        let _ = truncate_wal(conn);
    }
}

/// Runs a `TRUNCATE` checkpoint with no busy handler; returns whether it finished. The busy timeout is put back
/// whatever happens.
fn truncate_wal(conn: &Connection) -> rusqlite::Result<bool> {
    conn.busy_timeout(Duration::ZERO)?;
    let blocked = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        r.get::<_, i64>(0)
    });
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(blocked? == 0)
}

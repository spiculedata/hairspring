//! The read-only connections of a file-backed SQLite store.
//!
//! In WAL mode readers do not block one another or the writer, but one connection runs one statement at a time, so a
//! store with a single connection makes every read wait for every other. The pool holds a fixed number of extra
//! connections that only read; a read call takes one, runs inside one read transaction (so it sees one committed
//! snapshot, which includes every write that had returned before the call began) and gives it back.
//!
//! - **Bounded.** The pool never holds more than its size. When every connection is in use, callers queue and are
//!   served in arrival order, each as soon as a connection comes back.
//! - **Read only.** Every connection has `PRAGMA query_only=ON`, so a write through one fails with `SQLITE_READONLY`,
//!   including a write to a temporary table. It also has the writer's busy timeout and `foreign_keys` setting.
//! - **No poisoning.** The pool's lock is held only to take or return a connection, never while SQL runs. A connection
//!   is returned when the [`Reader`] that holds it drops, panics included; one left inside a transaction is rolled back
//!   first, and if that fails it is closed and a fresh one is opened when next needed.
use crate::sqlite::BUSY_TIMEOUT;
use crate::types::{Result, StoreError};
use crate::SqliteStore;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard};

impl SqliteStore {
    /// Runs one read call: on a read connection inside one read transaction, or on the writer's connection when the
    /// store has no read connections (the writer's mutex then keeps writes out for the length of the call).
    pub(crate) fn read<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let Some(pool) = &self.readers else {
            return f(&self.conn());
        };
        let mut reader = pool.take()?;
        // Deferred: the snapshot is taken by the first statement, so it follows every write that returned before this
        // call began. A panic in `f` drops the transaction, which rolls it back, and then the reader, which returns it.
        let tx = reader.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// The number of read-only connections; 0 when reads share the writer's connection.
    pub fn read_connections(&self) -> usize {
        self.readers.as_ref().map_or(0, ReadPool::size)
    }

    /// Runs `f` as one read call, on the connection a store read would use. For tests that hold a read in progress or
    /// check what a read connection allows; not part of the supported interface.
    #[doc(hidden)]
    pub fn read_with<R>(&self, f: impl FnOnce(&Connection) -> R) -> Result<R> {
        self.read(|c| Ok(f(c)))
    }

    /// Read calls waiting for a read connection. For tests; not part of the supported interface.
    #[doc(hidden)]
    pub fn readers_waiting(&self) -> usize {
        self.readers.as_ref().map_or(0, ReadPool::waiting)
    }
}

pub(crate) struct ReadPool {
    path: PathBuf,
    size: usize,
    state: Mutex<PoolState>,
    returned: Condvar,
}

struct PoolState {
    idle: Vec<Connection>,
    /// Connections in existence: idle plus lent out. Never more than the pool's size.
    open: usize,
    /// Tickets of the callers waiting, in arrival order; only the first may take a connection.
    queue: VecDeque<u64>,
    next_ticket: u64,
}

/// A connection lent out by the pool; it goes back when this drops.
pub(crate) struct Reader<'a> {
    pool: &'a ReadPool,
    conn: Option<Connection>,
}

impl ReadPool {
    /// Opens `size` read connections to the database file at `path`, which must already exist with its schema current.
    pub(crate) fn open(path: &Path, size: usize) -> Result<ReadPool> {
        let idle = (0..size)
            .map(|_| open_reader(path))
            .collect::<Result<Vec<_>>>()?;
        Ok(ReadPool {
            path: path.to_path_buf(),
            size,
            state: Mutex::new(PoolState {
                idle,
                open: size,
                queue: VecDeque::new(),
                next_ticket: 0,
            }),
            returned: Condvar::new(),
        })
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    /// Callers waiting for a connection.
    pub(crate) fn waiting(&self) -> usize {
        self.lock().queue.len()
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        // The lock is never held while SQL runs or user code is called, so a poisoned lock cannot hide a half-made
        // change; carry on with the state as it is.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a connection, waiting in turn when all are in use.
    pub(crate) fn take(&self) -> Result<Reader<'_>> {
        let mut st = self.lock();
        let ticket = st.next_ticket;
        st.next_ticket += 1;
        st.queue.push_back(ticket);
        loop {
            if st.queue.front() == Some(&ticket) {
                if let Some(conn) = st.idle.pop() {
                    st.queue.pop_front();
                    drop(st);
                    // The next in line may find another idle connection.
                    self.returned.notify_all();
                    return Ok(self.lend(conn));
                }
                if st.open < self.size {
                    // A connection was closed after a failed rollback; open its replacement outside the lock.
                    st.open += 1;
                    st.queue.pop_front();
                    drop(st);
                    self.returned.notify_all();
                    return match open_reader(&self.path) {
                        Ok(conn) => Ok(self.lend(conn)),
                        Err(e) => {
                            self.lock().open -= 1;
                            self.returned.notify_all();
                            Err(e)
                        }
                    };
                }
            }
            st = self.returned.wait(st).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn lend(&self, conn: Connection) -> Reader<'_> {
        Reader {
            pool: self,
            conn: Some(conn),
        }
    }

    fn give_back(&self, conn: Connection) {
        let conn = out_of_transaction(conn);
        let mut st = self.lock();
        match conn {
            Some(conn) => st.idle.push(conn),
            None => st.open -= 1,
        }
        drop(st);
        self.returned.notify_all();
    }
}

/// The connection, outside any transaction, or `None` if it could not be brought out of one (it is then closed).
fn out_of_transaction(conn: Connection) -> Option<Connection> {
    if !conn.is_autocommit() {
        // Only a read call that unwound past its own transaction leaves one open, and a read has nothing to keep. A
        // failed rollback is not reported here: the check below closes the connection instead of reusing it.
        conn.execute_batch("ROLLBACK").ok();
    }
    conn.is_autocommit().then_some(conn)
}

impl Deref for Reader<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
            .as_ref()
            .expect("a lent connection is present until drop")
    }
}

impl DerefMut for Reader<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.conn
            .as_mut()
            .expect("a lent connection is present until drop")
    }
}

impl Drop for Reader<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.give_back(conn);
        }
    }
}

/// One read connection: the file must exist (no create), and the settings match the writer's where they matter.
fn open_reader(path: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(path, flags).map_err(|e| {
        StoreError::Backend(format!(
            "cannot open a read connection to {}: {e}",
            path.display()
        ))
    })?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "query_only", "ON")?;
    Ok(conn)
}

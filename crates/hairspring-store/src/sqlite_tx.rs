//! Write transactions on SQLite.
use crate::types::Result;
use rusqlite::{Connection, Transaction, TransactionBehavior};

/// Starts a transaction that takes the write lock at once. A deferred transaction reads first and asks for the lock
/// later; if another process has committed in between, SQLite refuses the upgrade straight away (`SQLITE_BUSY`) and
/// the busy timeout never applies. Taking the lock at `BEGIN` makes the wait happen there, bounded by [`crate::BUSY_TIMEOUT`].
pub(crate) fn write_tx(conn: &mut Connection) -> Result<Transaction<'_>> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

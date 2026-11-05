//! Process-wide SQLite settings, made once at start-up.
use rusqlite::ffi;
use std::ffi::c_int;

/// Turns off SQLite's memory statistics for the whole process. While they are on (SQLite's default), every allocation
/// SQLite makes takes one process-wide mutex to count it, so read connections working at once queue on that mutex: with
/// it on, three heavy reads at once took longer side by side than one after another. Nothing in Hairspring reads the
/// statistics or sets a heap limit, which are the only things they serve.
///
/// Returns whether the setting took effect. Once SQLite has started in this process (any connection opened) the
/// setting can no longer change, and this returns `false` and changes nothing.
///
/// # Safety
///
/// No other thread may be using SQLite while this runs (the rule of `sqlite3_config`). Call it at the start of `main`,
/// before any thread is started.
#[must_use = "the setting does not take effect once SQLite has started; report it when this is false"]
pub unsafe fn disable_sqlite_memory_statistics() -> bool {
    // SAFETY: the caller guarantees no other thread is inside SQLite. The call takes one integer argument for this
    // option, passed as a C int, and only changes SQLite's own configuration.
    unsafe { ffi::sqlite3_config(ffi::SQLITE_CONFIG_MEMSTATUS, 0 as c_int) == ffi::SQLITE_OK }
}

//! Which connection each `Store` method of the SQLite store uses, read from the source. A method that only reads
//! belongs on a read connection (`self.read(..)`); the writer is for writes and for reads that share a transaction with
//! a write. A method newly placed on the writer fails this test until it is added to [`ON_THE_WRITER`] with a reason,
//! so a read-only method cannot end up behind the writer's mutex unnoticed.
use std::collections::BTreeSet;

const SQLITE: &str = include_str!("../src/sqlite.rs");
const SQLITE_JOBS: &str = include_str!("../src/sqlite_jobs.rs");

/// The `Store` methods that use the writer, each for a reason.
const ON_THE_WRITER: &[&str] = &[
    // Writes, or reads and writes in one transaction.
    "add_confirmation",
    "add_event",
    "add_failure",
    "add_tree_branch",
    "add_tree_node",
    "cancel_run",
    "cancel_run_with_event",
    "complete_job",
    "create_run",
    "create_tree",
    "enqueue_job",
    "fail_expired_jobs",
    "fail_job",
    "finish_job",
    "finish_run",
    "finish_run_as_error",
    "heartbeat_job",
    "ingest_trials",
    "lease_job",
    "lease_run_job",
    "mark_running",
    "prune_workers",
    "put_blob",
    "put_run_identity",
    "record_branch_verify",
    "register_worker",
    "register_worker_with_secret",
    "set_node_verified",
    "set_worker_secret_hash",
    "set_worker_trusted",
    "touch_worker",
    "update_progress",
    // The report (a dry run) keeps its working lists in temporary tables, which a read-only connection cannot create.
    "retention_delete",
    // Reads only the database path from the writer, to name the lock file; no SQL runs.
    "retention_lock",
];

/// The body of each `fn` in one `impl` block of `src`, by name: the text from the `fn` line to the next one at the
/// same indent.
fn methods(src: &str, impl_line: &str) -> Vec<(String, String)> {
    let start = src
        .find(impl_line)
        .unwrap_or_else(|| panic!("`{impl_line}` not found"));
    let mut out: Vec<(String, String)> = Vec::new();
    for line in src[start..].lines().skip(1) {
        if line == "}" {
            break;
        }
        let head = line
            .strip_prefix("    fn ")
            .or_else(|| line.strip_prefix("    pub(crate) fn "));
        if let Some(head) = head {
            let name: String = head
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect();
            out.push((name, String::new()));
        }
        if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out
}

fn uses_writer(body: &str) -> bool {
    body.contains(".conn()")
}

#[test]
fn every_store_method_that_only_reads_uses_a_read_connection() {
    let jobs = methods(SQLITE_JOBS, "impl SqliteStore {");
    let store = methods(SQLITE, "impl Store for SqliteStore {");
    assert!(store.len() > 60, "found only {} methods", store.len());
    let mut on_writer = BTreeSet::new();
    let mut on_reader = BTreeSet::new();
    for (name, body) in &store {
        // A job method hands over to its `jobs_` helper; follow it.
        let body = jobs
            .iter()
            .find(|(helper, _)| body.contains(&format!("self.{helper}(")))
            .map_or(body, |(_, helper_body)| helper_body);
        if uses_writer(body) {
            on_writer.insert(name.as_str());
        } else if body.contains("self.read(") {
            on_reader.insert(name.as_str());
        }
    }
    let expected: BTreeSet<&str> = ON_THE_WRITER.iter().copied().collect();
    let unexpected: Vec<_> = on_writer.difference(&expected).collect();
    assert!(
        unexpected.is_empty(),
        "these Store methods use the writer: {unexpected:?}. A method that only reads belongs on a read connection \
         (`self.read(|c| ..)`); one that writes goes in ON_THE_WRITER with its reason."
    );
    let missing: Vec<_> = expected.difference(&on_writer).collect();
    assert!(
        missing.is_empty(),
        "ON_THE_WRITER names methods that no longer use the writer: {missing:?}"
    );
    // The reads this guards, including the ones a status-filtered failure list makes.
    for read in [
        "get_run",
        "list_failures_page",
        "list_failure_checks",
        "list_checks_of_failures",
        "list_events",
        "retention_scan",
    ] {
        assert!(
            on_reader.contains(read),
            "{read} is not on a read connection"
        );
    }
}

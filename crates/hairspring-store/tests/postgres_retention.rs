//! Retention on PostgreSQL racing the writers it must not lose data to. Each test holds a lock on a raw connection so
//! that `retention_delete` is stopped at a known point, starts the competing write, waits (by asking the server who
//! blocks whom, never by sleeping) until each side is where the test needs it, and then lets go. Skipped unless
//! `HAIRSPRING_TEST_PG_URL` is set.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use common::tree_support::*;
use hairspring_store::*;
use postgres::Client;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The longest any wait in these tests may take before the test fails instead of hanging.
const WAIT_LIMIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);

fn store(url: &str, schema: &str) -> Arc<PgStore> {
    Arc::new(PgStore::connect_in_schema(url, schema, 4).unwrap())
}

fn backend_pid(c: &mut Client) -> i32 {
    c.query_one("SELECT pg_backend_pid()", &[]).unwrap().get(0)
}

/// How many sessions are waiting on a lock that `holder` holds.
fn blocked_by(c: &mut Client, holder: i32) -> i64 {
    c.query_one(
        "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
        &[&holder],
    )
    .unwrap()
    .get(0)
}

/// How many sessions are waiting on a session that is itself waiting on `holder`.
fn blocked_behind(c: &mut Client, holder: i32) -> i64 {
    c.query_one(
        "SELECT count(*) FROM pg_stat_activity a WHERE EXISTS (SELECT 1 FROM pg_stat_activity g \
         WHERE $1 = ANY(pg_blocking_pids(g.pid)) AND g.pid = ANY(pg_blocking_pids(a.pid)))",
        &[&holder],
    )
    .unwrap()
    .get(0)
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT_LIMIT, "timed out waiting: {what}");
        std::thread::sleep(POLL);
    }
}

fn spawn_gc(s: &Arc<PgStore>, runs: Vec<String>) -> JoinHandle<RetentionReport> {
    let s = Arc::clone(s);
    std::thread::spawn(move || s.retention_delete(&runs, i64::MAX, false).unwrap())
}

/// A tree whose root run is finished, with its jobs closed, so retention may remove it.
fn finished_tree(s: &dyn Store) -> (RunRecord, TreeRecord) {
    let root_run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &root_run.id, &passes(2));
    let tree = make_tree(s, &root_run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&root_run.id, 0));
    finish(s, &root_run.id);
    close_jobs(s, &root_run.id);
    (root_run, tree)
}

/// A branch added while retention is deciding about its tree must end up either with its tree (the tree kept) or not
/// added at all (the tree gone first). It must never be added and then lose its branch row with the tree.
#[test]
fn a_branch_added_while_gc_removes_its_tree_is_never_left_without_it() {
    let Some(url) = url() else { return };
    let schema = new_schema("rg");
    let s = store(&url, &schema);
    let (root_run, tree) = finished_tree(&*s);
    // The run a branch request created: queued, with the tree's identity.
    let branch_run = run_with_identity(&*s, "a\n");

    // Hold the root run's job rows: retention locks them before it decides, so it stops there.
    let mut holder = raw(&url, &schema);
    let holder_pid = backend_pid(&mut holder);
    let mut hold = holder.transaction().unwrap();
    hold.execute(
        "SELECT id FROM jobs WHERE run_id=$1 FOR UPDATE",
        &[&root_run.id],
    )
    .unwrap();
    let mut watch = admin(&url);

    let gc = spawn_gc(&s, vec![root_run.id.clone()]);
    wait_until("gc to wait on the held job rows", || {
        blocked_by(&mut watch, holder_pid) >= 1
    });
    let adder = {
        let s = Arc::clone(&s);
        let new = branch_for(&tree.id, &branch_run.id, 0, &"b".repeat(64));
        std::thread::spawn(move || s.add_tree_branch(&new, 9).unwrap())
    };
    wait_until("the branch to be added, or to wait behind gc", || {
        adder.is_finished() || blocked_behind(&mut watch, holder_pid) >= 1
    });
    hold.rollback().unwrap();

    let report = gc.join().unwrap();
    let added = adder.join().unwrap();
    assert_eq!(report.error, None, "gc decides cleanly: {report:?}");
    let tree_left = s.get_tree(&tree.id).unwrap();
    let branch_left = s.get_tree_branch(&branch_run.id).unwrap();
    assert!(
        s.get_run(&branch_run.id).unwrap().is_some(),
        "the branch run was never named"
    );
    match &added {
        BranchOutcome::Added(_) => {
            assert!(
                branch_left.is_some() && tree_left.is_some(),
                "an added branch keeps its row and its tree: {report:?}"
            );
            assert_eq!(report.trees_kept, vec![tree.id.clone()]);
            assert!(s.get_run(&root_run.id).unwrap().is_some());
        }
        BranchOutcome::Rejected(TreeRejection::TreeNotFound) => {
            assert!(branch_left.is_none() && tree_left.is_none());
            assert_eq!(report.trees_deleted, vec![tree.id.clone()]);
        }
        other => panic!("unexpected outcome {other:?}"),
    }
}

/// A finished run outside any tree that becomes a branch while retention is deciding about it is kept with its tree,
/// not half deleted.
#[test]
fn a_run_that_becomes_a_branch_while_gc_decides_is_kept() {
    let Some(url) = url() else { return };
    let schema = new_schema("rl");
    let s = store(&url, &schema);
    let (_, tree) = finished_tree(&*s);
    let lone = run_with_identity(&*s, "a\n");
    ingest_verdicts(&*s, &lone.id, &passes(1));
    finish(&*s, &lone.id);
    close_jobs(&*s, &lone.id);

    let mut holder = raw(&url, &schema);
    let holder_pid = backend_pid(&mut holder);
    let mut hold = holder.transaction().unwrap();
    hold.execute(
        "SELECT id FROM jobs WHERE run_id=$1 FOR UPDATE",
        &[&lone.id],
    )
    .unwrap();
    let mut watch = admin(&url);
    let gc = spawn_gc(&s, vec![lone.id.clone()]);
    wait_until("gc to wait on the held job rows", || {
        blocked_by(&mut watch, holder_pid) >= 1
    });
    let added = s
        .add_tree_branch(&branch_for(&tree.id, &lone.id, 0, &"c".repeat(64)), 9)
        .unwrap();
    assert!(matches!(added, BranchOutcome::Added(_)), "{added:?}");
    hold.rollback().unwrap();
    let report = gc.join().unwrap();

    assert_eq!(report.error, None, "{report:?}");
    assert!(report.run_ids.is_empty());
    assert_eq!(report.kept.get("tree_kept"), Some(&1));
    assert!(s.get_run(&lone.id).unwrap().is_some());
    assert!(s.get_tree_branch(&lone.id).unwrap().is_some());
}

/// An ingest that stores a blob another run already holds (ON CONFLICT DO NOTHING takes no row lock) and a trial that
/// refers to it, in a transaction not yet committed when retention looks: retention must wait for it and keep the blob.
#[test]
fn a_blob_a_trial_is_being_stored_against_is_not_removed() {
    let Some(url) = url() else { return };
    let schema = new_schema("rb");
    let s = store(&url, &schema);
    let old = new_run(&*s, "sim");
    let mut b = batch(&passes(1), 0);
    b.trials[0].output = Some(b"output both runs produce".to_vec());
    let job = ingest_batch(&*s, &old.id, &b);
    assert!(s.complete_job(&job.id, "w").unwrap());
    finish(&*s, &old.id);
    let hash = sha256_hex(b"output both runs produce");
    let live = new_run(&*s, "sim");

    // What the ingest of `live` does, stopped before it commits: the blob insert, then a trial naming the blob.
    let mut ingest = raw(&url, &schema);
    let ingest_pid = backend_pid(&mut ingest);
    let mut tx = ingest.transaction().unwrap();
    tx.execute(
        "INSERT INTO blobs(hash,data) VALUES ($1,$2) ON CONFLICT DO NOTHING",
        &[&hash, &b"output both runs produce".as_slice()],
    )
    .unwrap();
    tx.batch_execute(&format!(
        "CREATE TEMP TABLE copy AS SELECT * FROM trials WHERE run_id='{}'; \
         UPDATE copy SET run_id='{}'; INSERT INTO trials SELECT * FROM copy;",
        old.id, live.id
    ))
    .unwrap();
    let mut watch = admin(&url);

    let gc = spawn_gc(&s, vec![old.id.clone()]);
    wait_until("gc to finish, or to wait for the ingest", || {
        gc.is_finished() || blocked_by(&mut watch, ingest_pid) >= 1
    });
    tx.commit().unwrap();
    let report = gc.join().unwrap();

    assert_eq!(report.error, None);
    assert_eq!(report.run_ids, vec![old.id.clone()]);
    let trial = s.list_trials(&live.id, None, 10).unwrap().remove(0);
    assert_eq!(trial.output_ref.as_deref(), Some(hash.as_str()));
    assert!(
        s.get_blob(&hash).unwrap().is_some(),
        "the new trial's output is still stored"
    );
    assert_eq!(report.blobs, 0, "{report:?}");
}

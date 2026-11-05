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

/// A unit that fails on PostgreSQL is reported with the database's own message and SQLSTATE, so a deadlock, a foreign
/// key that kept a row, and a missing table can be told apart.
#[test]
fn a_failed_unit_reports_the_database_message_and_sqlstate() {
    let Some(url) = url() else { return };
    let schema = new_schema("re");
    let s = store(&url, &schema);
    let old = new_run(&*s, "sim");
    ingest_verdicts(&*s, &old.id, &passes(1));
    finish(&*s, &old.id);
    close_jobs(&*s, &old.id);
    raw(&url, &schema)
        .batch_execute("DROP TABLE run_coverage")
        .unwrap();

    let report = s
        .retention_delete(std::slice::from_ref(&old.id), i64::MAX, false)
        .unwrap();

    let error = report.error.expect("the unit fails");
    assert!(error.contains("SQLSTATE 42P01"), "{error}");
    assert!(error.contains("run_coverage"), "{error}");
    assert!(report.run_ids.is_empty());
}

/// A dry run takes no lock a writer waits for, and waits for none: it finishes while another transaction holds the
/// candidate run's job rows, its run row and the tree row (as a finishing worker or a branch request would) and is
/// storing a blob (as an ingest would). It used to take the job rows and then the blobs table lock.
#[test]
fn a_dry_run_finishes_while_writers_hold_what_an_apply_would_lock() {
    let Some(url) = url() else { return };
    let schema = new_schema("rd");
    let s = store(&url, &schema);
    let (root_run, tree) = finished_tree(&*s);
    let old = new_run(&*s, "sim");
    let mut b = batch(&passes(1), 0);
    b.trials[0].output = Some(b"old output".to_vec());
    let job = ingest_batch(&*s, &old.id, &b);
    assert!(s.complete_job(&job.id, "w").unwrap());
    finish(&*s, &old.id);

    let mut holder = raw(&url, &schema);
    let holder_pid = backend_pid(&mut holder);
    let mut hold = holder.transaction().unwrap();
    hold.batch_execute(&format!(
        "SELECT id FROM jobs WHERE run_id IN ('{old}','{root}') FOR UPDATE; \
         SELECT id FROM runs WHERE id IN ('{old}','{root}') FOR UPDATE; \
         SELECT id FROM trees WHERE id='{tree}' FOR UPDATE; \
         INSERT INTO blobs(hash,data) VALUES ('stored meanwhile','\\x00'::bytea) ON CONFLICT DO NOTHING;",
        old = old.id,
        root = root_run.id,
        tree = tree.id
    ))
    .unwrap();
    let mut watch = admin(&url);
    let dry = {
        let s = Arc::clone(&s);
        let runs = vec![old.id.clone(), root_run.id.clone()];
        std::thread::spawn(move || s.retention_delete(&runs, i64::MAX, true).unwrap())
    };
    wait_until("the dry run to finish without waiting", || {
        assert_eq!(
            blocked_by(&mut watch, holder_pid),
            0,
            "the dry run waited for the writer"
        );
        dry.is_finished()
    });
    hold.commit().unwrap();
    let report = dry.join().unwrap();

    assert_eq!(report.error, None);
    assert_eq!(report.runs_deleted(), 2);
    assert_eq!(report.trees_deleted, vec![tree.id.clone()]);
    assert_eq!(report.blobs, 1);
    assert!(s.get_run(&old.id).unwrap().is_some(), "nothing was removed");
    assert!(s.get_tree(&tree.id).unwrap().is_some());
}

/// Two gc commands on one schema take turns; a store in another schema of the same database is not held up.
#[test]
fn only_one_holder_of_the_retention_lock_at_a_time() {
    let Some(url) = url() else { return };
    let schema = new_schema("rk");
    let first = store(&url, &schema);
    let second = store(&url, &schema);
    let held = first.retention_lock().unwrap().expect("free at first");
    assert!(second.retention_lock().unwrap().is_none());
    assert!(first.retention_lock().unwrap().is_none());
    let elsewhere = store(&url, &new_schema("rk"));
    assert!(elsewhere.retention_lock().unwrap().is_some());
    drop(held);
    let again = second.retention_lock().unwrap();
    assert!(again.is_some(), "released when dropped");
}

/// Workers ingest (output shared with old runs, and their own), lease and finish jobs, while a report loop and an apply
/// of old runs that share that output run at the same time. No call fails (a deadlock would abort one of them), and no
/// trial is left naming a blob that is gone. Each side runs until the others have done their share; nothing sleeps.
#[test]
fn workers_a_report_and_an_apply_at_once_never_deadlock_or_lose_a_blob() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    const OLD_RUNS: usize = 60;
    const ROUNDS_PER_WORKER: u64 = 4;
    let Some(url) = url() else { return };
    let schema = new_schema("rs");
    let s = Arc::new(PgStore::connect_in_schema(&url, &schema, 16).unwrap());
    let shared = b"output old and live runs share".to_vec();
    let mut old_ids = Vec::new();
    for i in 0..OLD_RUNS {
        let r = if i % 5 == 0 {
            run_with_identity(&*s, "a\n")
        } else {
            new_run(&*s, "sim")
        };
        let mut b = batch(&passes(3), 0);
        b.trials[0].output = Some(shared.clone());
        b.trials[1].output = Some(format!("old unique {i}").into_bytes());
        let job = ingest_batch(&*s, &r.id, &b);
        assert!(s.complete_job(&job.id, "w").unwrap());
        if i % 5 == 0 {
            let t = make_tree(&*s, &r, Some(sha256_hex(b"a\n")));
            add(&*s, &t.id, &root(&r.id, 0));
        }
        finish(&*s, &r.id);
        close_jobs(&*s, &r.id);
        old_ids.push(r.id);
    }
    let applied = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(AtomicU64::new(0));
    let fail = |errors: &AtomicU64, what: &str, e: &dyn std::fmt::Display| {
        eprintln!("{what}: {e}");
        errors.fetch_add(1, Ordering::Relaxed);
    };
    let mut workers = Vec::new();
    for w in 0..6 {
        let (s, applied, errors, shared) = (
            Arc::clone(&s),
            applied.clone(),
            errors.clone(),
            shared.clone(),
        );
        workers.push(std::thread::spawn(move || {
            let worker = format!("w{w}");
            let mut round = 0u64;
            while round < ROUNDS_PER_WORKER || !applied.load(Ordering::Relaxed) {
                let r = new_run(&*s, "sim");
                s.enqueue_job(&r.id, "run", "{}", 2).unwrap();
                let job = s
                    .lease_run_job(&r.id, &worker, 600, 10)
                    .unwrap()
                    .expect("its own job");
                for k in 0..3u64 {
                    let mut b = batch(&passes(2), k * 2);
                    b.trials[0].output = Some(shared.clone());
                    b.trials[1].output = Some(format!("live {w} {round} {k}").into_bytes());
                    match s.ingest_trials(&job.id, &worker, true, &b, 11) {
                        Ok(IngestOutcome::Applied { .. }) => {}
                        Ok(o) => fail(&errors, "ingest outcome", &format!("{o:?}")),
                        Err(e) => fail(&errors, "ingest", &e),
                    }
                }
                let res = JobResult {
                    state: RunState::Passed,
                    steps: 1,
                    trials: 6,
                    error: None,
                    failure: None,
                    more_failures: vec![],
                    stats: Default::default(),
                    event_payload_json: "{}".into(),
                };
                if let Err(e) = s.finish_job(&job.id, &worker, &res, 20) {
                    fail(&errors, "finish", &e);
                }
                round += 1;
            }
        }));
    }
    let reporter = {
        let (s, applied, errors, ids) = (
            Arc::clone(&s),
            applied.clone(),
            errors.clone(),
            old_ids.clone(),
        );
        std::thread::spawn(move || {
            let mut reports = 0u64;
            while reports < 2 || !applied.load(Ordering::Relaxed) {
                match s.retention_delete(&ids, 100, true) {
                    Ok(r) if r.error.is_none() => {}
                    Ok(r) => fail(&errors, "report", &format!("{:?}", r.error)),
                    Err(e) => fail(&errors, "report", &e),
                }
                reports += 1;
            }
        })
    };
    let mut deleted = 0;
    for chunk in old_ids.chunks(7) {
        let r = s.retention_delete(chunk, 100, false).unwrap();
        deleted += r.run_ids.len();
        if let Some(e) = r.error {
            fail(&errors, "apply", &e);
        }
    }
    applied.store(true, Ordering::Relaxed);
    reporter.join().unwrap();
    for w in workers {
        w.join().unwrap();
    }

    assert_eq!(errors.load(Ordering::Relaxed), 0, "see the lines above");
    assert_eq!(deleted, OLD_RUNS);
    let dangling: i64 = raw(&url, &schema)
        .query_one(
            "SELECT count(*) FROM trials t WHERE (t.output_ref IS NOT NULL AND NOT EXISTS \
             (SELECT 1 FROM blobs b WHERE b.hash = t.output_ref)) OR (t.detail_ref IS NOT NULL AND NOT EXISTS \
             (SELECT 1 FROM blobs b WHERE b.hash = t.detail_ref))",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(dangling, 0, "a trial names a removed blob");
    assert!(s.get_blob(&sha256_hex(&shared)).unwrap().is_some());
}

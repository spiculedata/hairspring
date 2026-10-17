//! Completing a run's identity and creating the run's tree at the same time, on PostgreSQL. Skipped unless
//! `HAIRSPRING_TEST_PG_URL` is set (see `postgres.rs`); the CI workflow does not run the PostgreSQL tests.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use common::tree_support::*;
use hairspring_store::*;
use std::sync::Arc;
use std::time::Duration;

/// Time for a started operation to reach the lock it waits on.
const SETTLE: Duration = Duration::from_millis(600);

/// A run with an incomplete identity, and a connection that holds the run's row lock. Both operations are started
/// while the lock is held, `first` and then the other, so they queue in a known order; releasing the lock lets them
/// through in that order.
fn race(first_is_identity: bool) -> (IdentityOutcome, TreeOutcome, Arc<PgStore>, RunRecord) {
    let url = url().expect("checked by the caller");
    let schema = new_schema("ic");
    let s = Arc::new(PgStore::connect_in_schema(&url, &schema, 4).unwrap());
    let run = new_run(&*s, "sim");
    s.put_run_identity(&identity(&run.id, "p\n", false))
        .unwrap();

    let mut holder = raw(&url, &schema);
    holder.batch_execute("BEGIN").unwrap();
    holder
        .execute("SELECT 1 FROM runs WHERE id = $1 FOR UPDATE", &[&run.id])
        .unwrap();

    let complete = {
        let (s, id) = (s.clone(), run.id.clone());
        move || {
            std::thread::spawn(move || s.put_run_identity(&identity(&id, "full\n", true)).unwrap())
        }
    };
    let create = {
        let (s, run) = (s.clone(), run.clone());
        let tree = NewTree {
            run_id: run.id.clone(),
            backend: run.backend.clone(),
            target: run.target.clone(),
            identity_hash: None,
        };
        move || std::thread::spawn(move || s.create_tree(&tree, 5).unwrap())
    };
    let (identity_thread, tree_thread);
    if first_is_identity {
        identity_thread = complete();
        std::thread::sleep(SETTLE);
        tree_thread = create();
    } else {
        tree_thread = create();
        std::thread::sleep(SETTLE);
        identity_thread = complete();
    }
    std::thread::sleep(SETTLE);
    assert!(
        !identity_thread.is_finished() && !tree_thread.is_finished(),
        "both operations must be waiting on the run's lock"
    );
    holder.batch_execute("COMMIT").unwrap();
    (
        identity_thread.join().unwrap(),
        tree_thread.join().unwrap(),
        s,
        run,
    )
}

#[test]
fn completing_first_means_no_tree_without_point_identity_is_created() {
    if url().is_none() {
        return;
    }
    let (identity_outcome, tree_outcome, s, run) = race(true);
    assert_eq!(identity_outcome, IdentityOutcome::Completed);
    assert_eq!(
        tree_outcome,
        TreeOutcome::Rejected(TreeRejection::IdentityMismatch)
    );
    assert_eq!(s.get_tree(&run.id).unwrap(), None);
    assert!(s.get_run_identity(&run.id).unwrap().unwrap().complete);
}

#[test]
fn creating_the_tree_first_means_the_identity_stays_incomplete() {
    if url().is_none() {
        return;
    }
    let (identity_outcome, tree_outcome, s, run) = race(false);
    assert!(
        matches!(tree_outcome, TreeOutcome::Created(_)),
        "{tree_outcome:?}"
    );
    assert_eq!(
        identity_outcome,
        IdentityOutcome::Rejected(TreeRejection::Conflict)
    );
    assert_eq!(s.get_tree(&run.id).unwrap().unwrap().identity_hash, None);
    assert!(!s.get_run_identity(&run.id).unwrap().unwrap().complete);
}

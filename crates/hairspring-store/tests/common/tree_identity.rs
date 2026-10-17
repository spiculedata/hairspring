//! Conformance scenarios for completing a run's identity: an incomplete identity may be completed, once, and only
//! while no tree depends on the run.
#![allow(dead_code)]
use super::tree_support::*;
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_identity(make: &dyn Fn() -> Arc<dyn Store>) {
    an_incomplete_identity_can_be_completed(&*make());
    a_complete_identity_is_never_replaced(&*make());
    an_incomplete_identity_cannot_change_into_another_incomplete_one(&*make());
    an_identity_a_tree_depends_on_stays_as_it_is(&*make());
    completion_keeps_the_reporter_and_never_raises_trust(&*make());
}

fn an_incomplete_identity_can_be_completed(s: &dyn Store) {
    let run = new_run(s, "sim");
    let partial = identity(&run.id, "binary=old\n", false);
    assert_eq!(
        s.put_run_identity(&partial).unwrap(),
        IdentityOutcome::Stored
    );

    // A retried attempt on another binary reports the full inputs.
    let full = identity(&run.id, "binary=new\n", true);
    assert_eq!(
        s.put_run_identity(&full).unwrap(),
        IdentityOutcome::Completed
    );
    assert_eq!(s.get_run_identity(&run.id).unwrap(), Some(full.clone()));
    assert_eq!(
        s.put_run_identity(&full).unwrap(),
        IdentityOutcome::Duplicate
    );

    // The completed identity now roots a tree with point identity.
    let tree = make_tree(s, &run, Some(full.identity_hash.clone()));
    assert_eq!(
        tree.identity_hash.as_deref(),
        Some(full.identity_hash.as_str())
    );
}

fn a_complete_identity_is_never_replaced(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    let stored = s.get_run_identity(&run.id).unwrap();
    for offered in [
        identity(&run.id, "b\n", true),
        identity(&run.id, "a\n", false),
    ] {
        assert_eq!(
            s.put_run_identity(&offered).unwrap(),
            IdentityOutcome::Rejected(TreeRejection::Conflict)
        );
    }
    assert_eq!(s.get_run_identity(&run.id).unwrap(), stored);
}

fn an_incomplete_identity_cannot_change_into_another_incomplete_one(s: &dyn Store) {
    let run = new_run(s, "sim");
    let partial = identity(&run.id, "p\n", false);
    s.put_run_identity(&partial).unwrap();
    assert_eq!(
        s.put_run_identity(&identity(&run.id, "q\n", false))
            .unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Conflict)
    );
    assert_eq!(s.get_run_identity(&run.id).unwrap(), Some(partial));
}

fn an_identity_a_tree_depends_on_stays_as_it_is(s: &dyn Store) {
    // A tree rooted at the run was created without point identity; completing the run's identity afterwards would
    // contradict it.
    let run = new_run(s, "sim");
    let partial = identity(&run.id, "p\n", false);
    s.put_run_identity(&partial).unwrap();
    make_tree(s, &run, None);
    assert_eq!(
        s.put_run_identity(&identity(&run.id, "full\n", true))
            .unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Conflict)
    );
    assert_eq!(s.get_run_identity(&run.id).unwrap(), Some(partial));
}

fn completion_keeps_the_reporter_and_never_raises_trust(s: &dyn Store) {
    // Another reporter cannot complete (and so take over) an identity.
    let run = new_run(s, "sim");
    let partial = identity(&run.id, "p\n", false);
    s.put_run_identity(&partial).unwrap();
    let mut stranger = identity(&run.id, "full\n", true);
    stranger.reporter_id = "someone-else".into();
    assert_eq!(
        s.put_run_identity(&stranger).unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Conflict)
    );
    assert_eq!(s.get_run_identity(&run.id).unwrap(), Some(partial));

    // The same reporter completes, but an untrusted row stays untrusted.
    let run = new_run(s, "sim");
    let mut partial = identity(&run.id, "p\n", false);
    partial.reporter_trusted = false;
    s.put_run_identity(&partial).unwrap();
    let full = identity(&run.id, "full\n", true);
    assert!(full.reporter_trusted);
    assert_eq!(
        s.put_run_identity(&full).unwrap(),
        IdentityOutcome::Completed
    );
    let stored = s.get_run_identity(&run.id).unwrap().unwrap();
    assert!(stored.complete && !stored.reporter_trusted);
    assert_eq!(stored.identity_text, "full\n");

    // A trusted row completed by an untrusted report is lowered.
    let run = new_run(s, "sim");
    s.put_run_identity(&identity(&run.id, "p\n", false))
        .unwrap();
    let mut full = identity(&run.id, "full\n", true);
    full.reporter_trusted = false;
    assert_eq!(
        s.put_run_identity(&full).unwrap(),
        IdentityOutcome::Completed
    );
    assert!(
        !s.get_run_identity(&run.id)
            .unwrap()
            .unwrap()
            .reporter_trusted
    );
}

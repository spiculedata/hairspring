//! Conformance scenarios for a new attempt of a run's job (its tree nodes go first, and what depends on them blocks
//! it) and for hostile text in tree rows.
#![allow(dead_code)]
use super::tree_support::*;
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_attempts(make: &dyn Fn() -> Arc<dyn Store>) {
    a_new_attempt_removes_lineage_before_trials(&*make());
    hostile_text_in_tree_rows_is_stored_verbatim(&*make());
}

/// A run whose first attempt stored a tree, then a second attempt by another worker.
fn tree_with_two_attempts(s: &dyn Store) -> (RunRecord, Job, TreeRecord) {
    let run = run_with_identity(s, "a\n");
    let job = s.enqueue_job(&run.id, "run", "{}", 2).unwrap();
    s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    let first = batch(&passes(3), 0);
    s.ingest_trials(&job.id, "w1", true, &first, 1_100).unwrap();
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    add(s, &tree.id, &child(&run.id, 1, 0, 1));
    add(s, &tree.id, &child(&run.id, 2, 1, 1));
    let second = s.lease_job("w2", 10, 100_000).unwrap().unwrap();
    assert_eq!((second.id.as_str(), second.attempts), (job.id.as_str(), 2));
    (run, job, tree)
}

fn a_new_attempt_removes_lineage_before_trials(s: &dyn Store) {
    let (run, job, tree) = tree_with_two_attempts(s);
    let out = s
        .ingest_trials(&job.id, "w2", true, &batch(&passes(1), 0), 100_001)
        .unwrap();
    assert_eq!(
        out,
        IngestOutcome::Applied {
            inserted: 1,
            duplicates: 0
        }
    );
    assert_eq!(s.count_trials(&run.id).unwrap(), 1);
    assert_eq!(
        s.get_tree_node(&tree.id, 0).unwrap(),
        None,
        "lineage went with the trials"
    );
    assert_eq!(s.tree_node_of_trial(&run.id, 1).unwrap(), None);
    let t = s.get_tree(&tree.id).unwrap().unwrap();
    assert_eq!(
        t.node_count, 3,
        "the node cap counts numbers handed out, not rows: deleting nodes never lowers it"
    );
    assert!(
        s.get_run_identity(&run.id).unwrap().is_some(),
        "the run's identity is not attempt data"
    );
    // The new attempt's nodes are numbered after every earlier number, so a number never names two nodes.
    let fresh = add(s, &tree.id, &root(&run.id, 0));
    assert_eq!(fresh.node, 3);
    assert!(
        fresh.node > 2,
        "greater than every node of the earlier attempt"
    );
}

/// Adds a branch row the way nothing but a test can (the store refuses a branch below a running run): the guard in
/// `start_attempt` is a second line of defence and is exercised with this hook. Arguments: tree, run, source node.
pub type RawBranch<'a> = &'a dyn Fn(&str, &str, u64);

pub fn a_root_run_with_branches_cannot_start_a_new_attempt(s: &dyn Store, raw_branch: RawBranch) {
    let (run, job, tree) = tree_with_two_attempts(s);
    let b = run_with_identity(s, "a\n");
    raw_branch(&tree.id, &b.id, 2);
    let out = s
        .ingest_trials(&job.id, "w2", true, &batch(&passes(1), 0), 100_001)
        .unwrap();
    match out {
        IngestOutcome::Rejected(why) => assert!(why.contains("branch"), "{why}"),
        other => panic!("expected a rejection, got {other:?}"),
    }
    assert_eq!(s.count_trials(&run.id).unwrap(), 3, "nothing was deleted");
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().node_count, 3);
    assert!(s.get_tree_node(&tree.id, 2).unwrap().is_some());
}

pub fn a_branch_run_with_dependants_cannot_start_a_new_attempt(
    s: &dyn Store,
    raw_branch: RawBranch,
) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(2));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    add(s, &tree.id, &child(&run.id, 1, 0, 1));

    let b1 = run_with_identity(s, "a\n");
    raw_branch(&tree.id, &b1.id, 1);
    let job = s.enqueue_job(&b1.id, "run", "{}", 2).unwrap();
    s.lease_job("w1", 10, 1_000).unwrap().unwrap();
    s.ingest_trials(&job.id, "w1", true, &batch(&passes(2), 0), 1_100)
        .unwrap();
    let n = add(s, &tree.id, &child(&b1.id, 0, 1, 3));
    // A second branch starts at a node of the first branch run.
    let b2 = run_with_identity(s, "a\n");
    raw_branch(&tree.id, &b2.id, n.node);

    s.lease_job("w2", 10, 100_000).unwrap().unwrap();
    let out = s
        .ingest_trials(&job.id, "w2", true, &batch(&passes(1), 0), 100_001)
        .unwrap();
    assert!(matches!(out, IngestOutcome::Rejected(_)), "{out:?}");
    assert!(s.get_tree_node(&tree.id, n.node).unwrap().is_some());
    assert_eq!(s.count_trials(&b1.id).unwrap(), 2);
}

fn hostile_text_in_tree_rows_is_stored_verbatim(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(2));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    finish(s, &run.id);
    let evil = "x'); DROP TABLE trial_lineage; -- \u{202e}<script>alert(1)</script>";
    let mut node = child(&run.id, 1, 0, 1);
    node.reporter_id = evil.into();
    let n = add(s, &tree.id, &node);
    assert_eq!(n.reporter_id, evil);
    let b = run_with_identity(s, "a\n");
    let mut br = branch_for(&tree.id, &b.id, 0, &"9".repeat(64));
    br.command_json = evil.into();
    br.created_by = evil.into();
    s.add_tree_branch(&br, 9).unwrap();
    let got = s.get_tree_branch(&b.id).unwrap().unwrap();
    assert_eq!(
        (got.command_json.as_str(), got.created_by.as_str()),
        (evil, evil)
    );
    assert_eq!(
        s.get_tree_node(&tree.id, 1).unwrap().unwrap().reporter_id,
        evil
    );
}

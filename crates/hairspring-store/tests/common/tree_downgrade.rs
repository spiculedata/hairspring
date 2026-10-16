//! A version 3 database holding every kind of row the downgrade has to treat: a tree with a root run, a branch run
//! with its own trials, job, failure and event, an ordinary run, and blobs that are shared or used by one owner only.
#![allow(dead_code)]
use super::tree_support::*;
use hairspring_store::*;

pub struct DowngradeFixture {
    pub root_run: String,
    pub tree: String,
    pub branch_run: String,
    pub plain_run: String,
    /// Used by the tree's edge only; unreferenced once lineage is gone.
    pub edge_blob: String,
    /// Used by a trial of the plain run and a trial of the branch run.
    pub shared_blob: String,
    /// Used by the branch run only.
    pub branch_blob: String,
}

fn failure(run_id: &str, bundle_hash: &str) -> NewFailure {
    NewFailure {
        run_id: run_id.into(),
        target: "t".into(),
        message: "m".into(),
        original_len: 4,
        shrunk_len: 2,
        bundle_hash: bundle_hash.into(),
        steps_json: "[]".into(),
    }
}

/// `mark_branch` turns the run's `mode` into `branch` (the store has no call for it yet; the harness uses raw SQL).
pub fn build(s: &dyn Store, mark_branch: &dyn Fn(&str)) -> DowngradeFixture {
    let plain = new_run(s, "hermit");
    let mut b = batch(&passes(2), 0);
    b.trials[0].detail = Some(b"shared detail".to_vec());
    ingest_batch(s, &plain.id, &b);
    let shared_blob = s
        .get_trial(&plain.id, 0)
        .unwrap()
        .unwrap()
        .detail_ref
        .unwrap();

    let root_run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &root_run.id, &passes(3));
    let tree = make_tree(s, &root_run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&root_run.id, 0));
    let edge_blob = s.put_blob(b"actions from the cut").unwrap();
    let mut with_edge = child(&root_run.id, 1, 0, 2);
    with_edge.edge_ref = Some(edge_blob.clone());
    add(s, &tree.id, &with_edge);
    add(s, &tree.id, &child(&root_run.id, 2, 1, 2));

    let branch = run_with_identity(s, "a\n");
    let mut bb = batch(&passes(2), 0);
    bb.trials[0].detail = Some(b"shared detail".to_vec());
    bb.trials[1].detail = Some(b"branch only detail".to_vec());
    let job = ingest_batch(s, &branch.id, &bb);
    let branch_blob = s
        .get_trial(&branch.id, 1)
        .unwrap()
        .unwrap()
        .detail_ref
        .unwrap();
    assert_eq!(
        s.get_trial(&branch.id, 0)
            .unwrap()
            .unwrap()
            .detail_ref
            .as_deref(),
        Some(shared_blob.as_str())
    );
    let bundle = s.put_blob(b"branch failure bundle").unwrap();
    s.add_failure(&failure(&branch.id, &bundle), 7).unwrap();
    s.add_event(&branch.id, "progress", "{}", 8).unwrap();
    assert!(job.id.starts_with("job_"));
    finish(s, &root_run.id);
    let out = s
        .add_tree_branch(&branch_for(&tree.id, &branch.id, 2, &"b".repeat(64)), 9)
        .unwrap();
    assert!(matches!(out, BranchOutcome::Added(_)), "{out:?}");
    add(s, &tree.id, &child(&branch.id, 0, 2, 3));
    mark_branch(&branch.id);

    DowngradeFixture {
        root_run: root_run.id,
        tree: tree.id,
        branch_run: branch.id,
        plain_run: plain.id,
        edge_blob,
        shared_blob,
        branch_blob,
    }
}

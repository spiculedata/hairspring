//! Conformance scenarios for schema v3: run identity, trial trees, branch runs and the path queries.
#![allow(dead_code)]
use super::tree_support::*;
use super::{tree_attempts, tree_reads};
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_trees(make: &dyn Fn() -> Arc<dyn Store>) {
    identity_is_written_once_and_checked(&*make());
    tree_creation_checks_the_root_runs_identity(&*make());
    nodes_get_dense_numbers_depth_and_paths(&*make());
    shape_rules_are_enforced(&*make());
    parents_must_exist_in_the_same_tree(&*make());
    depth_and_fanout_caps(&*make());
    a_tree_without_point_identity_reads_but_takes_no_branches(&*make());
    branches_are_checked_and_deduplicated(&*make());
    branches_attach_only_below_finished_runs(&*make());
    a_run_cannot_be_a_root_and_a_branch(&*make());
    the_branch_cap_holds(&*make());
    tree_reads::run_all_reads(make);
    tree_attempts::run_all_attempts(make);
}

fn rejected<T: std::fmt::Debug>(r: Result<T>) -> T {
    r.unwrap()
}

fn identity_is_written_once_and_checked(s: &dyn Store) {
    let run = new_run(s, "sim");
    let id = identity(&run.id, "scheme=sim/1\ntarget=t\n", true);
    assert_eq!(s.get_run_identity(&run.id).unwrap(), None);
    assert_eq!(s.put_run_identity(&id).unwrap(), IdentityOutcome::Stored);
    assert_eq!(s.put_run_identity(&id).unwrap(), IdentityOutcome::Duplicate);
    assert_eq!(s.get_run_identity(&run.id).unwrap(), Some(id.clone()));

    let other = identity(&run.id, "scheme=sim/1\ntarget=other\n", true);
    assert_eq!(
        s.put_run_identity(&other).unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Conflict),
        "written once"
    );
    assert_eq!(s.get_run_identity(&run.id).unwrap(), Some(id.clone()));

    let mut wrong_hash = identity(&new_run(s, "sim").id, "x\n", true);
    wrong_hash.identity_hash = "0".repeat(64);
    assert!(matches!(
        s.put_run_identity(&wrong_hash).unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Invalid(_))
    ));
    let mut bad_scheme = identity(&new_run(s, "sim").id, "x\n", true);
    bad_scheme.scheme = "sim/9".into();
    assert!(matches!(
        s.put_run_identity(&bad_scheme).unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Invalid(_))
    ));
    let huge = "a".repeat(MAX_IDENTITY_TEXT_BYTES + 1);
    assert!(matches!(
        s.put_run_identity(&identity(&new_run(s, "sim").id, &huge, true))
            .unwrap(),
        IdentityOutcome::Rejected(TreeRejection::Invalid(_))
    ));
    assert_eq!(
        s.put_run_identity(&identity("run_missing", "x\n", true))
            .unwrap(),
        IdentityOutcome::Rejected(TreeRejection::RunNotFound)
    );
}

fn tree_creation_checks_the_root_runs_identity(s: &dyn Store) {
    let hash_of = |t: &str| sha256_hex(t.as_bytes());
    let new_tree = |run: &RunRecord, h: Option<String>| NewTree {
        run_id: run.id.clone(),
        backend: run.backend.clone(),
        target: run.target.clone(),
        identity_hash: h,
    };

    // A run with a complete identity roots a tree that carries its hash.
    let with = run_with_identity(s, "a\n");
    assert_eq!(
        s.create_tree(&new_tree(&with, Some("0".repeat(64))), 5)
            .unwrap(),
        TreeOutcome::Rejected(TreeRejection::IdentityMismatch)
    );
    assert_eq!(
        s.create_tree(&new_tree(&with, None), 5).unwrap(),
        TreeOutcome::Rejected(TreeRejection::IdentityMismatch),
        "a tree with point identity cannot be created without it"
    );
    let TreeOutcome::Created(t) = s
        .create_tree(&new_tree(&with, Some(hash_of("a\n"))), 5)
        .unwrap()
    else {
        panic!("not created")
    };
    assert_eq!(
        (t.id.as_str(), t.node_count, t.run_count, t.created_at),
        (with.id.as_str(), 0, 1, 5)
    );
    assert_eq!(t.identity_hash.as_deref(), Some(hash_of("a\n").as_str()));
    assert_eq!(
        s.create_tree(&new_tree(&with, Some(hash_of("a\n"))), 99)
            .unwrap(),
        TreeOutcome::Existing(t.clone()),
        "creating twice returns the stored tree"
    );
    let mut other_target = new_tree(&with, Some(hash_of("a\n")));
    other_target.target = "elsewhere".into();
    assert_eq!(
        s.create_tree(&other_target, 5).unwrap(),
        TreeOutcome::Rejected(TreeRejection::Conflict)
    );
    assert_eq!(s.get_tree(&with.id).unwrap(), Some(t));

    // No identity row, or an incomplete one: only a tree without point identity (NULL hash) can exist.
    let none = new_run(s, "sim");
    assert_eq!(
        s.create_tree(&new_tree(&none, Some(hash_of("a\n"))), 5)
            .unwrap(),
        TreeOutcome::Rejected(TreeRejection::IdentityIncomplete)
    );
    let TreeOutcome::Created(null_tree) = s.create_tree(&new_tree(&none, None), 5).unwrap() else {
        panic!("a tree without point identity is allowed")
    };
    assert_eq!(null_tree.identity_hash, None);

    let partial = new_run(s, "sim");
    s.put_run_identity(&identity(&partial.id, "p\n", false))
        .unwrap();
    assert_eq!(
        s.create_tree(&new_tree(&partial, Some(hash_of("p\n"))), 5)
            .unwrap(),
        TreeOutcome::Rejected(TreeRejection::IdentityIncomplete)
    );
    assert!(matches!(
        s.create_tree(&new_tree(&partial, None), 5).unwrap(),
        TreeOutcome::Created(_)
    ));

    let mut wrong_backend = new_tree(&new_run(s, "sim"), None);
    wrong_backend.backend = "wasm".into();
    assert!(matches!(
        s.create_tree(&wrong_backend, 5).unwrap(),
        TreeOutcome::Rejected(TreeRejection::Invalid(_))
    ));
    let mut missing = new_tree(&none, None);
    missing.run_id = "run_missing".into();
    assert_eq!(
        s.create_tree(&missing, 5).unwrap(),
        TreeOutcome::Rejected(TreeRejection::RunNotFound)
    );
}

fn nodes_get_dense_numbers_depth_and_paths(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(
        s,
        &run.id,
        &[
            TrialVerdict::Pass,
            TrialVerdict::Pass,
            TrialVerdict::Violation,
            TrialVerdict::Inconclusive,
        ],
    );
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    let n0 = add(s, &tree.id, &root(&run.id, 0));
    let n1 = add(s, &tree.id, &child(&run.id, 1, 0, 4));
    let mut walk = child(&run.id, 2, 1, 2);
    walk.operator = Operator::Walk;
    walk.suffix_seed = Some(u64::MAX - 5);
    walk.edge_ref = Some("e".repeat(64));
    walk.origin = NodeOrigin::PointExec;
    walk.reporter_trusted = false;
    let n2 = add(s, &tree.id, &walk);
    let n3 = add(s, &tree.id, &child(&run.id, 3, 0, 5));

    assert_eq!(
        [n0.node, n1.node, n2.node, n3.node],
        [0, 1, 2, 3],
        "node numbers are dense from 0"
    );
    assert_eq!((n0.depth, n1.depth, n2.depth, n3.depth), (0, 1, 2, 1));
    assert_eq!(n0.path, vec![0x00]);
    assert_eq!(n1.path, vec![0x00, 0x01]);
    assert_eq!(n2.path, vec![0x00, 0x01, 0x02]);
    assert_eq!(n3.path, vec![0x00, 0x03]);
    assert_eq!(
        (n0.parent_node, n1.parent_node, n2.parent_node),
        (None, Some(0), Some(1))
    );
    assert_eq!(n2.verdict, TrialVerdict::Violation, "copied from the trial");
    assert_eq!(n3.verdict, TrialVerdict::Inconclusive);
    assert_eq!(
        (n2.operator, n2.cut_kind, n2.cut_at, n2.suffix_seed),
        (Operator::Walk, CutKind::Step, 2, Some(u64::MAX - 5))
    );
    assert_eq!(n2.edge_ref.as_deref(), Some("e".repeat(64).as_str()));
    assert_eq!(
        (n2.origin, n2.reporter_id.as_str(), n2.reporter_trusted),
        (NodeOrigin::PointExec, "w", false)
    );
    assert_eq!((n2.verified, n2.created_at), (Verified::No, 6));
    assert_eq!(n2.tree_id, tree.id);
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().node_count, 4);
    assert_eq!(s.get_tree_node(&tree.id, 2).unwrap(), Some(n2.clone()));
    assert_eq!(s.get_tree_node(&tree.id, 9).unwrap(), None);
    assert_eq!(s.tree_node_of_trial(&run.id, 3).unwrap(), Some(n3));
    assert_eq!(s.tree_node_of_trial(&run.id, 9).unwrap(), None);
    assert_eq!(
        s.tree_of_run(&run.id).unwrap().as_deref(),
        Some(tree.id.as_str())
    );
    assert_eq!(s.tree_of_run("run_none").unwrap(), None);
}

fn shape_rules_are_enforced(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(6));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    let refused = |node: NewNode, want: &str| {
        let out = s.add_tree_node(&tree.id, &node, 7).unwrap();
        match (&out, want) {
            (NodeOutcome::Rejected(TreeRejection::Invalid(_)), "invalid")
            | (NodeOutcome::Rejected(TreeRejection::TrialNotFound), "trial")
            | (NodeOutcome::Rejected(TreeRejection::TrialAlreadyInTree), "twice")
            | (NodeOutcome::Rejected(TreeRejection::RunNotInTree), "run") => {}
            _ => panic!("expected {want}, got {out:?}"),
        }
        assert_eq!(
            s.get_tree(&tree.id).unwrap().unwrap().node_count,
            1,
            "a refused node changes nothing"
        );
    };
    let mut r = root(&run.id, 1);
    r.operator = Operator::Explore;
    refused(r, "invalid");
    let mut r = root(&run.id, 1);
    r.cut_at = 3;
    refused(r, "invalid");
    let mut r = root(&run.id, 1);
    r.suffix_seed = Some(1);
    refused(r, "invalid");
    let mut c = child(&run.id, 1, 0, 1);
    c.operator = Operator::Root;
    refused(c, "invalid");
    let mut c = child(&run.id, 1, 0, 1);
    c.cut_kind = CutKind::None;
    refused(c, "invalid");
    let mut c = child(&run.id, 1, 0, 1);
    c.reporter_id = String::new();
    refused(c, "invalid");
    refused(child(&run.id, 99, 0, 1), "trial");
    refused(root(&run.id, 0), "twice");
    let stranger = new_run(s, "sim");
    ingest_verdicts(s, &stranger.id, &passes(2));
    refused(root(&stranger.id, 0), "run");

    assert_eq!(
        rejected(s.add_tree_node("run_nope", &root(&run.id, 1), 7)),
        NodeOutcome::Rejected(TreeRejection::TreeNotFound)
    );
}

fn parents_must_exist_in_the_same_tree(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(4));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    // The parent must already exist: numbers only grow, so a parent can never be a later node.
    for parent in [1, 2, 7, u64::MAX] {
        assert_eq!(
            s.add_tree_node(&tree.id, &child(&run.id, 1, parent, 0), 7)
                .unwrap(),
            NodeOutcome::Rejected(TreeRejection::ParentNotFound),
            "parent {parent}"
        );
    }
    // A node number of another tree is not a parent here.
    let other_run = run_with_identity(s, "b\n");
    ingest_verdicts(s, &other_run.id, &passes(3));
    let other = make_tree(s, &other_run, Some(sha256_hex(b"b\n")));
    add(s, &other.id, &root(&other_run.id, 0));
    add(s, &other.id, &child(&other_run.id, 1, 0, 0));
    assert_eq!(
        s.add_tree_node(&tree.id, &child(&run.id, 1, 1, 0), 7)
            .unwrap(),
        NodeOutcome::Rejected(TreeRejection::ParentNotFound)
    );
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().node_count, 1);
}

fn depth_and_fanout_caps(s: &dyn Store) {
    // Depth: a chain of MAX_TREE_DEPTH children below the root is the deepest allowed.
    let depth = MAX_TREE_DEPTH as u64;
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(depth as usize + 3));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    for i in 1..=depth {
        let n = add(s, &tree.id, &child(&run.id, i, i - 1, 0));
        assert_eq!(n.depth as u64, i);
    }
    assert_eq!(
        s.add_tree_node(&tree.id, &child(&run.id, depth + 1, depth, 0), 7)
            .unwrap(),
        NodeOutcome::Rejected(TreeRejection::DepthCap)
    );
    let deepest = s.get_tree_node(&tree.id, depth).unwrap().unwrap();
    assert_eq!(deepest.depth, MAX_TREE_DEPTH);
    assert!(deepest.path.len() <= 2 * MAX_TREE_DEPTH as usize + 3);
    // The whole chain is still readable, in both forms.
    assert_eq!(
        s.tree_ancestors(&tree.id, depth).unwrap().len() as u64,
        depth
    );
    in_each_form(s, &mut |rule| {
        let first = s.subtree_descendants(&tree.id, 0, None, 500).unwrap();
        assert_eq!(
            (first.nodes.len(), first.next_cursor),
            (500, Some(500)),
            "{rule:?}"
        );
        let rest = s
            .subtree_descendants(&tree.id, 0, first.next_cursor, 500)
            .unwrap();
        assert_eq!(
            (rest.nodes.len() as u64, rest.next_cursor),
            (depth - 500, None),
            "{rule:?}"
        );
        let counted: u64 = s
            .subtree_tally(&tree.id, 0)
            .unwrap()
            .iter()
            .map(|v| v.count)
            .sum();
        assert_eq!(counted, depth + 1, "{rule:?}");
    });

    // Fan-out: MAX_FANOUT children per (parent, cut); another cut of the same parent has its own budget.
    let fan = MAX_FANOUT;
    let run = run_with_identity(s, "f\n");
    ingest_verdicts(s, &run.id, &passes(fan as usize + 3));
    let tree = make_tree(s, &run, Some(sha256_hex(b"f\n")));
    add(s, &tree.id, &root(&run.id, 0));
    for i in 1..=fan {
        add(s, &tree.id, &child(&run.id, i, 0, 5));
    }
    assert_eq!(
        s.add_tree_node(&tree.id, &child(&run.id, fan + 1, 0, 5), 7)
            .unwrap(),
        NodeOutcome::Rejected(TreeRejection::FanoutCap)
    );
    assert!(matches!(
        s.add_tree_node(&tree.id, &child(&run.id, fan + 1, 0, 6), 7)
            .unwrap(),
        NodeOutcome::Added(_)
    ));
}

/// The node cap needs a tree that is full; `set_node_count` writes the counter directly (the harness has raw access).
pub fn node_cap_is_enforced(s: &dyn Store, set_node_count: &dyn Fn(&str, u64)) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(3));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    set_node_count(&tree.id, MAX_TREE_NODES - 1);
    assert!(matches!(
        s.add_tree_node(&tree.id, &child(&run.id, 1, 0, 0), 7)
            .unwrap(),
        NodeOutcome::Added(n) if n.node == MAX_TREE_NODES - 1
    ));
    assert_eq!(
        s.add_tree_node(&tree.id, &child(&run.id, 2, 0, 0), 7)
            .unwrap(),
        NodeOutcome::Rejected(TreeRejection::NodeCap)
    );
}

fn a_tree_without_point_identity_reads_but_takes_no_branches(s: &dyn Store) {
    let run = new_run(s, "sim");
    ingest_verdicts(
        s,
        &run.id,
        &[
            TrialVerdict::Pass,
            TrialVerdict::Violation,
            TrialVerdict::Pass,
        ],
    );
    let tree = make_tree(s, &run, None);
    assert_eq!(tree.identity_hash, None);
    add(s, &tree.id, &root(&run.id, 0));
    add(s, &tree.id, &child(&run.id, 1, 0, 1));
    add(s, &tree.id, &child(&run.id, 2, 0, 2));
    // Every read works, in both forms.
    in_each_form(s, &mut |_| {
        assert_eq!(
            s.subtree_descendants(&tree.id, 0, None, 10)
                .unwrap()
                .nodes
                .len(),
            2
        );
        assert_eq!(
            s.tree_children(&tree.id, 0, None, 10).unwrap().nodes.len(),
            2
        );
        assert_eq!(s.tree_ancestors(&tree.id, 2).unwrap().len(), 1);
        assert_eq!(s.subtree_first_failing(&tree.id, 0).unwrap(), Some(1));
        assert_eq!(s.subtree_tally(&tree.id, 0).unwrap().len(), 2);
    });
    // A branch run cannot attach, even one with a complete identity: branches need the tree's point identity.
    let branch = run_with_identity(s, "a\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &branch.id, 0, &"c".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::IdentityIncomplete)
    );
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().run_count, 1);
}

fn branches_are_checked_and_deduplicated(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(3));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    add(s, &tree.id, &child(&run.id, 1, 0, 1));
    finish(s, &run.id);
    let hash = "c".repeat(64);

    let b1 = run_with_identity(s, "a\n");
    let BranchOutcome::Added(rec) = s
        .add_tree_branch(&branch_for(&tree.id, &b1.id, 1, &hash), 9)
        .unwrap()
    else {
        panic!("branch not added")
    };
    assert_eq!(
        (rec.run_id.as_str(), rec.tree_id.as_str(), rec.source_node),
        (b1.id.as_str(), tree.id.as_str(), 1)
    );
    assert_eq!(
        (rec.cut_kind, rec.cut_at, rec.seed_source, rec.created_at),
        (CutKind::Step, 3, SeedSource::ServerRandom, 9)
    );
    assert_eq!((rec.verify_runs, rec.verify_matches), (0, 0));
    assert_eq!(rec.created_by, "key_ab12");
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().run_count, 2);
    assert_eq!(s.get_tree_branch(&b1.id).unwrap(), Some(rec.clone()));
    assert_eq!(
        s.find_tree_branch(&tree.id, &hash).unwrap(),
        Some(rec.clone())
    );
    assert_eq!(
        s.tree_of_run(&b1.id).unwrap().as_deref(),
        Some(tree.id.as_str())
    );

    // The same command again is a cache hit: the stored branch comes back and nothing is added.
    let b2 = run_with_identity(s, "a\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &b2.id, 1, &hash), 10)
            .unwrap(),
        BranchOutcome::Existing(rec.clone())
    );
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().run_count, 2);
    assert_eq!(s.get_tree_branch(&b2.id).unwrap(), None);

    // Every run in a tree has exactly the tree's identity.
    let other_hash = "d".repeat(64);
    let different = run_with_identity(s, "b\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &different.id, 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::IdentityMismatch)
    );
    let bare = new_run(s, "sim");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &bare.id, 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::IdentityIncomplete)
    );
    let partial = new_run(s, "sim");
    s.put_run_identity(&identity(&partial.id, "a\n", false))
        .unwrap();
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &partial.id, 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::IdentityIncomplete)
    );
    let good = run_with_identity(s, "a\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &good.id, 77, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::SourceNotFound)
    );
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &run.id, 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::Invalid("the root run cannot be a branch")),
    );
    assert_eq!(
        s.add_tree_branch(&branch_for("run_nope", &good.id, 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::TreeNotFound)
    );
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, "run_nope", 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::RunNotFound)
    );
    // The same run cannot be attached twice under another command.
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &b1.id, 1, &other_hash), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::Conflict)
    );
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().run_count, 2);

    // A branch run's own trials become nodes of the tree, below the source node.
    ingest_verdicts(s, &b1.id, &passes(2));
    let n = add(s, &tree.id, &child(&b1.id, 0, 1, 3));
    assert_eq!((n.node, n.depth, n.parent_node), (2, 2, Some(1)));

    s.record_branch_verify(&b1.id, true).unwrap();
    s.record_branch_verify(&b1.id, false).unwrap();
    s.record_branch_verify(&b1.id, true).unwrap();
    let after = s.get_tree_branch(&b1.id).unwrap().unwrap();
    assert_eq!((after.verify_runs, after.verify_matches), (3, 2));
    assert!(s.record_branch_verify("run_none", true).is_err());
}

/// A finished run never starts a new attempt, so a branch may attach only below nodes that cannot be deleted: both the
/// tree's root run and the run of the source node must have finished.
fn branches_attach_only_below_finished_runs(s: &dyn Store) {
    let root_run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &root_run.id, &passes(2));
    let tree = make_tree(s, &root_run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&root_run.id, 0));
    add(s, &tree.id, &child(&root_run.id, 1, 0, 1));

    // The root run is still running.
    let b1 = run_with_identity(s, "a\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &b1.id, 1, &"1".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::RunNotFinished)
    );
    assert_eq!(s.get_tree_branch(&b1.id).unwrap(), None);
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().run_count, 1);
    finish(s, &root_run.id);
    let BranchOutcome::Added(_) = s
        .add_tree_branch(&branch_for(&tree.id, &b1.id, 1, &"1".repeat(64)), 9)
        .unwrap()
    else {
        panic!("a branch below finished runs is accepted")
    };

    // The source node belongs to the first branch run, which is still running.
    ingest_verdicts(s, &b1.id, &passes(1));
    let n = add(s, &tree.id, &child(&b1.id, 0, 1, 3));
    let b2 = run_with_identity(s, "a\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &b2.id, n.node, &"2".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::RunNotFinished)
    );
    finish(s, &b1.id);
    assert!(matches!(
        s.add_tree_branch(&branch_for(&tree.id, &b2.id, n.node, &"2".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Added(_)
    ));
    // A finished root run cannot start another attempt, so its nodes stay.
    let job = s.enqueue_job(&root_run.id, "run", "{}", 2).unwrap();
    s.lease_job("w9", 10, 1_000).unwrap();
    let out = s
        .ingest_trials(&job.id, "w9", true, &batch(&passes(1), 0), 1_100)
        .unwrap();
    assert!(!matches!(out, IngestOutcome::Applied { .. }), "{out:?}");
    assert!(s.get_tree_node(&tree.id, 1).unwrap().is_some());
}

/// A run is a tree's root or a branch of one tree, not both.
fn a_run_cannot_be_a_root_and_a_branch(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(1));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    finish(s, &run.id);

    // A branch run cannot become the root of a tree.
    let b = run_with_identity(s, "a\n");
    assert!(matches!(
        s.add_tree_branch(&branch_for(&tree.id, &b.id, 0, &"1".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Added(_)
    ));
    assert_eq!(
        s.create_tree(
            &NewTree {
                run_id: b.id.clone(),
                backend: b.backend.clone(),
                target: b.target.clone(),
                identity_hash: Some(sha256_hex(b"a\n")),
            },
            5
        )
        .unwrap(),
        TreeOutcome::Rejected(TreeRejection::Invalid("a branch run cannot root a tree"))
    );
    // A run that roots a tree cannot become a branch of another.
    let other = run_with_identity(s, "a\n");
    ingest_verdicts(s, &other.id, &passes(1));
    let other_tree = make_tree(s, &other, Some(sha256_hex(b"a\n")));
    add(s, &other_tree.id, &root(&other.id, 0));
    finish(s, &other.id);
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &other.id, 0, &"2".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::Invalid(
            "a run that roots a tree cannot be a branch"
        ))
    );
}

fn the_branch_cap_holds(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(1));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    finish(s, &run.id);
    for i in 0..MAX_BRANCH_RUNS {
        let b = run_with_identity(s, "a\n");
        let out = s
            .add_tree_branch(&branch_for(&tree.id, &b.id, 0, &format!("{i:064x}")), 9)
            .unwrap();
        assert!(
            matches!(out, BranchOutcome::Added(_)),
            "branch {i}: {out:?}"
        );
    }
    let one_more = run_with_identity(s, "a\n");
    assert_eq!(
        s.add_tree_branch(&branch_for(&tree.id, &one_more.id, 0, &"f".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Rejected(TreeRejection::BranchCap)
    );
    // A repeat of a stored command is still answered at the cap.
    let repeat = s
        .add_tree_branch(
            &branch_for(&tree.id, &one_more.id, 0, &format!("{:064x}", 3)),
            9,
        )
        .unwrap();
    assert!(matches!(repeat, BranchOutcome::Existing(_)), "{repeat:?}");
    assert_eq!(
        s.get_tree(&tree.id).unwrap().unwrap().run_count,
        MAX_BRANCH_RUNS + 1
    );
}

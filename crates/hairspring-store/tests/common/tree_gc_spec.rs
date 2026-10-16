//! Specification of the retention command that does not exist yet (`hairspring gc`, RX D6). Nothing deletes runs or
//! trials today, and this change does not add it. The scenarios here state what it must do with trial trees, so the
//! rule is written down and runs the day a deletion routine exists: wire `gc` to it and remove the `#[ignore]`.
//!
//! A tree is one unit. It is kept while any of its runs is kept, and deleted in one transaction otherwise, in the
//! order lineage, branches, trees, runs, with a report. No node may be left pointing at a deleted parent.
#![allow(dead_code)]
use super::tree_support::*;
use hairspring_store::*;

/// What the routine reports.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub trees_deleted: Vec<String>,
    pub trees_kept: Vec<String>,
    pub runs_deleted: u64,
    pub nodes_deleted: u64,
}

/// The routine under specification: delete the given expired runs, honouring the tree rule.
pub type Gc<'a> = &'a dyn Fn(&dyn Store, &[String]) -> GcReport;

/// Stand-in until the routine exists.
pub fn gc_is_not_built(_: &dyn Store, _: &[String]) -> GcReport {
    unimplemented!("hairspring gc (RX D6) is not built; this test is its specification")
}

struct Fixture {
    tree: String,
    root: String,
    branches: [String; 2],
    other: String,
}

fn build(s: &dyn Store) -> Fixture {
    let root_run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &root_run.id, &passes(3));
    let tree = make_tree(s, &root_run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&root_run.id, 0));
    add(s, &tree.id, &child(&root_run.id, 1, 0, 1));
    add(s, &tree.id, &child(&root_run.id, 2, 1, 1));
    finish(s, &root_run.id);
    let mut branches = Vec::new();
    for (i, source) in [(0u64, 2u64), (1, 1)] {
        let b = run_with_identity(s, "a\n");
        ingest_verdicts(s, &b.id, &passes(2));
        s.add_tree_branch(
            &branch_for(&tree.id, &b.id, source, &format!("{i:064x}")),
            9,
        )
        .unwrap();
        let first = add(s, &tree.id, &child(&b.id, 0, source, 3));
        add(s, &tree.id, &child(&b.id, 1, first.node, 4));
        branches.push(b.id);
    }
    let other = new_run(s, "sim");
    ingest_verdicts(s, &other.id, &passes(2));
    Fixture {
        tree: tree.id,
        root: root_run.id,
        branches: [branches[0].clone(), branches[1].clone()],
        other: other.id,
    }
}

/// No node of the tree names a parent that is gone.
fn assert_no_dangling_parents(s: &dyn Store, tree: &str, node_count: u64) {
    for node in 0..node_count {
        if let Some(n) = s.get_tree_node(tree, node).unwrap() {
            if let Some(p) = n.parent_node {
                assert!(s.get_tree_node(tree, p).unwrap().is_some(), "node {node}");
            }
        }
    }
}

pub fn a_tree_is_kept_while_any_of_its_runs_is_kept(s: &dyn Store, gc: Gc) {
    let f = build(s);
    let nodes = s.get_tree(&f.tree).unwrap().unwrap().node_count;
    // Only one branch run is still within retention.
    let expired = vec![f.root.clone(), f.branches[1].clone(), f.other.clone()];
    let report = gc(s, &expired);
    assert_eq!(report.trees_kept, vec![f.tree.clone()]);
    assert!(report.trees_deleted.is_empty());
    assert!(s.get_tree(&f.tree).unwrap().is_some());
    for run in [&f.root, &f.branches[0], &f.branches[1]] {
        assert!(
            s.get_run(run).unwrap().is_some(),
            "{run} stays with its tree"
        );
    }
    assert_no_dangling_parents(s, &f.tree, nodes);
    assert!(
        s.get_run(&f.other).unwrap().is_none(),
        "a run outside any tree is deleted as usual"
    );
}

pub fn a_tree_is_deleted_in_one_unit_when_every_run_has_expired(s: &dyn Store, gc: Gc) {
    let f = build(s);
    let tree = s.get_tree(&f.tree).unwrap().unwrap();
    let expired = vec![
        f.root.clone(),
        f.branches[0].clone(),
        f.branches[1].clone(),
        f.other.clone(),
    ];
    let report = gc(s, &expired);
    assert_eq!(report.trees_deleted, vec![f.tree.clone()]);
    assert!(report.trees_kept.is_empty());
    assert_eq!(report.runs_deleted, 4);
    assert_eq!(report.nodes_deleted, tree.node_count);
    assert_eq!(s.get_tree(&f.tree).unwrap(), None);
    assert_eq!(s.get_tree_node(&f.tree, 0).unwrap(), None);
    assert_eq!(s.get_tree_branch(&f.branches[0]).unwrap(), None);
    for run in &expired {
        assert!(s.get_run(run).unwrap().is_none(), "{run}");
        assert_eq!(s.count_trials(run).unwrap(), 0, "{run}");
    }
}

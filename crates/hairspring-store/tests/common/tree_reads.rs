//! Conformance scenarios for tree reads: paging with the one cursor rule, page limits, path boundaries and byte order.
//! Every subtree read runs in both of its forms.
#![allow(dead_code)]
use super::tree_support::*;
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_reads(make: &dyn Fn() -> Arc<dyn Store>) {
    descendant_pages_follow_the_one_cursor_rule(&*make());
    page_size_is_clamped(&*make());
    succ_of_a_last_byte_of_0x7f_does_not_leak_the_next_root(&*make());
    byte_order_is_not_node_order(&*make());
    verification_state_is_the_only_mutable_column(&*make());
}

fn descendant_pages_follow_the_one_cursor_rule(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(6));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    for i in 1..=5 {
        add(s, &tree.id, &child(&run.id, i, 0, 0));
    }
    let nodes = |p: &NodePage| p.nodes.iter().map(|n| n.node).collect::<Vec<_>>();

    in_each_form(s, &mut |rule| {
        // The first cursor is the node itself. A page that fills returns its last node as the cursor ...
        let p1 = s.subtree_descendants(&tree.id, 0, None, 2).unwrap();
        assert_eq!(
            (nodes(&p1), p1.next_cursor),
            (vec![1, 2], Some(2)),
            "{rule:?}"
        );
        let p2 = s
            .subtree_descendants(&tree.id, 0, p1.next_cursor, 2)
            .unwrap();
        assert_eq!(
            (nodes(&p2), p2.next_cursor),
            (vec![3, 4], Some(4)),
            "{rule:?}"
        );
        // ... and the list is finished once no later node can exist.
        let p3 = s
            .subtree_descendants(&tree.id, 0, p2.next_cursor, 2)
            .unwrap();
        assert_eq!((nodes(&p3), p3.next_cursor), (vec![5], None), "{rule:?}");
        let whole = s.subtree_descendants(&tree.id, 0, None, 5).unwrap();
        assert_eq!(
            (nodes(&whole), whole.next_cursor),
            (vec![1, 2, 3, 4, 5], None),
            "a page that fills on the last node is the last page ({rule:?})"
        );
        // A cursor below the node is raised to it; one past the end gives an empty, finished page.
        assert_eq!(
            nodes(&s.subtree_descendants(&tree.id, 3, Some(0), 10).unwrap()),
            Vec::<u64>::new()
        );
        let past = s.subtree_descendants(&tree.id, 0, Some(5), 2).unwrap();
        assert_eq!((nodes(&past), past.next_cursor), (vec![], None), "{rule:?}");
        // Unknown nodes are an error, not an empty answer.
        assert!(s.subtree_descendants(&tree.id, 99, None, 2).is_err());
        assert!(s.subtree_tally(&tree.id, 99).is_err());
        assert!(s.subtree_first_failing(&tree.id, 99).is_err());
    });
    // Children are keyset-paged with the same cursor.
    let c1 = s.tree_children(&tree.id, 0, None, 2).unwrap();
    assert_eq!((nodes(&c1), c1.next_cursor), (vec![1, 2], Some(2)));
    let c3 = s.tree_children(&tree.id, 0, Some(4), 2).unwrap();
    assert_eq!((nodes(&c3), c3.next_cursor), (vec![5], None));
    let c_all = s.tree_children(&tree.id, 0, None, 5).unwrap();
    assert_eq!(
        (nodes(&c_all), c_all.next_cursor),
        (vec![1, 2, 3, 4, 5], None)
    );
    assert!(s.tree_children(&tree.id, 99, None, 2).is_err());
    assert!(s.tree_ancestors(&tree.id, 99).is_err());
}

fn page_size_is_clamped(s: &dyn Store) {
    let n = MAX_PAGE_LIMIT as u64 + 100;
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(n as usize + 1));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    for i in 1..=n {
        add(s, &tree.id, &child(&run.id, i, 0, i % 2));
    }
    in_each_form(s, &mut |rule| {
        let big = s.subtree_descendants(&tree.id, 0, None, 100_000).unwrap();
        assert_eq!(big.nodes.len(), MAX_PAGE_LIMIT as usize, "{rule:?}");
        assert_eq!(big.next_cursor, Some(MAX_PAGE_LIMIT as u64), "{rule:?}");
        let zero = s.subtree_descendants(&tree.id, 0, None, 0).unwrap();
        assert_eq!(
            zero.nodes.len(),
            1,
            "a limit of 0 is raised to 1 ({rule:?})"
        );
    });
    let big = s.tree_children(&tree.id, 0, None, 100_000).unwrap();
    assert_eq!(big.nodes.len(), MAX_PAGE_LIMIT as usize);
    assert_eq!(big.next_cursor, Some(MAX_PAGE_LIMIT as u64));
    let zero = s.tree_children(&tree.id, 0, None, 0).unwrap();
    assert_eq!(zero.nodes.len(), 1, "a limit of 0 is raised to 1");
}

fn succ_of_a_last_byte_of_0x7f_does_not_leak_the_next_root(s: &dyn Store) {
    // Nodes 0..=127 are roots; node 127 has the one-byte path 0x7f, so its range ends at 0x80. Node 128 is the
    // next root (path 80 01) and must stay outside the range; node 129 is a child of 127.
    let run = run_with_identity(s, "a\n");
    let mut verdicts = passes(130);
    verdicts[129] = TrialVerdict::Violation;
    ingest_verdicts(s, &run.id, &verdicts);
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    for i in 0..=128 {
        add(s, &tree.id, &root(&run.id, i));
    }
    let kid = add(s, &tree.id, &child(&run.id, 129, 127, 0));
    assert_eq!(
        s.get_tree_node(&tree.id, 127).unwrap().unwrap().path,
        vec![0x7f]
    );
    assert_eq!(
        s.get_tree_node(&tree.id, 128).unwrap().unwrap().path,
        vec![0x80, 0x01]
    );
    assert_eq!(kid.path, vec![0x7f, 0x81, 0x01]);

    in_each_form(s, &mut |rule| {
        let page = s.subtree_descendants(&tree.id, 127, None, 500).unwrap();
        assert_eq!(
            page.nodes.iter().map(|n| n.node).collect::<Vec<_>>(),
            vec![129],
            "{rule:?}"
        );
        let tally: Vec<(TrialVerdict, u64)> = s
            .subtree_tally(&tree.id, 127)
            .unwrap()
            .into_iter()
            .map(|v| (v.verdict, v.count))
            .collect();
        assert_eq!(
            tally.iter().map(|(_, c)| c).sum::<u64>(),
            2,
            "node 127 and its child, not node 128 ({rule:?}): {tally:?}"
        );
        assert_eq!(
            s.subtree_first_failing(&tree.id, 127).unwrap(),
            Some(129),
            "{rule:?}"
        );
        assert_eq!(
            s.subtree_first_failing(&tree.id, 128).unwrap(),
            None,
            "{rule:?}"
        );
        let none = s.subtree_descendants(&tree.id, 128, None, 500).unwrap();
        assert!(none.nodes.is_empty(), "{rule:?}");
    });
}

fn byte_order_is_not_node_order(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(300));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    for i in 1..300 {
        add(s, &tree.id, &child(&run.id, i, 0, i % 2));
    }
    let p256 = s.get_tree_node(&tree.id, 256).unwrap().unwrap().path;
    let p130 = s.get_tree_node(&tree.id, 130).unwrap().unwrap().path;
    assert_eq!(p256, vec![0x00, 0x80, 0x02]);
    assert_eq!(p130, vec![0x00, 0x82, 0x01]);
    assert!(p256 < p130, "node 256 sorts before node 130 by path");
    // Lists stay in node order whatever the byte order is.
    in_each_form(s, &mut |rule| {
        let mut listed = Vec::new();
        let mut cursor = None;
        loop {
            let page = s.subtree_descendants(&tree.id, 0, cursor, 100).unwrap();
            listed.extend(page.nodes.iter().map(|n| n.node));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(listed, (1..300).collect::<Vec<u64>>(), "{rule:?}");
    });
    let mut kids = Vec::new();
    let mut cursor = None;
    loop {
        let page = s.tree_children(&tree.id, 0, cursor, 100).unwrap();
        kids.extend(page.nodes.iter().map(|n| n.node));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(kids, (1..300).collect::<Vec<u64>>());
}

fn verification_state_is_the_only_mutable_column(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(2));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    let before = add(s, &tree.id, &child(&run.id, 1, 0, 1));
    s.set_node_verified(&tree.id, 1, Verified::ReplayMismatch)
        .unwrap();
    let after = s.get_tree_node(&tree.id, 1).unwrap().unwrap();
    assert_eq!(after.verified, Verified::ReplayMismatch);
    assert_eq!(
        NodeRecord {
            verified: Verified::No,
            ..after
        },
        before
    );
    s.set_node_verified(&tree.id, 1, Verified::ReplayOk)
        .unwrap();
    assert_eq!(
        s.get_tree_node(&tree.id, 1).unwrap().unwrap().verified,
        Verified::ReplayOk
    );
    assert!(s
        .set_node_verified(&tree.id, 9, Verified::ReplayOk)
        .is_err());
}

//! Conformance scenarios for the content of the tree edge and command fields: hash encodings, the command's JSON and
//! its nesting bound. A malformed value is refused with a typed error and nothing is written.
#![allow(dead_code)]
use super::tree_support::*;
use hairspring_store::*;
use std::sync::Arc;

pub fn run_all_fields(make: &dyn Fn() -> Arc<dyn Store>) {
    edge_references_are_sha256_hex(&*make());
    branch_hashes_are_sha256_hex(&*make());
    commands_are_bounded_json(&*make());
}

fn invalid<T: std::fmt::Debug>(outcome: T, what: &str) {
    let text = format!("{outcome:?}");
    assert!(text.contains("Rejected(Invalid("), "{what}: {text}");
}

/// Hashes the encoding rule must refuse: wrong length, wrong alphabet (upper case included), padding and text a
/// query could be built from.
fn malformed_hashes() -> Vec<String> {
    vec![
        String::new(),
        "e".repeat(63),
        "e".repeat(65),
        "E".repeat(64),
        "g".repeat(64),
        format!("{}\u{0}", "e".repeat(63)),
        format!(" {}", "e".repeat(63)),
        format!("{}\n", "e".repeat(63)),
        format!("{}'", "e".repeat(63)),
        "é".repeat(32),
    ]
}

fn edge_references_are_sha256_hex(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(4));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    for (i, bad) in malformed_hashes().into_iter().enumerate() {
        let mut node = child(&run.id, 1, 0, 1);
        node.edge_ref = Some(bad);
        invalid(
            s.add_tree_node(&tree.id, &node, 6).unwrap(),
            &format!("edge reference {i}"),
        );
    }
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().node_count, 1);

    let mut node = child(&run.id, 1, 0, 1);
    node.edge_ref = Some(sha256_hex(b"an edge"));
    assert!(matches!(
        s.add_tree_node(&tree.id, &node, 6).unwrap(),
        NodeOutcome::Added(_)
    ));
}

fn branch_hashes_are_sha256_hex(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(2));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    finish(s, &run.id);
    let b = run_with_identity(s, "a\n");
    for (i, bad) in malformed_hashes().into_iter().enumerate() {
        let mut point = branch_for(&tree.id, &b.id, 0, &"1".repeat(64));
        point.point_hash = bad.clone();
        invalid(
            s.add_tree_branch(&point, 9).unwrap(),
            &format!("point hash {i}"),
        );
        invalid(
            s.add_tree_branch(&branch_for(&tree.id, &b.id, 0, &bad), 9)
                .unwrap(),
            &format!("command hash {i}"),
        );
    }
    assert_eq!(s.get_tree_branch(&b.id).unwrap(), None);
    assert!(matches!(
        s.add_tree_branch(&branch_for(&tree.id, &b.id, 0, &"1".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Added(_)
    ));
}

fn nested(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

fn commands_are_bounded_json(s: &dyn Store) {
    let run = run_with_identity(s, "a\n");
    ingest_verdicts(s, &run.id, &passes(2));
    let tree = make_tree(s, &run, Some(sha256_hex(b"a\n")));
    add(s, &tree.id, &root(&run.id, 0));
    finish(s, &run.id);
    let b = run_with_identity(s, "a\n");
    let try_command = |text: &str| {
        let mut br = branch_for(&tree.id, &b.id, 0, &"1".repeat(64));
        br.command_json = text.into();
        s.add_tree_branch(&br, 9).unwrap()
    };

    let too_deep = nested(MAX_COMMAND_JSON_DEPTH + 1);
    let truncated = r#"{"verb":"step""#;
    let trailing = r#"{"verb":"step"} {}"#;
    for (what, text) in [
        ("empty", ""),
        ("plain text", "step"),
        ("truncated", truncated),
        ("trailing data", trailing),
        ("too deep", too_deep.as_str()),
        ("single quotes", "{'verb':'step'}"),
        ("an unescaped control character", "{\"v\":\"a\nb\"}"),
    ] {
        invalid(try_command(text), what);
    }
    assert_eq!(s.get_tree_branch(&b.id).unwrap(), None);

    // Brackets inside strings are not nesting, and the deepest allowed command is accepted.
    let in_strings = r#"{"v":"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[["}"#;
    let deepest = nested(MAX_COMMAND_JSON_DEPTH);
    assert!(matches!(
        try_command(in_strings),
        BranchOutcome::Added(_) | BranchOutcome::Existing(_)
    ));
    assert_eq!(
        s.get_tree_branch(&b.id).unwrap().unwrap().command_json,
        in_strings
    );

    let c = run_with_identity(s, "a\n");
    let mut br = branch_for(&tree.id, &c.id, 0, &"2".repeat(64));
    br.command_json = deepest.clone();
    assert!(matches!(
        s.add_tree_branch(&br, 9).unwrap(),
        BranchOutcome::Added(_)
    ));
}

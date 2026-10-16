//! Trial tree queries on PostgreSQL: caps, the independent recursive oracle, a 100k-node tree in both forms, the
//! per-store rule and the query plans of every form. Skipped unless `HAIRSPRING_TEST_PG_URL` is set.
#![cfg(feature = "postgres")]
mod common;
use common::pg_support::*;
use common::tree_oracle::*;
use common::tree_support::*;
use hairspring_store::*;
use postgres::Client;

#[test]
fn the_node_cap_is_enforced() {
    let Some(url) = url() else { return };
    let schema = new_schema("q");
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let c = std::cell::RefCell::new(raw(&url, &schema));
    common::tree::node_cap_is_enforced(&s, &|tree, n| {
        c.borrow_mut()
            .execute(
                "UPDATE trees SET node_count = $1 WHERE id = $2",
                &[&(n as i64), &tree],
            )
            .unwrap();
    });
}

/// The independent oracle: a plain `WITH RECURSIVE` over `parent_node`, written for the harness and not shared with
/// the store.
fn recursive_answer(c: &mut Client, tree: &str, a: u64) -> CteAnswer {
    let subtree = c
        .query(
            "WITH RECURSIVE sub(node) AS (
               SELECT $2::bigint UNION ALL
               SELECT l.node FROM sub s JOIN trial_lineage l ON l.parent_node = s.node WHERE l.tree_id = $1)
             SELECT l.node, l.verdict, l.depth FROM sub JOIN trial_lineage l ON l.node = sub.node
              WHERE l.tree_id = $1 ORDER BY l.node",
            &[&tree, &(a as i64)],
        )
        .unwrap()
        .iter()
        .map(|r| {
            (
                r.get::<_, i64>(0) as u64,
                r.get::<_, String>(1),
                r.get::<_, i32>(2) as u32,
            )
        })
        .collect();
    let ancestors = c
        .query(
            "WITH RECURSIVE up(node, parent) AS (
               SELECT node, parent_node FROM trial_lineage WHERE tree_id = $1 AND node = $2::bigint UNION ALL
               SELECT l.node, l.parent_node FROM up JOIN trial_lineage l ON l.node = up.parent WHERE l.tree_id = $1)
             SELECT up.node FROM up JOIN trial_lineage l ON l.node = up.node AND l.tree_id = $1
              WHERE up.node <> $2 ORDER BY l.depth DESC",
            &[&tree, &(a as i64)],
        )
        .unwrap()
        .iter()
        .map(|r| r.get::<_, i64>(0) as u64)
        .collect();
    CteAnswer { subtree, ancestors }
}

#[test]
fn subtree_queries_match_a_recursive_query_on_random_trees_in_both_forms() {
    let Some(url) = url() else { return };
    for (seed, n, shape) in ORACLE_CASES {
        let schema = new_schema(&format!("o{seed}"));
        let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
        let spec = generate(*seed, *n, *shape);
        let started = std::time::Instant::now();
        let tree = load(&s, &spec);
        let loaded = started.elapsed();
        let c = std::cell::RefCell::new(raw(&url, &schema));
        let cte = |t: &str, a: u64| recursive_answer(&mut c.borrow_mut(), t, a);
        check_against_oracles(&s, &tree, &spec, &cte, *seed);
        eprintln!(
            "oracle case seed {seed}: {n} nodes ({shape:?}), max depth {}, loaded in {loaded:?}, checked in {:?}",
            spec.depths.iter().max().unwrap(),
            started.elapsed() - loaded
        );
    }
}

/// Seeds a big tree with array inserts (the store API would take minutes), then analyses it.
fn seed_big_tree(url: &str, schema: &str, spec: &TreeSpec) -> (PgStore, String) {
    let s = PgStore::connect_in_schema(url, schema, 2).unwrap();
    let run = new_run(&s, "sim");
    let tree = make_tree(&s, &run, None);
    let paths = paths_of(spec);
    let mut c = raw(url, schema);
    let n = spec.parents.len();
    for start in (0..n).step_by(5_000) {
        let end = (start + 5_000).min(n);
        let nodes: Vec<i64> = (start..end).map(|i| i as i64).collect();
        let verdicts: Vec<String> = (start..end)
            .map(|i| spec.verdicts[i].as_str().to_string())
            .collect();
        let parents: Vec<Option<i64>> = (start..end)
            .map(|i| spec.parents[i].map(|p| p as i64))
            .collect();
        let depths: Vec<i32> = (start..end).map(|i| spec.depths[i] as i32).collect();
        let node_paths: Vec<Vec<u8>> = (start..end).map(|i| paths[i].clone()).collect();
        let is_child = |i: usize| spec.parents[i].is_some();
        let ops: Vec<String> = (start..end)
            .map(|i| (if is_child(i) { "explore" } else { "root" }).to_string())
            .collect();
        let cut_kinds: Vec<String> = (start..end)
            .map(|i| (if is_child(i) { "step" } else { "none" }).to_string())
            .collect();
        let cut_ats: Vec<i64> = (start..end)
            .map(|i| if is_child(i) { (i % 8) as i64 } else { 0 })
            .collect();
        c.execute(
            "INSERT INTO trials(run_id,seq,seed,verdict,reason,timed_out,degraded,perturbing,report_missing,rated,
               claimable,syscalls,work_ops,elapsed_ms,worker_id,worker_trusted,attempt,kept,digest)
             SELECT $1, t.n, t.n, t.v, '', false, false, false, false, true, true, 1, 1, 1, 'w', true, 1, 'none', '\\x01000000'::bytea
               FROM unnest($2::bigint[], $3::text[]) AS t(n, v)",
            &[&run.id, &nodes, &verdicts],
        )
        .unwrap();
        c.execute(
            "INSERT INTO trial_lineage(tree_id,node,run_id,seq,parent_node,depth,path,verdict,operator,cut_kind,cut_at,
               origin,reporter_id,reporter_trusted,verified,created_at)
             SELECT $1, t.n, $2, t.n, t.p, t.d, t.path, t.v, t.op, t.ck, t.ca, 'exploration', 'w', true, 'no', 1
               FROM unnest($3::bigint[], $4::bigint[], $5::int[], $6::bytea[], $7::text[], $8::text[], $9::text[], $10::bigint[])
                    AS t(n, p, d, path, v, op, ck, ca)",
            &[&tree.id, &run.id, &nodes, &parents, &depths, &node_paths, &verdicts, &ops, &cut_kinds, &cut_ats],
        )
        .unwrap();
    }
    c.execute(
        "UPDATE trees SET node_count = $1 WHERE id = $2",
        &[&(n as i64), &tree.id],
    )
    .unwrap();
    c.batch_execute("ANALYZE trial_lineage; ANALYZE trials")
        .unwrap();
    (s, tree.id)
}

const BIG_NODES: usize = 100_000;

#[test]
fn a_hundred_thousand_node_tree_is_paged_in_both_forms_and_children_cross_big_gaps() {
    let Some(url) = url() else { return };
    let spec = big_tree(BIG_NODES, 99);
    let (s, tree) = seed_big_tree(&url, &new_schema("big"), &spec);
    for rule in FORMS {
        check_big_tree_pages(&s, &tree, &spec, rule);
    }
    check_children_far_apart(&s, &tree, &spec);
    // The store's own rule: a tree of 100,000 nodes uses the path form, so the whole subtree pages.
    assert_eq!(s.tree_query_form(&tree).unwrap(), TreeQueryForm::Path);
    check_big_tree_pages(&s, &tree, &spec, TreeQueryRule::StoreDefault);
}

fn plan_of(s: &PgStore, query: TreeQuery, form: TreeQueryForm, tree: &str, node: u64) -> String {
    let plan = s
        .explain_tree_query(query, form, tree, node)
        .unwrap()
        .join("\n");
    eprintln!(
        "{query:?} / {form:?} (node {node}):\n  {}",
        plan.replace('\n', "\n  ")
    );
    plan
}

#[test]
fn query_plans_on_a_hundred_thousand_node_tree_use_the_intended_indexes() {
    use TreeQuery::*;
    use TreeQueryForm::{Cte, Path};
    let Some(url) = url() else { return };
    let spec = big_tree(BIG_NODES, 99);
    let (s, tree) = seed_big_tree(&url, &new_schema("plan"), &spec);

    // Path form: an index scan on the primary key with both node bounds, and no sort node.
    for node in [0, 50] {
        let text = plan_of(&s, SubtreeDescendants, Path, &tree, node);
        assert!(
            text.contains("Index Scan using trial_lineage_pkey"),
            "primary key: {text}"
        );
        assert!(
            text.contains("node > ") && text.contains("node <= "),
            "both node bounds: {text}"
        );
        assert!(!text.contains("Sort"), "no sort node: {text}");
    }
    plan_of(&s, SubtreeTally, Path, &tree, 50);
    let text = plan_of(&s, SubtreeFirstFailing, Path, &tree, 50);
    assert!(!text.contains("Sort"), "{text}");

    // Recursive form: the recursion step is an index search on the parent index, and nothing scans the table.
    for query in [SubtreeDescendants, SubtreeTally, SubtreeFirstFailing] {
        let text = plan_of(&s, query, Cte, &tree, 50);
        assert!(text.contains("Recursive Union"), "{query:?}: {text}");
        assert!(
            text.contains("lineage_children"),
            "{query:?} walks the parent index: {text}"
        );
        assert!(
            !text.contains("Seq Scan on trial_lineage"),
            "{query:?} must not scan the table: {text}"
        );
    }
    // Children: keyset on the parent index with the cursor as a bound, no sort node.
    let text = plan_of(&s, TreeChildren, Cte, &tree, 50);
    assert!(text.contains("lineage_children"), "{text}");
    assert!(
        text.contains("node > "),
        "the cursor bounds the scan: {text}"
    );
    assert!(!text.contains("Sort"), "{text}");
    // Ancestors: one primary-key search per step.
    let text = plan_of(&s, Ancestors, Cte, &tree, 99_999);
    assert!(text.contains("Recursive Union"), "{text}");
    assert!(text.contains("trial_lineage_pkey"), "{text}");
    assert!(!text.contains("Seq Scan on trial_lineage"), "{text}");
}

#[test]
fn the_store_rule_switches_to_the_path_form_at_the_tree_size_threshold() {
    let Some(url) = url() else { return };
    let schema = new_schema("r");
    let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
    let run = new_run(&s, "sim");
    let tree = make_tree(&s, &run, None);
    let mut c = raw(&url, &schema);
    let mut set_count = |n: u64| {
        c.execute(
            "UPDATE trees SET node_count = $1 WHERE id = $2",
            &[&(n as i64), &tree.id],
        )
        .unwrap();
    };
    set_count(PG_PATH_FORM_MIN_NODES - 1);
    assert_eq!(
        s.tree_query_form(&tree.id).unwrap(),
        TreeQueryForm::Cte,
        "below the threshold"
    );
    set_count(PG_PATH_FORM_MIN_NODES);
    assert_eq!(
        s.tree_query_form(&tree.id).unwrap(),
        TreeQueryForm::Path,
        "at the threshold"
    );
    s.set_tree_query_rule(TreeQueryRule::PathFromNodes(PG_PATH_FORM_MIN_NODES + 1));
    assert_eq!(
        s.tree_query_form(&tree.id).unwrap(),
        TreeQueryForm::Cte,
        "the threshold is configurable"
    );
    s.set_tree_query_rule(TreeQueryRule::AlwaysCte);
    assert_eq!(s.tree_query_form(&tree.id).unwrap(), TreeQueryForm::Cte);
    s.set_tree_query_rule(TreeQueryRule::StoreDefault);
    assert_eq!(s.tree_query_rule(), TreeQueryRule::StoreDefault);
}

/// The interleaving: `finish_job` holds the source run's row FOR UPDATE and, in the same transaction, deletes that
/// run's nodes (the finishing attempt never ingested; an earlier attempt left nodes). `add_branch` has already read the
/// source node and blocks on the run's FOR SHARE lock. When the first transaction commits, the run reads as finished,
/// but the node is gone: the branch must be refused, not stored against a node that no longer exists.
#[test]
fn a_branch_is_refused_when_its_source_node_is_deleted_while_it_waits_for_the_run() {
    let Some(url) = url() else { return };
    let schema = new_schema("rc");
    let s = std::sync::Arc::new(PgStore::connect_in_schema(&url, &schema, 4).unwrap());
    let root_run = run_with_identity(&*s, "a\n");
    ingest_verdicts(&*s, &root_run.id, &passes(1));
    let tree = make_tree(&*s, &root_run, Some(sha256_hex(b"a\n")));
    add(&*s, &tree.id, &root(&root_run.id, 0));
    finish(&*s, &root_run.id);
    // The source run is a running branch run with one node.
    let source_run = run_with_identity(&*s, "a\n");
    assert!(matches!(
        s.add_tree_branch(&branch_for(&tree.id, &source_run.id, 0, &"1".repeat(64)), 9)
            .unwrap(),
        BranchOutcome::Added(_)
    ));
    ingest_verdicts(&*s, &source_run.id, &passes(1));
    let node = add(&*s, &tree.id, &child(&source_run.id, 0, 0, 1));
    let new_branch = run_with_identity(&*s, "a\n");

    // Transaction A plays finish_job: lock the run, delete its nodes, finish it, but do not commit yet.
    let mut a = raw(&url, &schema);
    a.batch_execute("BEGIN").unwrap();
    a.execute(
        "SELECT 1 FROM runs WHERE id = $1 FOR UPDATE",
        &[&source_run.id],
    )
    .unwrap();
    a.execute(
        "DELETE FROM trial_lineage WHERE run_id = $1",
        &[&source_run.id],
    )
    .unwrap();
    a.execute(
        "UPDATE runs SET state = 'passed' WHERE id = $1",
        &[&source_run.id],
    )
    .unwrap();

    let waiting = {
        let (s, tree_id, run_id, source) =
            (s.clone(), tree.id.clone(), new_branch.id.clone(), node.node);
        std::thread::spawn(move || {
            s.add_tree_branch(&branch_for(&tree_id, &run_id, source, &"2".repeat(64)), 9)
                .unwrap()
        })
    };
    // Let add_branch read the node and block on the run lock, then commit the deletion.
    std::thread::sleep(std::time::Duration::from_millis(700));
    assert!(
        !waiting.is_finished(),
        "add_branch must be waiting on the run lock"
    );
    a.batch_execute("COMMIT").unwrap();

    assert_eq!(
        waiting.join().unwrap(),
        BranchOutcome::Rejected(TreeRejection::SourceNotFound)
    );
    assert_eq!(s.get_tree_branch(&new_branch.id).unwrap(), None);
    assert_eq!(s.get_tree(&tree.id).unwrap().unwrap().run_count, 2);
}

#[test]
fn a_run_with_dependants_cannot_start_a_new_attempt_even_if_a_branch_row_exists() {
    let Some(url) = url() else { return };
    for which in 0..2 {
        let schema = new_schema(&format!("ab{which}"));
        let s = PgStore::connect_in_schema(&url, &schema, 2).unwrap();
        let c = std::cell::RefCell::new(raw(&url, &schema));
        // A branch row the way nothing but a test can add one: the store refuses a branch below a running run.
        let hook = |tree: &str, run: &str, source: u64| {
            c.borrow_mut()
                .execute(
                    "INSERT INTO tree_branches(run_id,tree_id,source_node,cut_kind,cut_at,point_hash,command_json,
                       command_hash,seed_source,created_by,created_at)
                     VALUES ($1,$2,$3,'step',3,'p','{}',$1,'server_random','k',1)",
                    &[&run, &tree, &(source as i64)],
                )
                .unwrap();
        };
        if which == 0 {
            common::tree_attempts::a_root_run_with_branches_cannot_start_a_new_attempt(&s, &hook);
        } else {
            common::tree_attempts::a_branch_run_with_dependants_cannot_start_a_new_attempt(
                &s, &hook,
            );
        }
    }
}

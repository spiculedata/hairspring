//! Trial tree queries on SQLite: caps, the independent recursive oracle, a 100k-node tree in both forms, and the
//! query plans of every form (on a database that was never analysed and on one that was).
mod common;
use common::tree_oracle::*;
use common::tree_support::*;
use hairspring_store::*;
use rusqlite::Connection;

#[test]
fn the_node_cap_is_enforced() {
    let s = SqliteStore::open_in_memory().unwrap();
    common::tree::node_cap_is_enforced(&s, &|tree, n| {
        s.with_connection(|conn| {
            conn.execute(
                "UPDATE trees SET node_count=? WHERE id=?",
                rusqlite::params![n as i64, tree],
            )
            .unwrap()
        });
    });
}

/// The independent oracle: a plain `WITH RECURSIVE` over `parent_node`, written for the harness and not shared with
/// the store (its joins are `CROSS JOIN`s for the reason given in the store's query module).
fn recursive_answer(conn: &Connection, tree: &str, a: u64) -> CteAnswer {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE sub(node) AS (
               SELECT ?2 UNION ALL
               SELECT l.node FROM sub s CROSS JOIN trial_lineage l ON l.parent_node = s.node WHERE l.tree_id = ?1)
             SELECT l.node, l.verdict, l.depth FROM sub CROSS JOIN trial_lineage l ON l.node = sub.node
              WHERE l.tree_id = ?1 ORDER BY l.node",
        )
        .unwrap();
    let subtree = stmt
        .query_map(rusqlite::params![tree, a as i64], |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let mut up = conn
        .prepare(
            "WITH RECURSIVE up(node, parent) AS (
               SELECT node, parent_node FROM trial_lineage WHERE tree_id = ?1 AND node = ?2 UNION ALL
               SELECT l.node, l.parent_node FROM up CROSS JOIN trial_lineage l ON l.node = up.parent WHERE l.tree_id = ?1)
             SELECT up.node FROM up CROSS JOIN trial_lineage l ON l.node = up.node AND l.tree_id = ?1
              WHERE up.node <> ?2 ORDER BY l.depth DESC",
        )
        .unwrap();
    let ancestors = up
        .query_map(rusqlite::params![tree, a as i64], |r| {
            Ok(r.get::<_, i64>(0)? as u64)
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    CteAnswer { subtree, ancestors }
}

#[test]
fn subtree_queries_match_a_recursive_query_on_random_trees_in_both_forms() {
    for (seed, n, shape) in ORACLE_CASES {
        let s = SqliteStore::open_in_memory().unwrap();
        let spec = generate(*seed, *n, *shape);
        let started = std::time::Instant::now();
        let tree = load(&s, &spec);
        let loaded = started.elapsed();
        let cte = |t: &str, a: u64| s.with_connection(|conn| recursive_answer(conn, t, a));
        check_against_oracles(&s, &tree, &spec, &cte, *seed);
        eprintln!(
            "oracle case seed {seed}: {n} nodes ({shape:?}), max depth {}, loaded in {loaded:?}, checked in {:?}",
            spec.depths.iter().max().unwrap(),
            started.elapsed() - loaded
        );
    }
}

/// Seeds a big tree with raw SQL (the store API would take minutes), then lets the store answer.
fn seed_big_tree(spec: &TreeSpec) -> (SqliteStore, String) {
    let s = SqliteStore::open_in_memory().unwrap();
    let run = new_run(&s, "sim");
    let tree = make_tree(&s, &run, None);
    s.with_connection(|conn| seed_lineage(conn, &run.id, &tree.id, spec));
    (s, tree.id)
}

fn seed_lineage(conn: &mut Connection, run_id: &str, tree_id: &str, spec: &TreeSpec) {
    let paths = paths_of(spec);
    let tx = conn.transaction().unwrap();
    {
        let mut trial = tx
            .prepare(
                "INSERT INTO trials(run_id,seq,seed,verdict,reason,timed_out,degraded,perturbing,report_missing,rated,
                   claimable,syscalls,work_ops,elapsed_ms,worker_id,worker_trusted,attempt,kept,digest)
                 VALUES (?,?,?,?,'',0,0,0,0,1,1,1,1,1,'w',1,1,'none',x'01000000')",
            )
            .unwrap();
        let mut node = tx
            .prepare(
                "INSERT INTO trial_lineage(tree_id,node,run_id,seq,parent_node,depth,path,verdict,operator,cut_kind,
                   cut_at,origin,reporter_id,reporter_trusted,verified,created_at)
                 VALUES (?1,?2,?3,?2,?4,?5,?6,?7,?8,?9,?10,'exploration','w',1,'no',1)",
            )
            .unwrap();
        for (i, parent) in spec.parents.iter().enumerate() {
            let verdict = spec.verdicts[i].as_str();
            trial
                .execute(rusqlite::params![run_id, i as i64, i as i64, verdict])
                .unwrap();
            let (op, cut_kind, cut_at) = if parent.is_some() {
                ("explore", "step", (i % 8) as i64)
            } else {
                ("root", "none", 0)
            };
            node.execute(rusqlite::params![
                tree_id,
                i as i64,
                run_id,
                parent.map(|p| p as i64),
                spec.depths[i],
                paths[i],
                verdict,
                op,
                cut_kind,
                cut_at
            ])
            .unwrap();
        }
    }
    tx.execute(
        "UPDATE trees SET node_count=? WHERE id=?",
        rusqlite::params![spec.parents.len() as i64, tree_id],
    )
    .unwrap();
    tx.commit().unwrap();
}

const BIG_NODES: usize = 100_000;

#[test]
fn a_hundred_thousand_node_tree_is_paged_in_both_forms_and_children_cross_big_gaps() {
    let spec = big_tree(BIG_NODES, 99);
    let (s, tree) = seed_big_tree(&spec);
    for rule in FORMS {
        check_big_tree_pages(&s, &tree, &spec, rule);
    }
    check_children_far_apart(&s, &tree, &spec);
}

fn plan_of(
    s: &SqliteStore,
    query: TreeQuery,
    form: TreeQueryForm,
    tree: &str,
    node: u64,
) -> String {
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
    let spec = big_tree(BIG_NODES, 99);
    let (s, tree) = seed_big_tree(&spec);
    // The plans must hold on a database that was never analysed (a real one) and on one that was.
    for analysed in [false, true] {
        if analysed {
            s.with_connection(|c| c.execute_batch("ANALYZE").unwrap());
        }
        eprintln!("--- statistics gathered: {analysed}");

        // Path form: the primary key with both node bounds, and no sort step.
        for node in [0, 50] {
            let text = plan_of(&s, SubtreeDescendants, Path, &tree, node);
            assert!(
                text.contains("USING PRIMARY KEY (tree_id=? AND node>? AND node<?)"),
                "both node bounds on the primary key: {text}"
            );
            assert!(!text.contains("TEMP B-TREE"), "no sort step: {text}");
        }
        plan_of(&s, SubtreeTally, Path, &tree, 50);
        let text = plan_of(&s, SubtreeFirstFailing, Path, &tree, 50);
        assert!(
            !text.contains("TEMP B-TREE"),
            "walks lineage_verdict in order: {text}"
        );

        // Recursive form: every step is an index search on the parent index, joined back by primary key. Nothing
        // scans the table, and no search is bounded by the tree alone (that is a scan of the whole tree).
        for query in [SubtreeDescendants, SubtreeTally, SubtreeFirstFailing] {
            let text = plan_of(&s, query, Cte, &tree, 50);
            assert!(
                text.contains("lineage_children (tree_id=? AND parent_node=?)"),
                "{query:?}: {text}"
            );
            assert!(
                text.contains("USING PRIMARY KEY (tree_id=? AND node=?)"),
                "{query:?}: {text}"
            );
            assert!(
                !text.contains("SCAN trial_lineage"),
                "{query:?} must not scan the table: {text}"
            );
            assert!(
                !text.contains("(tree_id=?)"),
                "{query:?} must not scan the whole tree: {text}"
            );
        }
        // Children: keyset on the parent index, no window upper bound and no sort.
        let text = plan_of(&s, TreeChildren, Cte, &tree, 50);
        assert!(
            text.contains("lineage_children (tree_id=? AND parent_node=? AND node>?)"),
            "{text}"
        );
        assert!(!text.contains("TEMP B-TREE"), "{text}");
        // Ancestors: one primary-key search per step.
        let text = plan_of(&s, Ancestors, Cte, &tree, 99_999);
        assert!(
            text.contains("USING PRIMARY KEY (tree_id=? AND node=?)"),
            "{text}"
        );
        assert!(!text.contains("SCAN trial_lineage"), "{text}");
        assert!(
            !text.contains("(tree_id=?)"),
            "ancestors must not scan the whole tree: {text}"
        );
        assert!(!text.contains("AUTOMATIC"), "{text}");
    }
}

#[test]
fn the_store_rule_is_the_recursive_form_for_every_tree_size_on_sqlite() {
    let s = SqliteStore::open_in_memory().unwrap();
    let small = new_run(&s, "sim");
    let tree = make_tree(&s, &small, None);
    assert_eq!(s.tree_query_form(&tree.id).unwrap(), TreeQueryForm::Cte);
    s.with_connection(|c| {
        c.execute(
            "UPDATE trees SET node_count=? WHERE id=?",
            rusqlite::params![PG_PATH_FORM_MIN_NODES as i64, tree.id],
        )
        .unwrap()
    });
    assert_eq!(
        s.tree_query_form(&tree.id).unwrap(),
        TreeQueryForm::Cte,
        "SQLite stays recursive at any size"
    );
    s.set_tree_query_rule(TreeQueryRule::PathFromNodes(10));
    assert_eq!(s.tree_query_form(&tree.id).unwrap(), TreeQueryForm::Path);
    s.set_tree_query_rule(TreeQueryRule::StoreDefault);
    assert_eq!(s.tree_query_rule(), TreeQueryRule::StoreDefault);
}

fn raw_branch(s: &SqliteStore) -> impl Fn(&str, &str, u64) + '_ {
    move |tree, run, source| {
        s.with_connection(|c| {
            c.execute(
                "INSERT INTO tree_branches(run_id,tree_id,source_node,cut_kind,cut_at,point_hash,command_json,
                   command_hash,seed_source,created_by,created_at)
                 VALUES (?1,?2,?3,'step',3,'p','{}',?1,'server_random','k',1)",
                rusqlite::params![run, tree, source as i64],
            )
            .unwrap()
        });
    }
}

#[test]
fn a_run_with_dependants_cannot_start_a_new_attempt_even_if_a_branch_row_exists() {
    let s = SqliteStore::open_in_memory().unwrap();
    common::tree_attempts::a_root_run_with_branches_cannot_start_a_new_attempt(&s, &raw_branch(&s));
    let s = SqliteStore::open_in_memory().unwrap();
    common::tree_attempts::a_branch_run_with_dependants_cannot_start_a_new_attempt(
        &s,
        &raw_branch(&s),
    );
}

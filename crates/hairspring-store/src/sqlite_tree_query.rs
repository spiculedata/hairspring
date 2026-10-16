//! SQLite tree reads: one query layer, two forms. Subtree queries run either as a recursive query over `parent_node`
//! (the default on SQLite) or as a range scan of the materialised path; the rule in [`TreeQueryRule`] picks, and both
//! forms return the same rows. Ancestors always use the recursive form, and children always use keyset paging on
//! `lineage_children`. Every statement binds its inputs; the path bounds come from the stored path of the node.
use crate::lineage::*;
use crate::sqlite_lineage::{get_node, get_tree, node_from_row};
use crate::trials::{to_db, TrialVerdict};
use crate::types::{Result, StoreError};
use rusqlite::{params, Connection, OptionalExtension};

/// SQLite never picks the path form by size: the bench found the recursive form within reach on every shape.
const SQLITE_PATH_MIN_NODES: Option<u64> = None;

/// The node and all its descendants, walking `lineage_children` (one index search per step). `?1` tree, `?2` node.
/// Every join here is a `CROSS JOIN`: in SQLite that fixes the order (the walked rows first, one index search per
/// row), so the plan does not depend on statistics. Written as plain joins, the recursion step on a database that was
/// never analysed is planned as a scan of the whole tree for every step.
macro_rules! subtree_cte {
    () => {
        "WITH RECURSIVE sub(node) AS (SELECT ?2 UNION ALL \
           SELECT l.node FROM sub s CROSS JOIN trial_lineage l ON l.tree_id=?1 AND l.parent_node=s.node) "
    };
}

/// Path form, a window of node numbers read in node order and filtered by path. The unary plus keeps the planner off
/// `lineage_path`: that index would find the subtree but then sort it, and every page would cost the whole subtree.
/// The primary key hands the window over already in node order.
const DESCENDANTS_PATH_SQL: &str = concat!(
    "SELECT ",
    node_columns!(""),
    " FROM trial_lineage \
     WHERE tree_id=?1 AND node>?2 AND node<=?3 AND +path>?4 AND +path<?5 ORDER BY node LIMIT ?6"
);
const TALLY_PATH_SQL: &str = "SELECT verdict, COUNT(*) FROM trial_lineage \
     WHERE tree_id=?1 AND path>=?2 AND path<?3 GROUP BY verdict ORDER BY verdict";
/// The unary plus again: the first failing node walks `lineage_verdict` in (depth, node) order and stops at the first
/// row inside the path range, so there is nothing to sort.
const FIRST_FAILING_PATH_SQL: &str = "SELECT node FROM trial_lineage \
     WHERE tree_id=?1 AND verdict='violation' AND +path>=?2 AND +path<?3 ORDER BY depth, node LIMIT 1";

/// Recursive form. The descendants are the walk without its start node (the cursor starts at the start node), paged
/// by keyset after a bounded probe has shown the walk is not too large.
const DESCENDANTS_CTE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT ",
    node_columns!("x."),
    " FROM sub CROSS JOIN trial_lineage x ON x.tree_id=?1 AND x.node=sub.node \
     WHERE sub.node>?3 ORDER BY sub.node LIMIT ?4"
);
const DESCENDANTS_PROBE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT COUNT(*) FROM (SELECT node FROM sub LIMIT ?3)"
);
const TALLY_CTE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT x.verdict, COUNT(*) FROM sub CROSS JOIN trial_lineage x ON x.tree_id=?1 AND x.node=sub.node \
     GROUP BY x.verdict ORDER BY x.verdict"
);
const FIRST_FAILING_CTE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT x.node FROM sub CROSS JOIN trial_lineage x ON x.tree_id=?1 AND x.node=sub.node \
     WHERE x.verdict='violation' ORDER BY x.depth, x.node LIMIT 1"
);

/// Children: keyset paging, bounded by LIMIT through the index. One row more than the page asks whether more exist.
/// `INDEXED BY` pins the parent index: left to itself, a database without statistics may walk the primary key from the
/// cursor and filter by parent, which is a scan of the rest of the tree.
const CHILDREN_SQL: &str = concat!(
    "SELECT ",
    node_columns!(""),
    " FROM trial_lineage INDEXED BY lineage_children \
     WHERE tree_id=?1 AND parent_node=?2 AND node>?3 ORDER BY node LIMIT ?4"
);

/// Ancestors, from the node up through `parent_node` (one primary-key search per step), nearest the root first.
const ANCESTORS_SQL: &str = concat!(
    "WITH RECURSIVE up(node, parent_node) AS (\
       SELECT node, parent_node FROM trial_lineage WHERE tree_id=?1 AND node=?2 UNION ALL \
       SELECT l.node, l.parent_node FROM up CROSS JOIN trial_lineage l ON l.tree_id=?1 AND l.node=up.parent_node) \
     SELECT ", node_columns!("x."), " FROM up CROSS JOIN trial_lineage x ON x.tree_id=?1 AND x.node=up.node \
     WHERE x.node<>?2 ORDER BY x.depth"
);

fn need_tree(conn: &Connection, tree_id: &str) -> Result<TreeRecord> {
    get_tree(conn, tree_id)?.ok_or_else(|| StoreError::NotFound(format!("tree {tree_id}")))
}

fn need_node(conn: &Connection, tree_id: &str, node: u64) -> Result<NodeRecord> {
    get_node(conn, tree_id, node)?
        .ok_or_else(|| StoreError::NotFound(format!("node {node} of tree {tree_id}")))
}

/// The form the rule picks for a tree of this size.
pub(crate) fn form_for(rule: TreeQueryRule, node_count: u64) -> TreeQueryForm {
    rule.form(SQLITE_PATH_MIN_NODES, node_count)
}

fn read_nodes(
    conn: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<NodeRecord>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt
        .query_map(args, node_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn tally_rows(
    conn: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<VerdictCount>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt
        .query_map(args, |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(v, n)| {
            let verdict = TrialVerdict::parse(&v)
                .ok_or_else(|| StoreError::Backend(format!("unknown verdict `{v}`")))?;
            Ok(VerdictCount {
                verdict,
                count: n as u64,
            })
        })
        .collect()
}

pub(crate) fn descendants(
    conn: &Connection,
    rule: TreeQueryRule,
    tree_id: &str,
    node: u64,
    after: Option<u64>,
    limit: u32,
) -> Result<NodePage> {
    let tree = need_tree(conn, tree_id)?;
    let at = need_node(conn, tree_id, node)?;
    let (start, limit) = (window_start(after, node), page_limit(limit));
    match form_for(rule, tree.node_count) {
        TreeQueryForm::Path => {
            let (lo, hi) = subtree_bounds(&at.path);
            let nodes = read_nodes(
                conn,
                DESCENDANTS_PATH_SQL,
                &[
                    &tree_id,
                    &to_db(start),
                    &to_db(window_end(start)),
                    &lo,
                    &hi,
                    &limit,
                ],
            )?;
            Ok(window_page(nodes, start, limit, tree.node_count))
        }
        TreeQueryForm::Cte => {
            let walked: i64 = conn.prepare_cached(DESCENDANTS_PROBE_SQL)?.query_row(
                params![tree_id, to_db(node), to_db(MAX_CTE_DESCENDANTS + 2)],
                |r| r.get(0),
            )?;
            if walked as u64 > MAX_CTE_DESCENDANTS + 1 {
                return Err(StoreError::LimitExceeded {
                    what: "descendants of a node",
                    limit: MAX_CTE_DESCENDANTS,
                });
            }
            let nodes = read_nodes(
                conn,
                DESCENDANTS_CTE_SQL,
                &[
                    &tree_id,
                    &to_db(node),
                    &to_db(start),
                    &(i64::from(limit) + 1),
                ],
            )?;
            Ok(keyset_page(nodes, limit))
        }
    }
}

pub(crate) fn children(
    conn: &Connection,
    tree_id: &str,
    node: u64,
    after: Option<u64>,
    limit: u32,
) -> Result<NodePage> {
    need_tree(conn, tree_id)?;
    need_node(conn, tree_id, node)?;
    let (start, limit) = (window_start(after, node), page_limit(limit));
    let nodes = read_nodes(
        conn,
        CHILDREN_SQL,
        &[
            &tree_id,
            &to_db(node),
            &to_db(start),
            &(i64::from(limit) + 1),
        ],
    )?;
    Ok(keyset_page(nodes, limit))
}

pub(crate) fn tally(
    conn: &Connection,
    rule: TreeQueryRule,
    tree_id: &str,
    node: u64,
) -> Result<Vec<VerdictCount>> {
    let tree = need_tree(conn, tree_id)?;
    let at = need_node(conn, tree_id, node)?;
    match form_for(rule, tree.node_count) {
        TreeQueryForm::Path => {
            let (lo, hi) = subtree_bounds(&at.path);
            tally_rows(conn, TALLY_PATH_SQL, &[&tree_id, &lo, &hi])
        }
        TreeQueryForm::Cte => tally_rows(conn, TALLY_CTE_SQL, &[&tree_id, &to_db(node)]),
    }
}

pub(crate) fn first_failing(
    conn: &Connection,
    rule: TreeQueryRule,
    tree_id: &str,
    node: u64,
) -> Result<Option<u64>> {
    let tree = need_tree(conn, tree_id)?;
    let at = need_node(conn, tree_id, node)?;
    let found: Option<i64> = match form_for(rule, tree.node_count) {
        TreeQueryForm::Path => {
            let (lo, hi) = subtree_bounds(&at.path);
            conn.prepare_cached(FIRST_FAILING_PATH_SQL)?
                .query_row(params![tree_id, lo, hi], |r| r.get(0))
                .optional()?
        }
        TreeQueryForm::Cte => conn
            .prepare_cached(FIRST_FAILING_CTE_SQL)?
            .query_row(params![tree_id, to_db(node)], |r| r.get(0))
            .optional()?,
    };
    Ok(found.map(|n| n as u64))
}

pub(crate) fn ancestors(conn: &Connection, tree_id: &str, node: u64) -> Result<Vec<NodeRecord>> {
    need_node(conn, tree_id, node)?;
    read_nodes(conn, ANCESTORS_SQL, &[&tree_id, &to_db(node)])
}

/// The plan SQLite chooses for a tree query in the given form, one line per step, with real bound values.
pub(crate) fn explain(
    conn: &Connection,
    query: TreeQuery,
    form: TreeQueryForm,
    tree_id: &str,
    node: u64,
) -> Result<Vec<String>> {
    use TreeQueryForm::{Cte, Path};
    let at = need_node(conn, tree_id, node)?;
    let (lo, hi) = subtree_bounds(&at.path);
    let (start, end, limit) = (to_db(node), to_db(window_end(node)), 501i64);
    let (sql, bound): (&str, Vec<rusqlite::types::Value>) = match (query, form) {
        (TreeQuery::SubtreeDescendants, Path) => (
            DESCENDANTS_PATH_SQL,
            vec![
                tree_id.to_string().into(),
                start.into(),
                end.into(),
                lo.into(),
                hi.into(),
                500i64.into(),
            ],
        ),
        (TreeQuery::SubtreeDescendants, Cte) => (
            DESCENDANTS_CTE_SQL,
            vec![
                tree_id.to_string().into(),
                start.into(),
                start.into(),
                limit.into(),
            ],
        ),
        (TreeQuery::TreeChildren, _) => (
            CHILDREN_SQL,
            vec![
                tree_id.to_string().into(),
                start.into(),
                start.into(),
                limit.into(),
            ],
        ),
        (TreeQuery::SubtreeTally, Path) => (
            TALLY_PATH_SQL,
            vec![tree_id.to_string().into(), lo.into(), hi.into()],
        ),
        (TreeQuery::SubtreeTally, Cte) => (
            TALLY_CTE_SQL,
            vec![tree_id.to_string().into(), start.into()],
        ),
        (TreeQuery::SubtreeFirstFailing, Path) => (
            FIRST_FAILING_PATH_SQL,
            vec![tree_id.to_string().into(), lo.into(), hi.into()],
        ),
        (TreeQuery::SubtreeFirstFailing, Cte) => (
            FIRST_FAILING_CTE_SQL,
            vec![tree_id.to_string().into(), start.into()],
        ),
        (TreeQuery::Ancestors, _) => (
            ANCESTORS_SQL,
            vec![tree_id.to_string().into(), start.into()],
        ),
    };
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bound), |r| r.get::<_, String>(3))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

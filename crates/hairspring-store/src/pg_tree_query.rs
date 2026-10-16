//! PostgreSQL tree reads: one query layer, two forms. Subtree queries run either as a recursive query over
//! `parent_node` or as a range scan of the materialised path; the rule in [`TreeQueryRule`] picks (recursive below
//! [`PG_PATH_FORM_MIN_NODES`] nodes, path at or above), and both forms return the same rows. Ancestors always use the
//! recursive form, and children always use keyset paging on `lineage_children`. Every statement binds its inputs.
use crate::lineage::*;
use crate::pg_lineage::{get_node, get_tree, nodes_from_rows};
use crate::trials::{to_db, TrialVerdict};
use crate::types::{Result, StoreError};
use postgres::Client;

type Param<'a> = &'a (dyn postgres::types::ToSql + Sync);

/// The node and all its descendants, walking `lineage_children`. `$1` tree, `$2` node.
macro_rules! subtree_cte {
    () => {
        "WITH RECURSIVE sub(node) AS (SELECT $2::bigint UNION ALL \
           SELECT l.node FROM sub s JOIN trial_lineage l ON l.tree_id=$1 AND l.parent_node=s.node) "
    };
}

/// Path form, a window of node numbers read in node order and filtered by path. The primary key hands the window over
/// already in node order, so the page stops at its size. Run with sorting disabled for the transaction
/// ([`ordered_scans`]): left to itself the planner finds a small subtree through `lineage_path` and sorts it, so a
/// page would cost the whole subtree instead of at most the window.
const DESCENDANTS_PATH_SQL: &str = concat!(
    "SELECT ",
    node_columns!(""),
    " FROM trial_lineage \
     WHERE tree_id=$1 AND node>$2 AND node<=$3 AND path>$4 AND path<$5 ORDER BY node LIMIT $6"
);
const TALLY_PATH_SQL: &str = "SELECT verdict, COUNT(*) FROM trial_lineage \
     WHERE tree_id=$1 AND path>=$2 AND path<$3 GROUP BY verdict ORDER BY verdict";
/// Walks `lineage_verdict` in (depth, node) order and stops at the first row inside the path range (also run with
/// sorting disabled, so the planner does not sort a path range instead).
const FIRST_FAILING_PATH_SQL: &str = "SELECT node FROM trial_lineage \
     WHERE tree_id=$1 AND verdict='violation' AND path>=$2 AND path<$3 ORDER BY depth, node LIMIT 1";

/// Runs `f` in a transaction in which the planner is told not to sort when an index can deliver the order (a cost
/// penalty, not a prohibition: the setting lasts for this transaction only).
fn ordered_scans<T>(
    c: &mut Client,
    f: impl FnOnce(&mut postgres::Transaction) -> Result<T>,
) -> Result<T> {
    let mut tx = c.transaction()?;
    tx.batch_execute("SET LOCAL enable_sort = off")?;
    let out = f(&mut tx)?;
    tx.commit()?;
    Ok(out)
}

/// Recursive form. The descendants are the walk without its start node (the cursor starts at the start node), paged
/// by keyset after a bounded probe has shown the walk is not too large.
const DESCENDANTS_CTE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT ",
    node_columns!("x."),
    " FROM sub JOIN trial_lineage x ON x.tree_id=$1 AND x.node=sub.node \
     WHERE sub.node>$3 ORDER BY sub.node LIMIT $4"
);
const DESCENDANTS_PROBE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT COUNT(*) FROM (SELECT node FROM sub LIMIT $3) probe"
);
const TALLY_CTE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT x.verdict, COUNT(*) FROM sub JOIN trial_lineage x ON x.tree_id=$1 AND x.node=sub.node \
     GROUP BY x.verdict ORDER BY x.verdict"
);
const FIRST_FAILING_CTE_SQL: &str = concat!(
    subtree_cte!(),
    "SELECT x.node FROM sub JOIN trial_lineage x ON x.tree_id=$1 AND x.node=sub.node \
     WHERE x.verdict='violation' ORDER BY x.depth, x.node LIMIT 1"
);

/// Children: keyset paging, bounded by LIMIT through the index. One row more than the page asks whether more exist.
const CHILDREN_SQL: &str = concat!(
    "SELECT ",
    node_columns!(""),
    " FROM trial_lineage \
     WHERE tree_id=$1 AND parent_node=$2 AND node>$3 ORDER BY node LIMIT $4"
);

/// Ancestors, from the node up through `parent_node` (one primary-key search per step), nearest the root first.
const ANCESTORS_SQL: &str = concat!(
    "WITH RECURSIVE up(node, parent_node) AS (\
       SELECT node, parent_node FROM trial_lineage WHERE tree_id=$1 AND node=$2::bigint UNION ALL \
       SELECT l.node, l.parent_node FROM up JOIN trial_lineage l ON l.tree_id=$1 AND l.node=up.parent_node) \
     SELECT ", node_columns!("x."), " FROM up JOIN trial_lineage x ON x.tree_id=$1 AND x.node=up.node \
     WHERE x.node<>$2 ORDER BY x.depth"
);

fn need_tree(c: &mut Client, tree_id: &str) -> Result<TreeRecord> {
    get_tree(c, tree_id)?.ok_or_else(|| StoreError::NotFound(format!("tree {tree_id}")))
}

fn need_node(c: &mut Client, tree_id: &str, node: u64) -> Result<NodeRecord> {
    get_node(c, tree_id, node)?
        .ok_or_else(|| StoreError::NotFound(format!("node {node} of tree {tree_id}")))
}

/// The form the rule picks for a tree of this size.
pub(crate) fn form_for(rule: TreeQueryRule, node_count: u64) -> TreeQueryForm {
    rule.form(Some(PG_PATH_FORM_MIN_NODES), node_count)
}

fn read_tally(c: &mut Client, sql: &str, args: &[Param]) -> Result<Vec<VerdictCount>> {
    c.query(sql, args)?
        .iter()
        .map(|r| {
            let text: String = r.get(0);
            let verdict = TrialVerdict::parse(&text)
                .ok_or_else(|| StoreError::Backend(format!("unknown verdict `{text}`")))?;
            Ok(VerdictCount {
                verdict,
                count: r.get::<_, i64>(1) as u64,
            })
        })
        .collect()
}

pub(crate) fn descendants(
    c: &mut Client,
    rule: TreeQueryRule,
    tree_id: &str,
    node: u64,
    after: Option<u64>,
    limit: u32,
) -> Result<NodePage> {
    let tree = need_tree(c, tree_id)?;
    let at = need_node(c, tree_id, node)?;
    let (start, limit) = (window_start(after, node), page_limit(limit));
    match form_for(rule, tree.node_count) {
        TreeQueryForm::Path => {
            let (lo, hi) = subtree_bounds(&at.path);
            let rows = ordered_scans(c, |tx| {
                Ok(tx.query(
                    DESCENDANTS_PATH_SQL,
                    &[
                        &tree_id,
                        &to_db(start),
                        &to_db(window_end(start)),
                        &lo,
                        &hi,
                        &i64::from(limit),
                    ],
                )?)
            })?;
            Ok(window_page(
                nodes_from_rows(&rows)?,
                start,
                limit,
                tree.node_count,
            ))
        }
        TreeQueryForm::Cte => {
            let walked: i64 = c
                .query_one(
                    DESCENDANTS_PROBE_SQL,
                    &[&tree_id, &to_db(node), &to_db(MAX_CTE_DESCENDANTS + 2)],
                )?
                .get(0);
            if walked as u64 > MAX_CTE_DESCENDANTS + 1 {
                return Err(StoreError::LimitExceeded {
                    what: "descendants of a node",
                    limit: MAX_CTE_DESCENDANTS,
                });
            }
            let rows = c.query(
                DESCENDANTS_CTE_SQL,
                &[
                    &tree_id,
                    &to_db(node),
                    &to_db(start),
                    &(i64::from(limit) + 1),
                ],
            )?;
            Ok(keyset_page(nodes_from_rows(&rows)?, limit))
        }
    }
}

pub(crate) fn children(
    c: &mut Client,
    tree_id: &str,
    node: u64,
    after: Option<u64>,
    limit: u32,
) -> Result<NodePage> {
    need_tree(c, tree_id)?;
    need_node(c, tree_id, node)?;
    let (start, limit) = (window_start(after, node), page_limit(limit));
    let rows = c.query(
        CHILDREN_SQL,
        &[
            &tree_id,
            &to_db(node),
            &to_db(start),
            &(i64::from(limit) + 1),
        ],
    )?;
    Ok(keyset_page(nodes_from_rows(&rows)?, limit))
}

pub(crate) fn tally(
    c: &mut Client,
    rule: TreeQueryRule,
    tree_id: &str,
    node: u64,
) -> Result<Vec<VerdictCount>> {
    let tree = need_tree(c, tree_id)?;
    let at = need_node(c, tree_id, node)?;
    match form_for(rule, tree.node_count) {
        TreeQueryForm::Path => {
            let (lo, hi) = subtree_bounds(&at.path);
            read_tally(c, TALLY_PATH_SQL, &[&tree_id, &lo, &hi])
        }
        TreeQueryForm::Cte => read_tally(c, TALLY_CTE_SQL, &[&tree_id, &to_db(node)]),
    }
}

pub(crate) fn first_failing(
    c: &mut Client,
    rule: TreeQueryRule,
    tree_id: &str,
    node: u64,
) -> Result<Option<u64>> {
    let tree = need_tree(c, tree_id)?;
    let at = need_node(c, tree_id, node)?;
    let row = match form_for(rule, tree.node_count) {
        TreeQueryForm::Path => {
            let (lo, hi) = subtree_bounds(&at.path);
            ordered_scans(c, |tx| {
                Ok(tx.query_opt(FIRST_FAILING_PATH_SQL, &[&tree_id, &lo, &hi])?)
            })?
        }
        TreeQueryForm::Cte => c.query_opt(FIRST_FAILING_CTE_SQL, &[&tree_id, &to_db(node)])?,
    };
    Ok(row.map(|r| r.get::<_, i64>(0) as u64))
}

pub(crate) fn ancestors(c: &mut Client, tree_id: &str, node: u64) -> Result<Vec<NodeRecord>> {
    need_node(c, tree_id, node)?;
    let rows = c.query(ANCESTORS_SQL, &[&tree_id, &to_db(node)])?;
    nodes_from_rows(&rows)
}

/// The plan PostgreSQL chooses for a tree query in the given form, one line per step, planned with real bound values.
pub(crate) fn explain(
    c: &mut Client,
    query: TreeQuery,
    form: TreeQueryForm,
    tree_id: &str,
    node: u64,
) -> Result<Vec<String>> {
    use TreeQueryForm::{Cte, Path};
    let at = need_node(c, tree_id, node)?;
    let (lo, hi) = subtree_bounds(&at.path);
    let (start, end, limit, small) = (to_db(node), to_db(window_end(node)), 501i64, 500i64);
    let (sql, bound): (&str, Vec<Param>) = match (query, form) {
        (TreeQuery::SubtreeDescendants, Path) => (
            DESCENDANTS_PATH_SQL,
            vec![&tree_id, &start, &end, &lo, &hi, &small],
        ),
        (TreeQuery::SubtreeDescendants, Cte) => {
            (DESCENDANTS_CTE_SQL, vec![&tree_id, &start, &start, &limit])
        }
        (TreeQuery::TreeChildren, _) => (CHILDREN_SQL, vec![&tree_id, &start, &start, &limit]),
        (TreeQuery::SubtreeTally, Path) => (TALLY_PATH_SQL, vec![&tree_id, &lo, &hi]),
        (TreeQuery::SubtreeTally, Cte) => (TALLY_CTE_SQL, vec![&tree_id, &start]),
        (TreeQuery::SubtreeFirstFailing, Path) => {
            (FIRST_FAILING_PATH_SQL, vec![&tree_id, &lo, &hi])
        }
        (TreeQuery::SubtreeFirstFailing, Cte) => (FIRST_FAILING_CTE_SQL, vec![&tree_id, &start]),
        (TreeQuery::Ancestors, _) => (ANCESTORS_SQL, vec![&tree_id, &start]),
    };
    let explain_sql = format!("EXPLAIN (COSTS OFF) {sql}");
    let ordered = form == Path
        && matches!(
            query,
            TreeQuery::SubtreeDescendants | TreeQuery::SubtreeFirstFailing
        );
    let rows = if ordered {
        ordered_scans(c, |tx| Ok(tx.query(&explain_sql, &bound)?))?
    } else {
        c.query(&explain_sql, &bound)?
    };
    Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
}

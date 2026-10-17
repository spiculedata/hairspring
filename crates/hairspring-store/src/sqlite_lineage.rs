//! SQLite side of trial trees (schema version 3). Every statement binds its inputs as parameters; the path bounds are
//! computed here from the stored path and never taken from a caller. The decisions (caps, identity rules, paging) are
//! made in [`crate::lineage`], so both stores apply the same ones.
use crate::lineage::*;
use crate::trials::{to_db, TrialVerdict};
use crate::types::{Result, StoreError};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};

const NODE_COLUMNS: &str = node_columns!("");

fn conversion(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        Type::Text,
        Box::new(StoreError::Backend(format!("unknown {column} `{value}`"))),
    )
}

fn parse_column<T>(r: &Row<'_>, name: &str, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    let text: String = r.get(name)?;
    parse(&text).ok_or_else(|| conversion(name, &text))
}

pub(crate) fn node_from_row(r: &Row<'_>) -> rusqlite::Result<NodeRecord> {
    Ok(NodeRecord {
        tree_id: r.get("tree_id")?,
        node: r.get::<_, i64>("node")? as u64,
        run_id: r.get("run_id")?,
        seq: r.get::<_, i64>("seq")? as u64,
        parent_node: r.get::<_, Option<i64>>("parent_node")?.map(|p| p as u64),
        depth: r.get::<_, i64>("depth")? as u32,
        path: r.get("path")?,
        verdict: parse_column(r, "verdict", TrialVerdict::parse)?,
        operator: parse_column(r, "operator", Operator::parse)?,
        cut_kind: parse_column(r, "cut_kind", CutKind::parse)?,
        cut_at: r.get::<_, i64>("cut_at")? as u64,
        suffix_seed: r.get::<_, Option<i64>>("suffix_seed")?.map(|s| s as u64),
        edge_ref: r.get("edge_ref")?,
        origin: parse_column(r, "origin", NodeOrigin::parse)?,
        reporter_id: r.get("reporter_id")?,
        reporter_trusted: r.get("reporter_trusted")?,
        verified: parse_column(r, "verified", Verified::parse)?,
        created_at: r.get("created_at")?,
    })
}

fn tree_from_row(r: &Row<'_>) -> rusqlite::Result<TreeRecord> {
    Ok(TreeRecord {
        id: r.get("id")?,
        backend: r.get("backend")?,
        target: r.get("target")?,
        identity_hash: r.get("identity_hash")?,
        node_count: r.get::<_, i64>("node_count")? as u64,
        run_count: r.get::<_, i64>("run_count")? as u64,
        created_at: r.get("created_at")?,
    })
}

fn branch_from_row(r: &Row<'_>) -> rusqlite::Result<BranchRecord> {
    Ok(BranchRecord {
        run_id: r.get("run_id")?,
        tree_id: r.get("tree_id")?,
        source_node: r.get::<_, i64>("source_node")? as u64,
        cut_kind: parse_column(r, "cut_kind", CutKind::parse)?,
        cut_at: r.get::<_, i64>("cut_at")? as u64,
        point_hash: r.get("point_hash")?,
        command_json: r.get("command_json")?,
        command_hash: r.get("command_hash")?,
        seed_source: parse_column(r, "seed_source", SeedSource::parse)?,
        verify_runs: r.get::<_, i64>("verify_runs")? as u64,
        verify_matches: r.get::<_, i64>("verify_matches")? as u64,
        created_by: r.get("created_by")?,
        created_at: r.get("created_at")?,
    })
}

fn identity_from_row(r: &Row<'_>) -> rusqlite::Result<RunIdentity> {
    Ok(RunIdentity {
        run_id: r.get("run_id")?,
        scheme: r.get("scheme")?,
        identity_text: r.get("identity_text")?,
        identity_hash: r.get("identity_hash")?,
        complete: r.get("complete")?,
        reporter_id: r.get("reporter_id")?,
        reporter_trusted: r.get("reporter_trusted")?,
    })
}

fn write_tx(conn: &mut Connection) -> Result<Transaction<'_>> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

fn run_exists(conn: &Connection, run_id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM runs WHERE id=?", [run_id], |_| Ok(()))
        .optional()?
        .is_some())
}

fn run_finished(conn: &Connection, run_id: &str) -> Result<bool> {
    let state: Option<String> = conn
        .query_row("SELECT state FROM runs WHERE id=?", [run_id], |r| r.get(0))
        .optional()?;
    Ok(state
        .as_deref()
        .and_then(crate::types::RunState::parse)
        .is_some_and(|s| s.is_terminal()))
}

// ---- run identity ------------------------------------------------------------------------------------------------

pub(crate) fn get_identity(conn: &Connection, run_id: &str) -> Result<Option<RunIdentity>> {
    Ok(conn
        .query_row(
            "SELECT run_id, scheme, identity_text, identity_hash, complete, reporter_id, reporter_trusted \
             FROM run_identity WHERE run_id=?",
            [run_id],
            identity_from_row,
        )
        .optional()?)
}

pub(crate) fn put_identity(conn: &mut Connection, id: &RunIdentity) -> Result<IdentityOutcome> {
    if let Err(why) = check_identity(id) {
        return Ok(IdentityOutcome::Rejected(why));
    }
    let tx = write_tx(conn)?;
    if !run_exists(&tx, &id.run_id)? {
        return Ok(IdentityOutcome::Rejected(TreeRejection::RunNotFound));
    }
    if let Some(stored) = get_identity(&tx, &id.run_id)? {
        if same_identity(&stored, id) {
            return Ok(IdentityOutcome::Duplicate);
        }
        let Some(merged) = completed(&stored, id) else {
            return Ok(IdentityOutcome::Rejected(TreeRejection::Conflict));
        };
        if tree_of_run(&tx, &id.run_id)?.is_some() {
            return Ok(IdentityOutcome::Rejected(TreeRejection::Conflict));
        }
        tx.execute(
            "UPDATE run_identity SET scheme=?, identity_text=?, identity_hash=?, complete=?, reporter_id=?, reporter_trusted=? \
             WHERE run_id=?",
            params![merged.scheme, merged.identity_text, merged.identity_hash, merged.complete, merged.reporter_id, merged.reporter_trusted, merged.run_id],
        )?;
        tx.commit()?;
        return Ok(IdentityOutcome::Completed);
    }
    tx.execute(
        "INSERT INTO run_identity(run_id,scheme,identity_text,identity_hash,complete,reporter_id,reporter_trusted) \
         VALUES (?,?,?,?,?,?,?)",
        params![id.run_id, id.scheme, id.identity_text, id.identity_hash, id.complete, id.reporter_id, id.reporter_trusted],
    )?;
    tx.commit()?;
    Ok(IdentityOutcome::Stored)
}

// ---- trees -------------------------------------------------------------------------------------------------------

pub(crate) fn get_tree(conn: &Connection, id: &str) -> Result<Option<TreeRecord>> {
    Ok(conn
        .query_row(
            "SELECT id, backend, target, identity_hash, node_count, run_count, created_at FROM trees WHERE id=?",
            [id],
            tree_from_row,
        )
        .optional()?)
}

pub(crate) fn tree_of_run(conn: &Connection, run_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM trees WHERE id=?1 UNION ALL SELECT tree_id FROM tree_branches WHERE run_id=?1 LIMIT 1",
            [run_id],
            |r| r.get(0),
        )
        .optional()?)
}

pub(crate) fn create_tree(conn: &mut Connection, new: &NewTree, now: i64) -> Result<TreeOutcome> {
    let tx = write_tx(conn)?;
    let run: Option<(String, String)> = tx
        .query_row(
            "SELECT backend, target FROM runs WHERE id=?",
            [&new.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((backend, target)) = run else {
        return Ok(TreeOutcome::Rejected(TreeRejection::RunNotFound));
    };
    if let Some(stored) = get_tree(&tx, &new.run_id)? {
        let same = (&stored.backend, &stored.target, &stored.identity_hash)
            == (&new.backend, &new.target, &new.identity_hash);
        return Ok(if same {
            TreeOutcome::Existing(stored)
        } else {
            TreeOutcome::Rejected(TreeRejection::Conflict)
        });
    }
    if get_branch(&tx, &new.run_id)?.is_some() {
        return Ok(TreeOutcome::Rejected(TreeRejection::Invalid(
            "a branch run cannot root a tree",
        )));
    }
    if backend != new.backend || target != new.target {
        return Ok(TreeOutcome::Rejected(TreeRejection::Invalid(
            "the tree's backend and target are the run's",
        )));
    }
    let identity = get_identity(&tx, &new.run_id)?;
    if let Err(why) = check_tree_identity(new.identity_hash.as_deref(), identity.as_ref()) {
        return Ok(TreeOutcome::Rejected(why));
    }
    tx.execute(
        "INSERT INTO trees(id,backend,target,identity_hash,node_count,run_count,created_at) VALUES (?,?,?,?,0,1,?)",
        params![new.run_id, new.backend, new.target, new.identity_hash, now],
    )?;
    tx.commit()?;
    Ok(TreeOutcome::Created(TreeRecord {
        id: new.run_id.clone(),
        backend: new.backend.clone(),
        target: new.target.clone(),
        identity_hash: new.identity_hash.clone(),
        node_count: 0,
        run_count: 1,
        created_at: now,
    }))
}

// ---- nodes -------------------------------------------------------------------------------------------------------

pub(crate) fn get_node(conn: &Connection, tree_id: &str, node: u64) -> Result<Option<NodeRecord>> {
    Ok(conn
        .query_row(
            &format!("SELECT {NODE_COLUMNS} FROM trial_lineage WHERE tree_id=? AND node=?"),
            params![tree_id, to_db(node)],
            node_from_row,
        )
        .optional()?)
}

pub(crate) fn node_of_trial(
    conn: &Connection,
    run_id: &str,
    seq: u64,
) -> Result<Option<NodeRecord>> {
    Ok(conn
        .query_row(
            &format!("SELECT {NODE_COLUMNS} FROM trial_lineage WHERE run_id=? AND seq=?"),
            params![run_id, to_db(seq)],
            node_from_row,
        )
        .optional()?)
}

fn run_belongs(tx: &Transaction, tree_id: &str, run_id: &str) -> Result<bool> {
    if tree_id == run_id {
        return Ok(true);
    }
    Ok(tx
        .query_row(
            "SELECT 1 FROM tree_branches WHERE tree_id=? AND run_id=?",
            [tree_id, run_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn parent_facts(tx: &Transaction, tree_id: &str, parent: u64) -> Result<Option<ParentFacts>> {
    Ok(tx
        .query_row(
            "SELECT depth, path FROM trial_lineage WHERE tree_id=? AND node=?",
            params![tree_id, to_db(parent)],
            |r| {
                Ok(ParentFacts {
                    depth: r.get::<_, i64>(0)? as u32,
                    path: r.get(1)?,
                })
            },
        )
        .optional()?)
}

fn siblings_at_cut(tx: &Transaction, tree_id: &str, new: &NewNode) -> Result<u64> {
    let Some(parent) = new.parent_node else {
        return Ok(0);
    };
    let n: i64 = tx.query_row(
        "SELECT COUNT(*) FROM trial_lineage WHERE tree_id=? AND parent_node=? AND cut_kind=? AND cut_at=?",
        params![tree_id, to_db(parent), new.cut_kind.as_str(), to_db(new.cut_at)],
        |r| r.get(0),
    )?;
    Ok(n as u64)
}

pub(crate) fn add_node(
    conn: &mut Connection,
    tree_id: &str,
    new: &NewNode,
    now: i64,
) -> Result<NodeOutcome> {
    use NodeOutcome::Rejected;
    let tx = write_tx(conn)?;
    let Some(tree) = get_tree(&tx, tree_id)? else {
        return Ok(Rejected(TreeRejection::TreeNotFound));
    };
    if let Err(why) = check_node_shape(new) {
        return Ok(Rejected(why));
    }
    if !run_belongs(&tx, tree_id, &new.run_id)? {
        return Ok(Rejected(TreeRejection::RunNotInTree));
    }
    let verdict: Option<String> = tx
        .query_row(
            "SELECT verdict FROM trials WHERE run_id=? AND seq=?",
            params![new.run_id, to_db(new.seq)],
            |r| r.get(0),
        )
        .optional()?;
    let Some(verdict) = verdict.as_deref().and_then(TrialVerdict::parse) else {
        return Ok(Rejected(TreeRejection::TrialNotFound));
    };
    if node_of_trial(&tx, &new.run_id, new.seq)?.is_some() {
        return Ok(Rejected(TreeRejection::TrialAlreadyInTree));
    }
    let parent = match new.parent_node {
        Some(p) => parent_facts(&tx, tree_id, p)?,
        None => None,
    };
    let siblings = siblings_at_cut(&tx, tree_id, new)?;
    let placed = match place(tree.node_count, new, parent.as_ref(), siblings) {
        Ok(p) => p,
        Err(why) => return Ok(Rejected(why)),
    };
    tx.execute(
        "INSERT INTO trial_lineage(tree_id,node,run_id,seq,parent_node,depth,path,verdict,operator,cut_kind,cut_at,\
           suffix_seed,edge_ref,origin,reporter_id,reporter_trusted,verified,created_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,'no',?)",
        params![
            tree_id, to_db(placed.node), new.run_id, to_db(new.seq), new.parent_node.map(to_db), placed.depth,
            placed.path, verdict.as_str(), new.operator.as_str(), new.cut_kind.as_str(), to_db(new.cut_at),
            new.suffix_seed.map(|s| s as i64), new.edge_ref, new.origin.as_str(), new.reporter_id,
            new.reporter_trusted, now,
        ],
    )?;
    tx.execute(
        "UPDATE trees SET node_count=? WHERE id=?",
        params![to_db(placed.node + 1), tree_id],
    )?;
    tx.commit()?;
    Ok(NodeOutcome::Added(NodeRecord {
        tree_id: tree_id.to_string(),
        node: placed.node,
        run_id: new.run_id.clone(),
        seq: new.seq,
        parent_node: new.parent_node,
        depth: placed.depth,
        path: placed.path,
        verdict,
        operator: new.operator,
        cut_kind: new.cut_kind,
        cut_at: new.cut_at,
        suffix_seed: new.suffix_seed,
        edge_ref: new.edge_ref.clone(),
        origin: new.origin,
        reporter_id: new.reporter_id.clone(),
        reporter_trusted: new.reporter_trusted,
        verified: Verified::No,
        created_at: now,
    }))
}

pub(crate) fn set_verified(
    conn: &Connection,
    tree_id: &str,
    node: u64,
    verified: Verified,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE trial_lineage SET verified=? WHERE tree_id=? AND node=?",
        params![verified.as_str(), tree_id, to_db(node)],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!(
            "node {node} of tree {tree_id}"
        )));
    }
    Ok(())
}

// ---- branch runs -------------------------------------------------------------------------------------------------

const BRANCH_COLUMNS: &str =
    "run_id, tree_id, source_node, cut_kind, cut_at, point_hash, command_json, \
     command_hash, seed_source, verify_runs, verify_matches, created_by, created_at";

pub(crate) fn get_branch(conn: &Connection, run_id: &str) -> Result<Option<BranchRecord>> {
    Ok(conn
        .query_row(
            &format!("SELECT {BRANCH_COLUMNS} FROM tree_branches WHERE run_id=?"),
            [run_id],
            branch_from_row,
        )
        .optional()?)
}

pub(crate) fn find_branch(
    conn: &Connection,
    tree_id: &str,
    command_hash: &str,
) -> Result<Option<BranchRecord>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {BRANCH_COLUMNS} FROM tree_branches WHERE tree_id=? AND command_hash=?"
            ),
            [tree_id, command_hash],
            branch_from_row,
        )
        .optional()?)
}

pub(crate) fn add_branch(
    conn: &mut Connection,
    new: &NewBranch,
    now: i64,
) -> Result<BranchOutcome> {
    use BranchOutcome::Rejected;
    if let Err(why) = check_branch_shape(new) {
        return Ok(Rejected(why));
    }
    let tx = write_tx(conn)?;
    let Some(tree) = get_tree(&tx, &new.tree_id)? else {
        return Ok(Rejected(TreeRejection::TreeNotFound));
    };
    if let Some(stored) = find_branch(&tx, &new.tree_id, &new.command_hash)? {
        return Ok(BranchOutcome::Existing(stored));
    }
    if !run_exists(&tx, &new.run_id)? {
        return Ok(Rejected(TreeRejection::RunNotFound));
    }
    if new.run_id == tree.id {
        return Ok(Rejected(TreeRejection::Invalid(
            "the root run cannot be a branch",
        )));
    }
    if get_tree(&tx, &new.run_id)?.is_some() {
        return Ok(Rejected(TreeRejection::Invalid(
            "a run that roots a tree cannot be a branch",
        )));
    }
    if get_branch(&tx, &new.run_id)?.is_some() {
        return Ok(Rejected(TreeRejection::Conflict));
    }
    let identity = get_identity(&tx, &new.run_id)?;
    if let Err(why) = check_branch_identity(tree.identity_hash.as_deref(), identity.as_ref()) {
        return Ok(Rejected(why));
    }
    let Some(source) = get_node(&tx, &new.tree_id, new.source_node)? else {
        return Ok(Rejected(TreeRejection::SourceNotFound));
    };
    // Finished is final: a finished run never starts another attempt, so its nodes (and the source node) stay.
    if !run_finished(&tx, &tree.id)? || !run_finished(&tx, &source.run_id)? {
        return Ok(Rejected(TreeRejection::RunNotFinished));
    }
    let branches: i64 = tx.query_row(
        "SELECT COUNT(*) FROM tree_branches WHERE tree_id=?",
        [&new.tree_id],
        |r| r.get(0),
    )?;
    if branches as u64 >= MAX_BRANCH_RUNS {
        return Ok(Rejected(TreeRejection::BranchCap));
    }
    tx.execute(
        &format!("INSERT INTO tree_branches({BRANCH_COLUMNS}) VALUES (?,?,?,?,?,?,?,?,?,0,0,?,?)"),
        params![
            new.run_id,
            new.tree_id,
            to_db(new.source_node),
            new.cut_kind.as_str(),
            to_db(new.cut_at),
            new.point_hash,
            new.command_json,
            new.command_hash,
            new.seed_source.as_str(),
            new.created_by,
            now,
        ],
    )?;
    tx.execute(
        "UPDATE trees SET run_count=run_count+1 WHERE id=?",
        [&new.tree_id],
    )?;
    let stored = get_branch(&tx, &new.run_id)?.ok_or_else(|| {
        StoreError::Backend("a branch row vanished inside its transaction".into())
    })?;
    tx.commit()?;
    Ok(BranchOutcome::Added(stored))
}

pub(crate) fn record_verify(conn: &Connection, run_id: &str, matched: bool) -> Result<()> {
    let n = conn.execute(
        "UPDATE tree_branches SET verify_runs=verify_runs+1, verify_matches=verify_matches+? WHERE run_id=?",
        params![i64::from(matched), run_id],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("branch run {run_id}")));
    }
    Ok(())
}

// ---- a new attempt of a run's job --------------------------------------------------------------------------------

/// Why the run's rows may not be started over, if something depends on them: branch runs of the tree it roots, other
/// runs' nodes below its nodes, or branches that start at its nodes.
pub(crate) fn attempt_blocked(tx: &Transaction, run_id: &str) -> Result<Option<String>> {
    let blocked: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM tree_branches WHERE tree_id=?1) \
           OR EXISTS (SELECT 1 FROM trial_lineage c JOIN trial_lineage p ON p.tree_id=c.tree_id AND p.node=c.parent_node \
                       WHERE p.run_id=?1 AND c.run_id<>?1) \
           OR EXISTS (SELECT 1 FROM tree_branches b JOIN trial_lineage n ON n.tree_id=b.tree_id AND n.node=b.source_node \
                       WHERE n.run_id=?1)",
        [run_id],
        |r| r.get(0),
    )?;
    Ok(blocked.then(|| {
        "this run roots a tree with branch runs, or other runs build on its nodes; it cannot start a new attempt"
            .to_string()
    }))
}

/// Deletes the run's nodes (they refer to its trial rows, which are deleted next) and returns the edge blobs they
/// named. `trees.node_count` is never lowered: it counts the numbers handed out, so a deleted node's number is never
/// given to another node and the gap stays.
pub(crate) fn drop_run_lineage(tx: &Transaction, run_id: &str) -> Result<Vec<String>> {
    let mut stmt =
        tx.prepare("SELECT edge_ref FROM trial_lineage WHERE run_id=? AND edge_ref IS NOT NULL")?;
    let edges = stmt
        .query_map([run_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    tx.execute("DELETE FROM trial_lineage WHERE run_id=?", [run_id])?;
    Ok(edges)
}

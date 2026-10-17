//! PostgreSQL side of trial trees (schema version 3). Every statement binds its inputs as parameters; the path bounds
//! are computed here from the stored path and never taken from a caller. The decisions (caps, identity rules, paging)
//! are made in [`crate::lineage`], so both stores apply the same ones. Writers to one tree are serialised by a lock on
//! the tree's row.
use crate::lineage::*;
use crate::trials::{to_db, TrialVerdict};
use crate::types::{Result, StoreError};
use postgres::{GenericClient, Row};

const NODE_COLUMNS: &str = node_columns!("");

/// A stored value outside its enum means something else wrote the database; report it.
fn checked<T>(what: &str, text: &str, parse: fn(&str) -> Option<T>) -> Result<T> {
    parse(text)
        .ok_or_else(|| StoreError::Backend(format!("unknown {what} `{text}` in the database")))
}

fn node_from_row(r: &Row) -> Result<NodeRecord> {
    Ok(NodeRecord {
        tree_id: r.get("tree_id"),
        node: r.get::<_, i64>("node") as u64,
        run_id: r.get("run_id"),
        seq: r.get::<_, i64>("seq") as u64,
        parent_node: r.get::<_, Option<i64>>("parent_node").map(|p| p as u64),
        depth: r.get::<_, i32>("depth") as u32,
        path: r.get("path"),
        verdict: checked(
            "verdict",
            &r.get::<_, String>("verdict"),
            TrialVerdict::parse,
        )?,
        operator: checked("operator", &r.get::<_, String>("operator"), Operator::parse)?,
        cut_kind: checked("cut kind", &r.get::<_, String>("cut_kind"), CutKind::parse)?,
        cut_at: r.get::<_, i64>("cut_at") as u64,
        suffix_seed: r.get::<_, Option<i64>>("suffix_seed").map(|s| s as u64),
        edge_ref: r.get("edge_ref"),
        origin: checked("origin", &r.get::<_, String>("origin"), NodeOrigin::parse)?,
        reporter_id: r.get("reporter_id"),
        reporter_trusted: r.get("reporter_trusted"),
        verified: checked(
            "verification state",
            &r.get::<_, String>("verified"),
            Verified::parse,
        )?,
        created_at: r.get("created_at"),
    })
}

pub(crate) fn nodes_from_rows(rows: &[Row]) -> Result<Vec<NodeRecord>> {
    rows.iter().map(node_from_row).collect()
}

fn tree_from_row(r: &Row) -> TreeRecord {
    TreeRecord {
        id: r.get("id"),
        backend: r.get("backend"),
        target: r.get("target"),
        identity_hash: r.get("identity_hash"),
        node_count: r.get::<_, i64>("node_count") as u64,
        run_count: r.get::<_, i64>("run_count") as u64,
        created_at: r.get("created_at"),
    }
}

fn branch_from_row(r: &Row) -> Result<BranchRecord> {
    Ok(BranchRecord {
        run_id: r.get("run_id"),
        tree_id: r.get("tree_id"),
        source_node: r.get::<_, i64>("source_node") as u64,
        cut_kind: checked("cut kind", &r.get::<_, String>("cut_kind"), CutKind::parse)?,
        cut_at: r.get::<_, i64>("cut_at") as u64,
        point_hash: r.get("point_hash"),
        command_json: r.get("command_json"),
        command_hash: r.get("command_hash"),
        seed_source: checked(
            "seed source",
            &r.get::<_, String>("seed_source"),
            SeedSource::parse,
        )?,
        verify_runs: r.get::<_, i64>("verify_runs") as u64,
        verify_matches: r.get::<_, i64>("verify_matches") as u64,
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
    })
}

fn identity_from_row(r: &Row) -> RunIdentity {
    RunIdentity {
        run_id: r.get("run_id"),
        scheme: r.get("scheme"),
        identity_text: r.get("identity_text"),
        identity_hash: r.get("identity_hash"),
        complete: r.get("complete"),
        reporter_id: r.get("reporter_id"),
        reporter_trusted: r.get("reporter_trusted"),
    }
}

fn run_exists<C: GenericClient>(c: &mut C, run_id: &str) -> Result<bool> {
    Ok(c.query_opt("SELECT 1 FROM runs WHERE id=$1", &[&run_id])?
        .is_some())
}

/// Whether the run is in a final state; the row is share-locked, so a run being finished or restarted at the same
/// time is waited for.
fn run_finished<C: GenericClient>(c: &mut C, run_id: &str) -> Result<bool> {
    let row = c.query_opt("SELECT state FROM runs WHERE id=$1 FOR SHARE", &[&run_id])?;
    Ok(row
        .and_then(|r| crate::types::RunState::parse(&r.get::<_, String>(0)))
        .is_some_and(|s| s.is_terminal()))
}

// ---- run identity ------------------------------------------------------------------------------------------------

pub(crate) fn get_identity<C: GenericClient>(
    c: &mut C,
    run_id: &str,
) -> Result<Option<RunIdentity>> {
    Ok(c.query_opt(
        "SELECT run_id, scheme, identity_text, identity_hash, complete, reporter_id, reporter_trusted \
         FROM run_identity WHERE run_id=$1",
        &[&run_id],
    )?
    .map(|r| identity_from_row(&r)))
}

pub(crate) fn put_identity(c: &mut postgres::Client, id: &RunIdentity) -> Result<IdentityOutcome> {
    if let Err(why) = check_identity(id) {
        return Ok(IdentityOutcome::Rejected(why));
    }
    let mut tx = c.transaction()?;
    if !run_exists(&mut tx, &id.run_id)? {
        return Ok(IdentityOutcome::Rejected(TreeRejection::RunNotFound));
    }
    let inserted = tx.execute(
        "INSERT INTO run_identity(run_id,scheme,identity_text,identity_hash,complete,reporter_id,reporter_trusted) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (run_id) DO NOTHING",
        &[&id.run_id, &id.scheme, &id.identity_text, &id.identity_hash, &id.complete, &id.reporter_id, &id.reporter_trusted],
    )?;
    if inserted == 1 {
        tx.commit()?;
        return Ok(IdentityOutcome::Stored);
    }
    let stored = get_identity(&mut tx, &id.run_id)?.ok_or_else(|| {
        StoreError::Backend("a run identity vanished inside its transaction".into())
    })?;
    if same_identity(&stored, id) {
        return Ok(IdentityOutcome::Duplicate);
    }
    if !completes(&stored, id) {
        return Ok(IdentityOutcome::Rejected(TreeRejection::Conflict));
    }
    complete_identity(tx, id)
}

/// Replaces an incomplete identity with the complete one offered, unless a tree depends on the run. The run row is
/// locked first, as `create_tree` locks it, so a tree being created at the same time is waited for and then seen.
fn complete_identity(
    mut tx: postgres::Transaction<'_>,
    id: &RunIdentity,
) -> Result<IdentityOutcome> {
    tx.query_opt("SELECT 1 FROM runs WHERE id=$1 FOR UPDATE", &[&id.run_id])?;
    // Read again under the lock: another writer may have completed the identity since it was first read.
    let stored = get_identity(&mut tx, &id.run_id)?.ok_or_else(|| {
        StoreError::Backend("a run identity vanished inside its transaction".into())
    })?;
    if same_identity(&stored, id) {
        return Ok(IdentityOutcome::Duplicate);
    }
    if !completes(&stored, id) || tree_of_run(&mut tx, &id.run_id)?.is_some() {
        return Ok(IdentityOutcome::Rejected(TreeRejection::Conflict));
    }
    tx.execute(
        "UPDATE run_identity SET scheme=$2, identity_text=$3, identity_hash=$4, complete=$5, reporter_id=$6, \
         reporter_trusted=$7 WHERE run_id=$1",
        &[&id.run_id, &id.scheme, &id.identity_text, &id.identity_hash, &id.complete, &id.reporter_id, &id.reporter_trusted],
    )?;
    tx.commit()?;
    Ok(IdentityOutcome::Completed)
}

// ---- trees -------------------------------------------------------------------------------------------------------

pub(crate) fn get_tree<C: GenericClient>(c: &mut C, id: &str) -> Result<Option<TreeRecord>> {
    Ok(c.query_opt(
        "SELECT id, backend, target, identity_hash, node_count, run_count, created_at FROM trees WHERE id=$1",
        &[&id],
    )?
    .map(|r| tree_from_row(&r)))
}

pub(crate) fn tree_of_run<C: GenericClient>(c: &mut C, run_id: &str) -> Result<Option<String>> {
    Ok(c.query_opt(
        "SELECT id FROM trees WHERE id=$1 UNION ALL SELECT tree_id FROM tree_branches WHERE run_id=$1 LIMIT 1",
        &[&run_id],
    )?
    .map(|r| r.get(0)))
}

fn same_tree(stored: &TreeRecord, new: &NewTree) -> bool {
    (&stored.backend, &stored.target, &stored.identity_hash)
        == (&new.backend, &new.target, &new.identity_hash)
}

pub(crate) fn create_tree(
    c: &mut postgres::Client,
    new: &NewTree,
    now: i64,
) -> Result<TreeOutcome> {
    let mut tx = c.transaction()?;
    // Share-locked: completing the run's identity waits for this tree, and this waits for a completion in progress.
    let run = tx.query_opt(
        "SELECT backend, target FROM runs WHERE id=$1 FOR SHARE",
        &[&new.run_id],
    )?;
    let Some(run) = run else {
        return Ok(TreeOutcome::Rejected(TreeRejection::RunNotFound));
    };
    let (backend, target): (String, String) = (run.get(0), run.get(1));
    if let Some(stored) = get_tree(&mut tx, &new.run_id)? {
        return Ok(if same_tree(&stored, new) {
            TreeOutcome::Existing(stored)
        } else {
            TreeOutcome::Rejected(TreeRejection::Conflict)
        });
    }
    if get_branch(&mut tx, &new.run_id)?.is_some() {
        return Ok(TreeOutcome::Rejected(TreeRejection::Invalid(
            "a branch run cannot root a tree",
        )));
    }
    if backend != new.backend || target != new.target {
        return Ok(TreeOutcome::Rejected(TreeRejection::Invalid(
            "the tree's backend and target are the run's",
        )));
    }
    let identity = get_identity(&mut tx, &new.run_id)?;
    if let Err(why) = check_tree_identity(new.identity_hash.as_deref(), identity.as_ref()) {
        return Ok(TreeOutcome::Rejected(why));
    }
    let inserted = tx.execute(
        "INSERT INTO trees(id,backend,target,identity_hash,node_count,run_count,created_at) \
         VALUES ($1,$2,$3,$4,0,1,$5) ON CONFLICT (id) DO NOTHING",
        &[
            &new.run_id,
            &new.backend,
            &new.target,
            &new.identity_hash,
            &now,
        ],
    )?;
    if inserted == 0 {
        // A concurrent creator won; answer as the sequential case would.
        let stored = get_tree(&mut tx, &new.run_id)?
            .ok_or_else(|| StoreError::Backend("a tree vanished inside its transaction".into()))?;
        return Ok(if same_tree(&stored, new) {
            TreeOutcome::Existing(stored)
        } else {
            TreeOutcome::Rejected(TreeRejection::Conflict)
        });
    }
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

pub(crate) fn get_node<C: GenericClient>(
    c: &mut C,
    tree_id: &str,
    node: u64,
) -> Result<Option<NodeRecord>> {
    c.query_opt(
        &format!("SELECT {NODE_COLUMNS} FROM trial_lineage WHERE tree_id=$1 AND node=$2"),
        &[&tree_id, &to_db(node)],
    )?
    .map(|r| node_from_row(&r))
    .transpose()
}

pub(crate) fn node_of_trial<C: GenericClient>(
    c: &mut C,
    run_id: &str,
    seq: u64,
) -> Result<Option<NodeRecord>> {
    c.query_opt(
        &format!("SELECT {NODE_COLUMNS} FROM trial_lineage WHERE run_id=$1 AND seq=$2"),
        &[&run_id, &to_db(seq)],
    )?
    .map(|r| node_from_row(&r))
    .transpose()
}

fn run_belongs<C: GenericClient>(c: &mut C, tree_id: &str, run_id: &str) -> Result<bool> {
    if tree_id == run_id {
        return Ok(true);
    }
    Ok(c.query_opt(
        "SELECT 1 FROM tree_branches WHERE tree_id=$1 AND run_id=$2",
        &[&tree_id, &run_id],
    )?
    .is_some())
}

fn siblings_at_cut<C: GenericClient>(c: &mut C, tree_id: &str, new: &NewNode) -> Result<u64> {
    let Some(parent) = new.parent_node else {
        return Ok(0);
    };
    let n: i64 = c
        .query_one(
            "SELECT COUNT(*) FROM trial_lineage WHERE tree_id=$1 AND parent_node=$2 AND cut_kind=$3 AND cut_at=$4",
            &[&tree_id, &to_db(parent), &new.cut_kind.as_str(), &to_db(new.cut_at)],
        )?
        .get(0);
    Ok(n as u64)
}

pub(crate) fn add_node(
    c: &mut postgres::Client,
    tree_id: &str,
    new: &NewNode,
    now: i64,
) -> Result<NodeOutcome> {
    use NodeOutcome::Rejected;
    let mut tx = c.transaction()?;
    // The tree's row is the write lock: node numbers are handed out one writer at a time.
    let locked = tx.query_opt(
        "SELECT id, backend, target, identity_hash, node_count, run_count, created_at FROM trees WHERE id=$1 FOR UPDATE",
        &[&tree_id],
    )?;
    let Some(tree) = locked.map(|r| tree_from_row(&r)) else {
        return Ok(Rejected(TreeRejection::TreeNotFound));
    };
    if let Err(why) = check_node_shape(new) {
        return Ok(Rejected(why));
    }
    if !run_belongs(&mut tx, tree_id, &new.run_id)? {
        return Ok(Rejected(TreeRejection::RunNotInTree));
    }
    let verdict = tx.query_opt(
        "SELECT verdict FROM trials WHERE run_id=$1 AND seq=$2",
        &[&new.run_id, &to_db(new.seq)],
    )?;
    let Some(verdict) = verdict.and_then(|r| TrialVerdict::parse(&r.get::<_, String>(0))) else {
        return Ok(Rejected(TreeRejection::TrialNotFound));
    };
    if node_of_trial(&mut tx, &new.run_id, new.seq)?.is_some() {
        return Ok(Rejected(TreeRejection::TrialAlreadyInTree));
    }
    let parent = match new.parent_node {
        Some(p) => tx
            .query_opt(
                "SELECT depth, path FROM trial_lineage WHERE tree_id=$1 AND node=$2",
                &[&tree_id, &to_db(p)],
            )?
            .map(|r| ParentFacts {
                depth: r.get::<_, i32>(0) as u32,
                path: r.get(1),
            }),
        None => None,
    };
    let siblings = siblings_at_cut(&mut tx, tree_id, new)?;
    let placed = match place(tree.node_count, new, parent.as_ref(), siblings) {
        Ok(p) => p,
        Err(why) => return Ok(Rejected(why)),
    };
    tx.execute(
        "INSERT INTO trial_lineage(tree_id,node,run_id,seq,parent_node,depth,path,verdict,operator,cut_kind,cut_at,\
           suffix_seed,edge_ref,origin,reporter_id,reporter_trusted,verified,created_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'no',$17)",
        &[
            &tree_id, &to_db(placed.node), &new.run_id, &to_db(new.seq), &new.parent_node.map(to_db),
            &(placed.depth as i32), &placed.path, &verdict.as_str(), &new.operator.as_str(),
            &new.cut_kind.as_str(), &to_db(new.cut_at), &new.suffix_seed.map(|s| s as i64), &new.edge_ref,
            &new.origin.as_str(), &new.reporter_id, &new.reporter_trusted, &now,
        ],
    )?;
    tx.execute(
        "UPDATE trees SET node_count=$1 WHERE id=$2",
        &[&to_db(placed.node + 1), &tree_id],
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
    c: &mut postgres::Client,
    tree_id: &str,
    node: u64,
    verified: Verified,
) -> Result<()> {
    let n = c.execute(
        "UPDATE trial_lineage SET verified=$1 WHERE tree_id=$2 AND node=$3",
        &[&verified.as_str(), &tree_id, &to_db(node)],
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

pub(crate) fn get_branch<C: GenericClient>(
    c: &mut C,
    run_id: &str,
) -> Result<Option<BranchRecord>> {
    c.query_opt(
        &format!("SELECT {BRANCH_COLUMNS} FROM tree_branches WHERE run_id=$1"),
        &[&run_id],
    )?
    .map(|r| branch_from_row(&r))
    .transpose()
}

pub(crate) fn find_branch<C: GenericClient>(
    c: &mut C,
    tree_id: &str,
    command_hash: &str,
) -> Result<Option<BranchRecord>> {
    c.query_opt(
        &format!("SELECT {BRANCH_COLUMNS} FROM tree_branches WHERE tree_id=$1 AND command_hash=$2"),
        &[&tree_id, &command_hash],
    )?
    .map(|r| branch_from_row(&r))
    .transpose()
}

pub(crate) fn add_branch(
    c: &mut postgres::Client,
    new: &NewBranch,
    now: i64,
) -> Result<BranchOutcome> {
    use BranchOutcome::Rejected;
    if let Err(why) = check_branch_shape(new) {
        return Ok(Rejected(why));
    }
    let mut tx = c.transaction()?;
    let locked = tx.query_opt(
        "SELECT id, backend, target, identity_hash, node_count, run_count, created_at FROM trees WHERE id=$1 FOR UPDATE",
        &[&new.tree_id],
    )?;
    let Some(tree) = locked.map(|r| tree_from_row(&r)) else {
        return Ok(Rejected(TreeRejection::TreeNotFound));
    };
    if let Some(stored) = find_branch(&mut tx, &new.tree_id, &new.command_hash)? {
        return Ok(BranchOutcome::Existing(stored));
    }
    if !run_exists(&mut tx, &new.run_id)? {
        return Ok(Rejected(TreeRejection::RunNotFound));
    }
    if new.run_id == tree.id {
        return Ok(Rejected(TreeRejection::Invalid(
            "the root run cannot be a branch",
        )));
    }
    if get_tree(&mut tx, &new.run_id)?.is_some() {
        return Ok(Rejected(TreeRejection::Invalid(
            "a run that roots a tree cannot be a branch",
        )));
    }
    if get_branch(&mut tx, &new.run_id)?.is_some() {
        return Ok(Rejected(TreeRejection::Conflict));
    }
    let identity = get_identity(&mut tx, &new.run_id)?;
    if let Err(why) = check_branch_identity(tree.identity_hash.as_deref(), identity.as_ref()) {
        return Ok(Rejected(why));
    }
    let Some(source) = get_node(&mut tx, &new.tree_id, new.source_node)? else {
        return Ok(Rejected(TreeRejection::SourceNotFound));
    };
    // Finished is final: a finished run never starts another attempt, so its nodes (and the source node) stay. The
    // share lock keeps the answer true until this transaction ends.
    if !run_finished(&mut tx, &tree.id)? || !run_finished(&mut tx, &source.run_id)? {
        return Ok(Rejected(TreeRejection::RunNotFinished));
    }
    // Read the node again now that both run locks are held. Interleaving this closes: `finish_job` holds the source
    // run FOR UPDATE and, in that transaction, deletes the run's nodes (`start_attempt`); we had read the node before
    // that, blocked on the share lock, and under READ COMMITTED see the run as finished once it commits, but the node
    // is gone, and `tree_branches.source_node` has no foreign key. The new statement sees the committed delete.
    match get_node(&mut tx, &new.tree_id, new.source_node)? {
        Some(again) if again.run_id == source.run_id => {}
        _ => return Ok(Rejected(TreeRejection::SourceNotFound)),
    }
    let branches: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM tree_branches WHERE tree_id=$1",
            &[&new.tree_id],
        )?
        .get(0);
    if branches as u64 >= MAX_BRANCH_RUNS {
        return Ok(Rejected(TreeRejection::BranchCap));
    }
    tx.execute(
        &format!("INSERT INTO tree_branches({BRANCH_COLUMNS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,0,0,$10,$11)"),
        &[
            &new.run_id, &new.tree_id, &to_db(new.source_node), &new.cut_kind.as_str(), &to_db(new.cut_at),
            &new.point_hash, &new.command_json, &new.command_hash, &new.seed_source.as_str(), &new.created_by, &now,
        ],
    )?;
    tx.execute(
        "UPDATE trees SET run_count=run_count+1 WHERE id=$1",
        &[&new.tree_id],
    )?;
    let stored = get_branch(&mut tx, &new.run_id)?.ok_or_else(|| {
        StoreError::Backend("a branch row vanished inside its transaction".into())
    })?;
    tx.commit()?;
    Ok(BranchOutcome::Added(stored))
}

pub(crate) fn record_verify(c: &mut postgres::Client, run_id: &str, matched: bool) -> Result<()> {
    let n = c.execute(
        "UPDATE tree_branches SET verify_runs=verify_runs+1, verify_matches=verify_matches+$1 WHERE run_id=$2",
        &[&i64::from(matched), &run_id],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("branch run {run_id}")));
    }
    Ok(())
}

// ---- a new attempt of a run's job --------------------------------------------------------------------------------

/// Why the run's rows may not be started over, if something depends on them: branch runs of the tree it roots, other
/// runs' nodes below its nodes, or branches that start at its nodes.
pub(crate) fn attempt_blocked(
    tx: &mut postgres::Transaction,
    run_id: &str,
) -> Result<Option<String>> {
    let blocked: bool = tx
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM tree_branches WHERE tree_id=$1) \
               OR EXISTS (SELECT 1 FROM trial_lineage c JOIN trial_lineage p ON p.tree_id=c.tree_id AND p.node=c.parent_node \
                           WHERE p.run_id=$1 AND c.run_id<>$1) \
               OR EXISTS (SELECT 1 FROM tree_branches b JOIN trial_lineage n ON n.tree_id=b.tree_id AND n.node=b.source_node \
                           WHERE n.run_id=$1)",
            &[&run_id],
        )?
        .get(0);
    Ok(blocked.then(|| {
        "this run roots a tree with branch runs, or other runs build on its nodes; it cannot start a new attempt"
            .to_string()
    }))
}

/// Deletes the run's nodes (they refer to its trial rows, which are deleted next) and returns the edge blobs they
/// named. `trees.node_count` is never lowered: it counts the numbers handed out, so a deleted node's number is never
/// given to another node and the gap stays.
pub(crate) fn drop_run_lineage(
    tx: &mut postgres::Transaction,
    run_id: &str,
) -> Result<Vec<String>> {
    let edges: Vec<String> = tx
        .query(
            "SELECT edge_ref FROM trial_lineage WHERE run_id=$1 AND edge_ref IS NOT NULL",
            &[&run_id],
        )?
        .iter()
        .map(|r| r.get(0))
        .collect();
    tx.execute("DELETE FROM trial_lineage WHERE run_id=$1", &[&run_id])?;
    Ok(edges)
}

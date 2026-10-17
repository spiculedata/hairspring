//! Trial trees (schema version 3): the types the [`crate::Store`] methods use, the limits, and the pure arithmetic both
//! stores share (paths, paging windows, the rules a new node must satisfy). The stores only fetch facts and run SQL;
//! every decision that must not differ between SQLite and PostgreSQL is made here.
//!
//! A node's `path` is the concatenation of the unsigned LEB128 encodings of the node numbers from the root to the
//! node. LEB128 is prefix-free, so the descendants of a node are exactly the rows whose path lies strictly between
//! the node's path and its successor (the same bytes with the last one increased by one).
use crate::trials::TrialVerdict;
use serde::{Deserialize, Serialize};

/// Deepest node (the root is depth 0).
pub const MAX_TREE_DEPTH: u32 = 512;
/// Children of one parent that start at the same cut.
pub const MAX_FANOUT: u64 = 1024;
/// Node numbers a tree may allocate.
pub const MAX_TREE_NODES: u64 = 100_000;
/// Branch runs per tree.
pub const MAX_BRANCH_RUNS: u64 = 1000;
/// The width of the node-number window one path-form list request scans.
pub const NODE_WINDOW: u64 = 20_000;
/// A recursive-form descendant listing answers for at most this many descendants; above it, page by children or export.
pub const MAX_CTE_DESCENDANTS: u64 = 50_000;
/// Trees with at least this many nodes use the path form for subtree queries on PostgreSQL. The tree cap equals it
/// today, so the rule is dormant until the cap is raised.
pub const PG_PATH_FORM_MIN_NODES: u64 = 100_000;
/// The most rows one page returns.
pub const MAX_PAGE_LIMIT: u32 = 500;
/// The most text a run identity holds.
pub const MAX_IDENTITY_TEXT_BYTES: usize = 16 * 1024;
/// The most text a stored command holds.
pub const MAX_COMMAND_JSON_BYTES: usize = 64 * 1024;
const MAX_SHORT_TEXT_BYTES: usize = 128;

string_enum!(Operator { Root => "root", Explore => "explore", Step => "step", Walk => "walk" });
string_enum!(CutKind { None => "none", Step => "step", Choice => "choice" });
string_enum!(NodeOrigin { Server => "server", Exploration => "exploration", PointExec => "point_exec" });
string_enum!(Verified { No => "no", ReplayOk => "replay_ok", ReplayMismatch => "replay_mismatch" });
string_enum!(SeedSource { ServerRandom => "server_random", Caller => "caller" });

const IDENTITY_SCHEMES: [&str; 3] = ["sim/1", "wasm/1", "hermit/1"];

/// Why the store refused a tree, node, branch or identity. Nothing was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRejection {
    RunNotFound,
    TreeNotFound,
    TrialNotFound,
    /// The run is neither the tree's root run nor one of its branch runs.
    RunNotInTree,
    ParentNotFound,
    SourceNotFound,
    /// A branch attaches only below nodes that cannot be deleted: the tree's root run and the run of the source node
    /// must both have finished (a finished run never starts a new attempt).
    RunNotFinished,
    TrialAlreadyInTree,
    DepthCap,
    FanoutCap,
    NodeCap,
    BranchCap,
    /// The identity differs from the tree's.
    IdentityMismatch,
    /// There is no complete identity (or the tree has none), so there is no point identity to compare.
    IdentityIncomplete,
    /// The stored row differs from the one offered, and stored rows never change.
    Conflict,
    Invalid(&'static str),
}

/// The execution inputs of a run, recorded once. `identity_hash` is the SHA-256 hex of `identity_text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunIdentity {
    pub run_id: String,
    pub scheme: String,
    pub identity_text: String,
    pub identity_hash: String,
    pub complete: bool,
    pub reporter_id: String,
    pub reporter_trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityOutcome {
    Stored,
    /// An incomplete identity was replaced by a complete one.
    Completed,
    /// The same identity was already stored; nothing changed.
    Duplicate,
    Rejected(TreeRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTree {
    /// The root run; it is also the tree's id.
    pub run_id: String,
    pub backend: String,
    pub target: String,
    /// The root run's identity hash; `None` for a tree without point identity.
    pub identity_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRecord {
    pub id: String,
    pub backend: String,
    pub target: String,
    pub identity_hash: Option<String>,
    /// The next node number; also the number of nodes allocated so far.
    pub node_count: u64,
    pub run_count: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeOutcome {
    Created(TreeRecord),
    /// The same tree already exists.
    Existing(TreeRecord),
    Rejected(TreeRejection),
}

/// A trial to attach to a tree. Everything structural (number, depth, path, verdict) is derived by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNode {
    pub run_id: String,
    pub seq: u64,
    pub parent_node: Option<u64>,
    pub operator: Operator,
    pub cut_kind: CutKind,
    pub cut_at: u64,
    pub suffix_seed: Option<u64>,
    pub edge_ref: Option<String>,
    pub origin: NodeOrigin,
    pub reporter_id: String,
    pub reporter_trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRecord {
    pub tree_id: String,
    pub node: u64,
    pub run_id: String,
    pub seq: u64,
    pub parent_node: Option<u64>,
    pub depth: u32,
    pub path: Vec<u8>,
    pub verdict: TrialVerdict,
    pub operator: Operator,
    pub cut_kind: CutKind,
    pub cut_at: u64,
    pub suffix_seed: Option<u64>,
    pub edge_ref: Option<String>,
    pub origin: NodeOrigin,
    pub reporter_id: String,
    pub reporter_trusted: bool,
    pub verified: Verified,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeOutcome {
    Added(NodeRecord),
    Rejected(TreeRejection),
}

/// One page of nodes in node order. `next_cursor` is `None` once no later node can exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePage {
    pub nodes: Vec<NodeRecord>,
    pub next_cursor: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictCount {
    pub verdict: TrialVerdict,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBranch {
    pub run_id: String,
    pub tree_id: String,
    pub source_node: u64,
    pub cut_kind: CutKind,
    pub cut_at: u64,
    pub point_hash: String,
    pub command_json: String,
    pub command_hash: String,
    pub seed_source: SeedSource,
    /// The token's key prefix, never the token.
    pub created_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRecord {
    pub run_id: String,
    pub tree_id: String,
    pub source_node: u64,
    pub cut_kind: CutKind,
    pub cut_at: u64,
    pub point_hash: String,
    pub command_json: String,
    pub command_hash: String,
    pub seed_source: SeedSource,
    pub verify_runs: u64,
    pub verify_matches: u64,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchOutcome {
    Added(BranchRecord),
    /// A branch for the same command already exists (a cache hit); nothing was written.
    Existing(BranchRecord),
    Rejected(TreeRejection),
}

/// The tree queries whose plans the tests inspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeQuery {
    SubtreeDescendants,
    TreeChildren,
    SubtreeTally,
    SubtreeFirstFailing,
    Ancestors,
}

/// How a subtree query is answered. Both forms return the same rows; they differ in cost. `Cte` walks `parent_node`
/// with a recursive query (one index search per descendant). `Path` reads one range of the materialised path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeQueryForm {
    Cte,
    Path,
}

/// Which form the subtree queries use. Ancestors always use the recursive form and children always use keyset paging
/// on `lineage_children`; neither depends on the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeQueryRule {
    /// The store's own rule: SQLite always uses the recursive form; PostgreSQL uses it below
    /// [`PG_PATH_FORM_MIN_NODES`] nodes in the tree and the path form at or above.
    StoreDefault,
    AlwaysCte,
    AlwaysPath,
    /// The path form on trees with at least this many nodes, the recursive form below.
    PathFromNodes(u64),
}

impl TreeQueryRule {
    /// `store_default_min` is the store's own threshold for the path form (`None`: never).
    pub(crate) fn form(self, store_default_min: Option<u64>, node_count: u64) -> TreeQueryForm {
        let min = match self {
            TreeQueryRule::AlwaysCte => None,
            TreeQueryRule::AlwaysPath => Some(0),
            TreeQueryRule::PathFromNodes(n) => Some(n),
            TreeQueryRule::StoreDefault => store_default_min,
        };
        match min {
            Some(n) if node_count >= n => TreeQueryForm::Path,
            _ => TreeQueryForm::Cte,
        }
    }
}

// ---- paths -------------------------------------------------------------------------------------------------------

/// The path of a child: the parent's path followed by the LEB128 encoding of the child's node number.
pub fn child_path(parent: &[u8], node: u64) -> Vec<u8> {
    let mut path = parent.to_vec();
    let mut rest = node;
    loop {
        let low = (rest & 0x7f) as u8;
        rest >>= 7;
        if rest == 0 {
            path.push(low);
            return path;
        }
        path.push(low | 0x80);
    }
}

/// The node numbers a path encodes, root first. `None` for bytes that are not a whole sequence of numbers.
pub fn decode_path(path: &[u8]) -> Option<Vec<u64>> {
    let mut out = Vec::new();
    let (mut value, mut shift) = (0u64, 0u32);
    for byte in path {
        let part = u64::from(byte & 0x7f);
        value |= part.checked_shl(shift).filter(|v| v >> shift == part)?;
        if byte & 0x80 == 0 {
            out.push(value);
            (value, shift) = (0, 0);
        } else {
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
    (shift == 0).then_some(out)
}

/// `[path, successor)` holds the node and all of its descendants. The successor adds one to the last byte; the last
/// byte of a whole path is below 0x80, so the addition never carries.
pub fn subtree_bounds(path: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut hi = path.to_vec();
    match hi.last_mut() {
        Some(last) if *last < 0xff => *last += 1,
        _ => hi.push(0xff),
    }
    (path.to_vec(), hi)
}

// ---- paging: one cursor rule for every tree endpoint ---------------------------------------------------------------

pub(crate) fn page_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_PAGE_LIMIT)
}

/// Nodes after the cursor, never at or below `floor` (the listed node itself).
pub(crate) fn window_start(after: Option<u64>, floor: u64) -> u64 {
    after.map_or(floor, |a| a.max(floor))
}

pub(crate) fn window_end(start: u64) -> u64 {
    start.saturating_add(NODE_WINDOW)
}

/// The cursor for the next request: the last node returned if the page filled, otherwise the window end. `None` once
/// the cursor has reached the last node number the tree has allocated.
pub(crate) fn next_cursor(start: u64, last_if_filled: Option<u64>, node_count: u64) -> Option<u64> {
    let next = last_if_filled.unwrap_or_else(|| window_end(start));
    (next.saturating_add(1) < node_count).then_some(next)
}

// ---- rules -------------------------------------------------------------------------------------------------------

fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains('\0')
}

fn opt_text_ok(s: &Option<String>) -> bool {
    s.as_deref()
        .is_none_or(|s| text_ok(s, MAX_SHORT_TEXT_BYTES))
}

pub(crate) fn check_identity(id: &RunIdentity) -> std::result::Result<(), TreeRejection> {
    if !IDENTITY_SCHEMES.contains(&id.scheme.as_str()) {
        return Err(TreeRejection::Invalid("unknown identity scheme"));
    }
    if !text_ok(&id.identity_text, MAX_IDENTITY_TEXT_BYTES) {
        return Err(TreeRejection::Invalid(
            "identity text is empty, too long or has a NUL",
        ));
    }
    if id.identity_hash != crate::sha256_hex(id.identity_text.as_bytes()) {
        return Err(TreeRejection::Invalid(
            "identity hash is not the SHA-256 of the identity text",
        ));
    }
    if !text_ok(&id.reporter_id, MAX_SHORT_TEXT_BYTES) {
        return Err(TreeRejection::Invalid(
            "reporter id is empty, too long or has a NUL",
        ));
    }
    Ok(())
}

/// Whether a stored identity is the one offered (the reporter of the first write stays).
pub(crate) fn same_identity(a: &RunIdentity, b: &RunIdentity) -> bool {
    (&a.scheme, &a.identity_text, &a.identity_hash, a.complete)
        == (&b.scheme, &b.identity_text, &b.identity_hash, b.complete)
}

/// Whether the offered identity completes the stored one: an incomplete identity is the only one that may be replaced,
/// and only by a complete one (and only while no tree depends on the run; the store checks that).
pub(crate) fn completes(stored: &RunIdentity, offered: &RunIdentity) -> bool {
    !stored.complete && offered.complete
}

/// A tree has point identity exactly when its root run has a complete identity, and then they must agree.
pub(crate) fn check_tree_identity(
    given: Option<&str>,
    root_identity: Option<&RunIdentity>,
) -> std::result::Result<(), TreeRejection> {
    match (given, root_identity.filter(|i| i.complete)) {
        (Some(g), Some(i)) if g == i.identity_hash => Ok(()),
        (Some(_), Some(_)) | (None, Some(_)) => Err(TreeRejection::IdentityMismatch),
        (Some(_), None) => Err(TreeRejection::IdentityIncomplete),
        (None, None) => Ok(()),
    }
}

/// Every run in a tree has exactly the tree's identity.
pub(crate) fn check_branch_identity(
    tree_hash: Option<&str>,
    run_identity: Option<&RunIdentity>,
) -> std::result::Result<(), TreeRejection> {
    let Some(tree_hash) = tree_hash else {
        return Err(TreeRejection::IdentityIncomplete);
    };
    match run_identity.filter(|i| i.complete) {
        None => Err(TreeRejection::IdentityIncomplete),
        Some(i) if i.identity_hash == tree_hash => Ok(()),
        Some(_) => Err(TreeRejection::IdentityMismatch),
    }
}

/// The fields a node must have before any lookup is made.
pub(crate) fn check_node_shape(new: &NewNode) -> std::result::Result<(), TreeRejection> {
    if !text_ok(&new.reporter_id, MAX_SHORT_TEXT_BYTES) || !opt_text_ok(&new.edge_ref) {
        return Err(TreeRejection::Invalid(
            "reporter id or edge reference is empty, too long or has a NUL",
        ));
    }
    if new.cut_kind == CutKind::None && new.cut_at != 0 {
        return Err(TreeRejection::Invalid("a cut position needs a cut kind"));
    }
    match (new.parent_node, new.operator) {
        (None, Operator::Root) if new.cut_kind == CutKind::None && new.suffix_seed.is_none() => {
            Ok(())
        }
        (None, _) => Err(TreeRejection::Invalid(
            "a root node has operator root, no cut and no suffix seed",
        )),
        (Some(_), Operator::Root) => {
            Err(TreeRejection::Invalid("only a root node has operator root"))
        }
        (Some(_), _) => Ok(()),
    }
}

pub(crate) fn check_branch_shape(new: &NewBranch) -> std::result::Result<(), TreeRejection> {
    let hashes_ok = text_ok(&new.point_hash, MAX_SHORT_TEXT_BYTES)
        && text_ok(&new.command_hash, MAX_SHORT_TEXT_BYTES);
    if !hashes_ok || !text_ok(&new.created_by, MAX_SHORT_TEXT_BYTES) {
        return Err(TreeRejection::Invalid(
            "a hash or the creator is empty, too long or has a NUL",
        ));
    }
    if !text_ok(&new.command_json, MAX_COMMAND_JSON_BYTES) {
        return Err(TreeRejection::Invalid(
            "the command is empty, too long or has a NUL",
        ));
    }
    if new.cut_kind == CutKind::None && new.cut_at != 0 {
        return Err(TreeRejection::Invalid("a cut position needs a cut kind"));
    }
    Ok(())
}

/// What the store learned about the parent.
pub(crate) struct ParentFacts {
    pub depth: u32,
    pub path: Vec<u8>,
}

/// Where a new node lands.
pub(crate) struct Placement {
    pub node: u64,
    pub depth: u32,
    pub path: Vec<u8>,
}

/// Applies the caps and derives number, depth and path. `siblings_at_cut` counts the parent's children that start at
/// the new node's cut.
pub(crate) fn place(
    node_count: u64,
    new: &NewNode,
    parent: Option<&ParentFacts>,
    siblings_at_cut: u64,
) -> std::result::Result<Placement, TreeRejection> {
    if node_count >= MAX_TREE_NODES {
        return Err(TreeRejection::NodeCap);
    }
    let (depth, base): (u32, &[u8]) = match (new.parent_node, parent) {
        (None, _) => (0, &[]),
        (Some(_), None) => return Err(TreeRejection::ParentNotFound),
        (Some(_), Some(p)) => (p.depth + 1, &p.path),
    };
    if depth > MAX_TREE_DEPTH {
        return Err(TreeRejection::DepthCap);
    }
    if siblings_at_cut >= MAX_FANOUT {
        return Err(TreeRejection::FanoutCap);
    }
    Ok(Placement {
        node: node_count,
        depth,
        path: child_path(base, node_count),
    })
}

/// A page read by keyset: the query asked for one row more than `limit`. If the extra row came, more exist and the
/// cursor is the last row kept; otherwise the list is finished.
pub(crate) fn keyset_page(mut nodes: Vec<NodeRecord>, limit: u32) -> NodePage {
    let more = nodes.len() > limit as usize;
    nodes.truncate(limit as usize);
    NodePage {
        next_cursor: more.then(|| nodes.last().map(|n| n.node)).flatten(),
        nodes,
    }
}

/// The page of a path-form read: a window of node numbers filtered by path (see [`next_cursor`]).
pub(crate) fn window_page(
    nodes: Vec<NodeRecord>,
    start: u64,
    limit: u32,
    node_count: u64,
) -> NodePage {
    let filled = nodes.len() as u64 >= u64::from(limit);
    let last = nodes.last().map(|n| n.node).filter(|_| filled);
    NodePage {
        next_cursor: next_cursor(start, last, node_count),
        nodes,
    }
}

/// Builds the column list of a node row, optionally behind a table alias, for use in `SELECT` text.
macro_rules! node_columns {
    ($p:literal) => {
        concat!(
            $p,
            "tree_id, ",
            $p,
            "node, ",
            $p,
            "run_id, ",
            $p,
            "seq, ",
            $p,
            "parent_node, ",
            $p,
            "depth, ",
            $p,
            "path, ",
            $p,
            "verdict, ",
            $p,
            "operator, ",
            $p,
            "cut_kind, ",
            $p,
            "cut_at, ",
            $p,
            "suffix_seed, ",
            $p,
            "edge_ref, ",
            $p,
            "origin, ",
            $p,
            "reporter_id, ",
            $p,
            "reporter_trusted, ",
            $p,
            "verified, ",
            $p,
            "created_at"
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb128_paths_round_trip_and_are_prefix_free() {
        for node in [
            0u64,
            1,
            127,
            128,
            129,
            255,
            256,
            16_383,
            16_384,
            99_999,
            u64::MAX,
        ] {
            let path = child_path(&child_path(&[], 3), node);
            assert_eq!(decode_path(&path), Some(vec![3, node]), "{node}");
        }
        assert_eq!(child_path(&[], 256), vec![0x80, 0x02]);
        assert_eq!(child_path(&[], 130), vec![0x82, 0x01]);
        assert!(
            child_path(&[], 256) < child_path(&[], 130),
            "byte order is not node order"
        );
    }

    #[test]
    fn malformed_paths_do_not_decode() {
        assert_eq!(decode_path(&[0x80]), None, "a number that never ends");
        assert_eq!(decode_path(&[0xff; 11]), None, "more than 64 bits");
        assert_eq!(decode_path(&[]), Some(vec![]));
    }

    #[test]
    fn the_successor_of_a_whole_path_adds_one_to_the_last_byte() {
        assert_eq!(
            subtree_bounds(&[0x00, 0x7f]),
            (vec![0x00, 0x7f], vec![0x00, 0x80])
        );
        assert_eq!(subtree_bounds(&[0x05]).1, vec![0x06]);
    }

    #[test]
    fn descendants_are_exactly_the_paths_between_the_bounds() {
        let a = child_path(&child_path(&[], 4), 127);
        let (lo, hi) = subtree_bounds(&a);
        let inside = [
            child_path(&a, 0),
            child_path(&a, 9_999),
            child_path(&child_path(&a, 3), 3),
        ];
        let outside = [
            child_path(&child_path(&[], 4), 128),
            child_path(&child_path(&[], 4), 126),
            child_path(&[], 5),
            child_path(&[], 4),
        ];
        assert!(inside.iter().all(|p| *p > lo && *p < hi));
        assert!(outside.iter().all(|p| !(*p > lo && *p < hi)));
    }

    #[test]
    fn the_rule_picks_the_form_by_tree_size() {
        use TreeQueryForm::{Cte, Path};
        assert_eq!(
            TreeQueryRule::StoreDefault.form(None, 10_000_000),
            Cte,
            "SQLite never uses the path form"
        );
        assert_eq!(
            TreeQueryRule::StoreDefault
                .form(Some(PG_PATH_FORM_MIN_NODES), PG_PATH_FORM_MIN_NODES - 1),
            Cte
        );
        assert_eq!(
            TreeQueryRule::StoreDefault.form(Some(PG_PATH_FORM_MIN_NODES), PG_PATH_FORM_MIN_NODES),
            Path
        );
        assert_eq!(TreeQueryRule::AlwaysCte.form(Some(0), 1_000_000), Cte);
        assert_eq!(TreeQueryRule::AlwaysPath.form(None, 0), Path);
        assert_eq!(TreeQueryRule::PathFromNodes(10).form(None, 9), Cte);
        assert_eq!(TreeQueryRule::PathFromNodes(10).form(None, 10), Path);
    }

    fn node(n: u64) -> NodeRecord {
        NodeRecord {
            tree_id: "t".into(),
            node: n,
            run_id: "r".into(),
            seq: n,
            parent_node: None,
            depth: 0,
            path: child_path(&[], n),
            verdict: TrialVerdict::Pass,
            operator: Operator::Root,
            cut_kind: CutKind::None,
            cut_at: 0,
            suffix_seed: None,
            edge_ref: None,
            origin: NodeOrigin::Server,
            reporter_id: "w".into(),
            reporter_trusted: true,
            verified: Verified::No,
            created_at: 0,
        }
    }

    #[test]
    fn a_keyset_page_has_a_cursor_only_when_the_extra_row_came() {
        let rows = |n: u64| (1..=n).map(node).collect::<Vec<_>>();
        let full = keyset_page(rows(3), 2);
        assert_eq!((full.nodes.len(), full.next_cursor), (2, Some(2)));
        let exact = keyset_page(rows(2), 2);
        assert_eq!((exact.nodes.len(), exact.next_cursor), (2, None));
        let empty = keyset_page(vec![], 2);
        assert_eq!((empty.nodes.len(), empty.next_cursor), (0, None));
    }

    #[test]
    fn the_cursor_is_the_last_node_when_full_and_the_window_end_when_not() {
        assert_eq!(next_cursor(0, Some(40), 100), Some(40));
        assert_eq!(next_cursor(0, None, 100_000), Some(20_000));
        assert_eq!(next_cursor(20_000, None, 100_000), Some(40_000));
        assert_eq!(
            next_cursor(0, None, 100),
            None,
            "the window reaches the last node"
        );
        assert_eq!(
            next_cursor(0, Some(99), 100),
            None,
            "the page ended on the last node"
        );
        assert_eq!(next_cursor(0, Some(98), 100), Some(98));
        assert_eq!(next_cursor(0, None, 0), None);
    }
}

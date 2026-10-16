//! Randomly generated trial trees (deterministic, no dependency) and the oracles the path queries are compared with:
//! an in-memory model over the parent array, and a `WITH RECURSIVE` query over `parent_node` run by the store's own
//! test harness.
#![allow(dead_code)]
use super::tree_support::{in_each_form, ingest_verdicts, new_run};
use hairspring_store::*;
use std::collections::BTreeSet;

/// Small xorshift generator: the same seed gives the same tree on every platform.
pub struct Prng(u64);

impl Prng {
    pub fn new(seed: u64) -> Prng {
        Prng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// Parents are uniform over the earlier nodes.
    Uniform,
    /// Mostly the previous node: long chains, capped at the depth limit.
    Deep,
    /// Parents come from the first few nodes: very wide.
    Wide,
}

/// `(seed, node count, shape)`: from a single node to 5,000 nodes, including chains that reach the depth cap.
pub const ORACLE_CASES: &[(u64, usize, Shape)] = &[
    (1, 1, Shape::Uniform),
    (2, 2, Shape::Deep),
    (3, 60, Shape::Uniform),
    (4, 400, Shape::Wide),
    (5, 1_500, Shape::Uniform),
    (6, 3_000, Shape::Deep),
    (7, 5_000, Shape::Wide),
];

pub struct TreeSpec {
    pub parents: Vec<Option<usize>>,
    pub verdicts: Vec<TrialVerdict>,
    pub depths: Vec<u32>,
}

/// Node `i` has a parent with a smaller number (or none, for roots: node 0 and about 2% of the others).
pub fn generate(seed: u64, n: usize, shape: Shape) -> TreeSpec {
    let mut rng = Prng::new(seed);
    let mut parents: Vec<Option<usize>> = Vec::with_capacity(n);
    let mut depths: Vec<u32> = Vec::with_capacity(n);
    let mut children = vec![0usize; n];
    for i in 0..n {
        let parent = if i == 0 || rng.below(50) == 0 {
            None
        } else {
            let pick = |rng: &mut Prng| match shape {
                Shape::Uniform => rng.below(i as u64) as usize,
                Shape::Deep if rng.below(10) < 8 => i - 1,
                Shape::Deep => rng.below(i as u64) as usize,
                Shape::Wide => rng.below(i.min(6) as u64) as usize,
            };
            // The depth cap and the fan-out cap are store rules; the generator stays inside them.
            let mut p = pick(&mut rng);
            while depths[p] >= MAX_TREE_DEPTH - 1 || children[p] >= 4 * MAX_FANOUT as usize - 8 {
                p = rng.below(i as u64) as usize;
            }
            Some(p)
        };
        depths.push(parent.map_or(0, |p| depths[p] + 1));
        if let Some(p) = parent {
            children[p] += 1;
        }
        parents.push(parent);
    }
    let verdicts = (0..n)
        .map(|_| {
            if rng.below(10) == 0 {
                TrialVerdict::Violation
            } else {
                TrialVerdict::Pass
            }
        })
        .collect();
    TreeSpec {
        parents,
        verdicts,
        depths,
    }
}

/// A random recursive tree with the single root 0: the parent of node `i` is uniform over `0..i`, so the depth stays
/// far below the cap and a small node's subtree is spread over the whole number range.
pub fn big_tree(n: usize, seed: u64) -> TreeSpec {
    let mut rng = Prng::new(seed);
    let mut parents: Vec<Option<usize>> = vec![None];
    let mut depths = vec![0u32];
    for i in 1..n {
        let p = rng.below(i as u64) as usize;
        parents.push(Some(p));
        depths.push(depths[p] + 1);
    }
    let verdicts = (0..n)
        .map(|_| {
            if rng.below(100) == 0 {
                TrialVerdict::Violation
            } else {
                TrialVerdict::Pass
            }
        })
        .collect();
    TreeSpec {
        parents,
        verdicts,
        depths,
    }
}

/// The path of every node, computed the way the store does.
pub fn paths_of(spec: &TreeSpec) -> Vec<Vec<u8>> {
    let mut paths: Vec<Vec<u8>> = Vec::with_capacity(spec.parents.len());
    for (i, parent) in spec.parents.iter().enumerate() {
        let path = match parent {
            None => child_path(&[], i as u64),
            Some(p) => child_path(&paths[*p], i as u64),
        };
        paths.push(path);
    }
    paths
}

/// Pages every descendant of several nodes of a big tree and compares with the model. Prints how many requests each
/// took. In the recursive form a node with more than [`MAX_CTE_DESCENDANTS`] descendants must be refused, not paged.
pub fn check_big_tree_pages(s: &dyn Store, tree: &str, spec: &TreeSpec, rule: TreeQueryRule) {
    let n = spec.parents.len();
    s.set_tree_query_rule(rule);
    for a in [0usize, 1, 50, 5_000, n - 1] {
        let want: Vec<u64> = model_subtree(spec, a)
            .into_iter()
            .filter(|x| *x != a)
            .map(|x| x as u64)
            .collect();
        if rule == TreeQueryRule::AlwaysCte && want.len() as u64 > MAX_CTE_DESCENDANTS {
            let err = s
                .subtree_descendants(tree, a as u64, None, 500)
                .unwrap_err();
            assert!(
                matches!(err, StoreError::LimitExceeded { limit, .. } if limit == MAX_CTE_DESCENDANTS),
                "node {a} has {} descendants: {err}",
                want.len()
            );
            eprintln!(
                "node {a}: {} descendants, refused in the recursive form: {err}",
                want.len()
            );
            continue;
        }
        let mut got: Vec<u64> = Vec::new();
        let (mut cursor, mut requests) = (None, 0u32);
        loop {
            requests += 1;
            let page = s.subtree_descendants(tree, a as u64, cursor, 500).unwrap();
            got.extend(page.nodes.iter().map(|r| r.node));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(
            got, want,
            "every descendant of node {a} exactly once, in node order ({rule:?})"
        );
        eprintln!(
            "node {a}: {} descendants in {requests} requests ({rule:?})",
            want.len()
        );
    }
    s.set_tree_query_rule(TreeQueryRule::StoreDefault);
}

/// Children are read by keyset, so siblings that are far apart in node numbers still come back page by page: no page
/// is empty unless it is the last, whatever the gaps are.
pub fn check_children_far_apart(s: &dyn Store, tree: &str, spec: &TreeSpec) {
    for a in [0usize, 1, 7] {
        let want: Vec<u64> = (0..spec.parents.len())
            .filter(|i| spec.parents[*i] == Some(a))
            .map(|i| i as u64)
            .collect();
        let widest_gap = want.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
        let (mut got, mut cursor, mut pages) = (Vec::new(), None, 0u32);
        loop {
            let page = s.tree_children(tree, a as u64, cursor, 10).unwrap();
            pages += 1;
            assert!(
                !page.nodes.is_empty() || page.next_cursor.is_none(),
                "an empty page that is not the last"
            );
            got.extend(page.nodes.iter().map(|r| r.node));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(got, want, "children of node {a}");
        eprintln!(
            "children of node {a}: {} in {pages} pages, widest gap {widest_gap}",
            want.len()
        );
        if a == 0 {
            assert!(
                widest_gap > NODE_WINDOW,
                "the fixture must have siblings further apart than a window"
            );
        }
    }
}

/// The cut a child of `parent` starts at. Spreading children over several cuts keeps every `(parent, cut)` group under
/// the fan-out cap even for the wide shape.
fn cut_for(i: usize) -> u64 {
    (i % 8) as u64
}

/// Stores the spec as one run and one tree; trial `seq` and node number both equal the spec index.
pub fn load(s: &dyn Store, spec: &TreeSpec) -> String {
    let run = new_run(s, "sim");
    ingest_verdicts(s, &run.id, &spec.verdicts);
    let outcome = s
        .create_tree(
            &NewTree {
                run_id: run.id.clone(),
                backend: "sim".into(),
                target: "t".into(),
                identity_hash: None,
            },
            5,
        )
        .unwrap();
    assert!(matches!(outcome, TreeOutcome::Created(_)), "{outcome:?}");
    for (i, parent) in spec.parents.iter().enumerate() {
        let new = NewNode {
            run_id: run.id.clone(),
            seq: i as u64,
            parent_node: parent.map(|p| p as u64),
            operator: if parent.is_some() {
                Operator::Explore
            } else {
                Operator::Root
            },
            cut_kind: if parent.is_some() {
                CutKind::Step
            } else {
                CutKind::None
            },
            cut_at: if parent.is_some() { cut_for(i) } else { 0 },
            suffix_seed: None,
            edge_ref: None,
            origin: NodeOrigin::Exploration,
            reporter_id: "w".into(),
            reporter_trusted: true,
        };
        match s.add_tree_node(&run.id, &new, 6).unwrap() {
            NodeOutcome::Added(n) => assert_eq!(n.node, i as u64),
            other => panic!("node {i} refused: {other:?}"),
        }
    }
    run.id
}

/// What a `WITH RECURSIVE` query over `parent_node` returns for a node.
pub struct CteAnswer {
    /// `(node, verdict, depth)` of the node and every descendant.
    pub subtree: Vec<(u64, String, u32)>,
    /// The ancestors, nearest first, found by walking `parent_node` upwards.
    pub ancestors: Vec<u64>,
}

pub fn model_subtree(spec: &TreeSpec, a: usize) -> BTreeSet<usize> {
    let mut found = BTreeSet::from([a]);
    // Parents have smaller numbers, so one ascending pass reaches every descendant.
    for i in a + 1..spec.parents.len() {
        if spec.parents[i].is_some_and(|p| found.contains(&p)) {
            found.insert(i);
        }
    }
    found
}

pub fn model_ancestors(spec: &TreeSpec, a: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = a;
    while let Some(p) = spec.parents[at] {
        out.push(p);
        at = p;
    }
    out
}

/// Every descendant of `a` read page by page; checks the cursor contract on the way.
pub fn collect_descendants(s: &dyn Store, tree: &str, a: u64, limit: u32) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let mut cursor = None;
    for _ in 0..100_000 {
        let page = s.subtree_descendants(tree, a, cursor, limit).unwrap();
        assert!(page.nodes.len() <= limit as usize);
        out.extend(page.nodes.iter().map(|n| n.node));
        match page.next_cursor {
            Some(c) => {
                assert!(cursor.is_none_or(|prev| c > prev), "cursor must advance");
                cursor = Some(c);
            }
            None => break,
        }
    }
    assert!(
        out.windows(2).all(|w| w[0] < w[1]),
        "node order, no repeats"
    );
    out
}

/// Compares every subtree query, in each of its two forms, with the in-memory model and with an independent recursive
/// query run by the harness, for a sample of nodes.
pub fn check_against_oracles(
    s: &dyn Store,
    tree: &str,
    spec: &TreeSpec,
    cte: &dyn Fn(&str, u64) -> CteAnswer,
    seed: u64,
) {
    in_each_form(s, &mut |rule| check_in_form(s, tree, spec, cte, seed, rule));
}

fn check_in_form(
    s: &dyn Store,
    tree: &str,
    spec: &TreeSpec,
    cte: &dyn Fn(&str, u64) -> CteAnswer,
    seed: u64,
    rule: TreeQueryRule,
) {
    let n = spec.parents.len();
    let mut rng = Prng::new(seed ^ 0xABCD);
    let deepest = (0..n).max_by_key(|i| spec.depths[*i]).unwrap();
    let mut sample: BTreeSet<usize> = BTreeSet::from([0, n - 1, deepest]);
    while sample.len() < n.min(40) {
        sample.insert(rng.below(n as u64) as usize);
    }
    for a in sample {
        let want = model_subtree(spec, a);
        let answer = cte(tree, a as u64);
        let cte_nodes: BTreeSet<usize> =
            answer.subtree.iter().map(|(n, _, _)| *n as usize).collect();
        assert_eq!(
            cte_nodes, want,
            "the model and the recursive query agree on node {a}"
        );

        let below: Vec<u64> = collect_descendants(s, tree, a as u64, 7);
        let expected: Vec<u64> = want
            .iter()
            .filter(|x| **x != a)
            .map(|x| *x as u64)
            .collect();
        assert_eq!(below, expected, "descendants of node {a}");
        assert_eq!(
            collect_descendants(s, tree, a as u64, 500),
            expected,
            "page size does not change the answer"
        );

        let mut tally = std::collections::BTreeMap::<String, u64>::new();
        for (_, verdict, _) in &answer.subtree {
            *tally.entry(verdict.clone()).or_default() += 1;
        }
        let got: std::collections::BTreeMap<String, u64> = s
            .subtree_tally(tree, a as u64)
            .unwrap()
            .into_iter()
            .map(|v| (v.verdict.as_str().to_string(), v.count))
            .collect();
        assert_eq!(got, tally, "tally of node {a} includes the node itself");

        let first = answer
            .subtree
            .iter()
            .filter(|(_, v, _)| v == "violation")
            .min_by_key(|(node, _, depth)| (*depth, *node))
            .map(|(node, _, _)| *node);
        assert_eq!(
            s.subtree_first_failing(tree, a as u64).unwrap(),
            first,
            "first failing node under {a}"
        );

        let walked: Vec<u64> = model_ancestors(spec, a).iter().map(|x| *x as u64).collect();
        assert_eq!(
            answer.ancestors, walked,
            "recursive query walks the same chain"
        );
        let chain: Vec<u64> = s
            .tree_ancestors(tree, a as u64)
            .unwrap()
            .iter()
            .rev()
            .map(|r| r.node)
            .collect();
        assert_eq!(chain, walked, "ancestors of node {a} ({rule:?})");
        // The stored path encodes the same chain, root first.
        let stored = s.get_tree_node(tree, a as u64).unwrap().unwrap();
        let mut from_path = decode_path(&stored.path).expect("a whole path");
        assert_eq!(from_path.pop(), Some(a as u64));
        let mut root_first = walked.clone();
        root_first.reverse();
        assert_eq!(
            from_path, root_first,
            "the path of node {a} names its ancestors"
        );

        let kids: Vec<u64> = (0..n)
            .filter(|i| spec.parents[*i] == Some(a))
            .map(|i| i as u64)
            .collect();
        let mut listed = Vec::new();
        let mut cursor = None;
        loop {
            let page = s.tree_children(tree, a as u64, cursor, 5).unwrap();
            listed.extend(page.nodes.iter().map(|r| r.node));
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(listed, kids, "children of node {a}");
    }
}

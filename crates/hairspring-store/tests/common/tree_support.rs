//! Builders shared by the trial tree scenarios.
#![allow(dead_code)]
use hairspring_store::*;

/// The two forms every subtree query exists in.
pub const FORMS: [TreeQueryRule; 2] = [TreeQueryRule::AlwaysCte, TreeQueryRule::AlwaysPath];

/// Runs `check` once with the store forced to each form, then puts the store's own rule back.
pub fn in_each_form(s: &dyn Store, check: &mut dyn FnMut(TreeQueryRule)) {
    for rule in FORMS {
        s.set_tree_query_rule(rule);
        check(rule);
    }
    s.set_tree_query_rule(TreeQueryRule::StoreDefault);
}

pub fn new_run(s: &dyn Store, backend: &str) -> RunRecord {
    s.create_run(
        &NewRun {
            target: "t".into(),
            backend: backend.into(),
            seed: 7,
            config_json: "{}".into(),
        },
        1,
    )
    .unwrap()
}

fn trial(seq: u64, verdict: TrialVerdict) -> NewTrial {
    NewTrial {
        seq,
        seed: 100 + seq,
        verdict,
        reason: String::new(),
        exit_code: Some(0),
        signal: None,
        timed_out: false,
        degraded: false,
        perturbing: false,
        report_missing: false,
        rated: true,
        claimable: true,
        behaviour_hash: None,
        schedule_fp: None,
        syscalls: 1,
        work_ops: 1,
        elapsed_ms: 1,
        max_idx: None,
        digest: vec![1, 0, 0, 0],
        detail: None,
        output: None,
    }
}

/// One batch of trials, one per verdict, numbered from `first_seq`.
pub fn batch(verdicts: &[TrialVerdict], first_seq: u64) -> TrialBatch {
    TrialBatch {
        catalog: vec![],
        trials: verdicts
            .iter()
            .enumerate()
            .map(|(i, v)| trial(first_seq + i as u64, *v))
            .collect(),
        kept_bytes_cap: MAX_KEPT_BYTES_PER_RUN,
    }
}

/// Stores one batch for the run through a freshly leased job; returns the job.
pub fn ingest_batch(s: &dyn Store, run_id: &str, b: &TrialBatch) -> Job {
    s.enqueue_job(run_id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w", 600, 10).unwrap().unwrap();
    assert_eq!(job.run_id, run_id);
    let out = s.ingest_trials(&job.id, "w", true, b, 11).unwrap();
    assert!(matches!(out, IngestOutcome::Applied { .. }), "{out:?}");
    job
}

/// Stores one trial per verdict (`seq` = position) for the run through a leased job, as a worker would.
pub fn ingest_verdicts(s: &dyn Store, run_id: &str, verdicts: &[TrialVerdict]) {
    s.enqueue_job(run_id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w", 600, 10).unwrap().unwrap();
    assert_eq!(job.run_id, run_id);
    for (chunk_no, chunk) in verdicts.chunks(200).enumerate() {
        let b = batch(chunk, (chunk_no * 200) as u64);
        let out = s.ingest_trials(&job.id, "w", true, &b, 11).unwrap();
        assert!(matches!(out, IngestOutcome::Applied { .. }), "{out:?}");
    }
}

pub fn passes(n: usize) -> Vec<TrialVerdict> {
    vec![TrialVerdict::Pass; n]
}

pub fn identity(run_id: &str, text: &str, complete: bool) -> RunIdentity {
    RunIdentity {
        run_id: run_id.into(),
        scheme: "sim/1".into(),
        identity_text: text.into(),
        identity_hash: sha256_hex(text.as_bytes()),
        complete,
        reporter_id: "w".into(),
        reporter_trusted: true,
    }
}

pub fn root(run_id: &str, seq: u64) -> NewNode {
    NewNode {
        run_id: run_id.into(),
        seq,
        parent_node: None,
        operator: Operator::Root,
        cut_kind: CutKind::None,
        cut_at: 0,
        suffix_seed: None,
        edge_ref: None,
        origin: NodeOrigin::Server,
        reporter_id: "w".into(),
        reporter_trusted: true,
    }
}

pub fn child(run_id: &str, seq: u64, parent: u64, cut_at: u64) -> NewNode {
    NewNode {
        run_id: run_id.into(),
        seq,
        parent_node: Some(parent),
        operator: Operator::Explore,
        cut_kind: CutKind::Step,
        cut_at,
        suffix_seed: None,
        edge_ref: None,
        origin: NodeOrigin::Exploration,
        reporter_id: "w".into(),
        reporter_trusted: true,
    }
}

/// A run with an identity, ready to root a tree.
pub fn run_with_identity(s: &dyn Store, text: &str) -> RunRecord {
    let run = new_run(s, "sim");
    assert_eq!(
        s.put_run_identity(&identity(&run.id, text, true)).unwrap(),
        IdentityOutcome::Stored
    );
    run
}

pub fn make_tree(s: &dyn Store, run: &RunRecord, identity_hash: Option<String>) -> TreeRecord {
    let new = NewTree {
        run_id: run.id.clone(),
        backend: run.backend.clone(),
        target: run.target.clone(),
        identity_hash,
    };
    match s.create_tree(&new, 5).unwrap() {
        TreeOutcome::Created(t) => t,
        other => panic!("tree not created: {other:?}"),
    }
}

pub fn add(s: &dyn Store, tree: &str, node: &NewNode) -> NodeRecord {
    match s.add_tree_node(tree, node, 6).unwrap() {
        NodeOutcome::Added(n) => n,
        other => panic!("node refused: {other:?}"),
    }
}

pub fn branch_for(tree: &str, run_id: &str, source_node: u64, hash: &str) -> NewBranch {
    NewBranch {
        run_id: run_id.into(),
        tree_id: tree.into(),
        source_node,
        cut_kind: CutKind::Step,
        cut_at: 3,
        point_hash: "p".repeat(64),
        command_json: "{\"verb\":\"step\"}".into(),
        command_hash: hash.into(),
        seed_source: SeedSource::ServerRandom,
        created_by: "key_ab12".into(),
    }
}

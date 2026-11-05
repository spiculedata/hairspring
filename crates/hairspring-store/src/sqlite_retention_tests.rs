//! The backstop in `delete_unit`, which no public call can reach while the tree rule holds.
use super::*;
use crate::*;

fn run_with_identity(s: &SqliteStore) -> RunRecord {
    let run = s
        .create_run(
            &NewRun {
                target: "t".into(),
                backend: "sim".into(),
                seed: 7,
                config_json: "{}".into(),
            },
            1,
        )
        .unwrap();
    let id = RunIdentity {
        run_id: run.id.clone(),
        scheme: "sim/1".into(),
        identity_text: "a\n".into(),
        identity_hash: sha256_hex(b"a\n"),
        complete: true,
        reporter_id: "w".into(),
        reporter_trusted: true,
    };
    assert_eq!(s.put_run_identity(&id).unwrap(), IdentityOutcome::Stored);
    run
}

fn one_trial() -> NewTrial {
    NewTrial {
        seq: 0,
        seed: 100,
        verdict: TrialVerdict::Pass,
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
        on_digest_cap: None,
    }
}

/// A finished root run with one node, rooting a tree that has one branch run. Returns (tree, branch run).
fn tree_with_a_branch(s: &SqliteStore) -> (String, String) {
    let root = run_with_identity(s);
    s.enqueue_job(&root.id, "run", "{}", 2).unwrap();
    let job = s.lease_job("w", 600, 10).unwrap().unwrap();
    let b = TrialBatch {
        catalog: vec![],
        trials: vec![one_trial()],
        kept_bytes_cap: MAX_KEPT_BYTES_PER_RUN,
        digest_bytes_cap: MAX_DIGEST_BYTES_PER_RUN,
    };
    s.ingest_trials(&job.id, "w", true, &b, 11).unwrap();
    assert!(s.complete_job(&job.id, "w").unwrap());
    let new_tree = NewTree {
        run_id: root.id.clone(),
        backend: root.backend.clone(),
        target: root.target.clone(),
        identity_hash: Some(sha256_hex(b"a\n")),
    };
    assert!(matches!(
        s.create_tree(&new_tree, 5).unwrap(),
        TreeOutcome::Created(_)
    ));
    let node = NewNode {
        run_id: root.id.clone(),
        seq: 0,
        parent_node: None,
        operator: Operator::Root,
        cut_kind: CutKind::None,
        cut_at: 0,
        suffix_seed: None,
        edge_ref: None,
        origin: NodeOrigin::Server,
        reporter_id: "w".into(),
        reporter_trusted: true,
    };
    assert!(matches!(
        s.add_tree_node(&root.id, &node, 6).unwrap(),
        NodeOutcome::Added(_)
    ));
    assert!(s
        .finish_run(&root.id, RunState::Passed, 1, 1, None, 3)
        .unwrap());
    let branch = run_with_identity(s);
    let new = NewBranch {
        run_id: branch.id.clone(),
        tree_id: root.id.clone(),
        source_node: 0,
        cut_kind: CutKind::Step,
        cut_at: 3,
        point_hash: "a".repeat(64),
        command_json: "{\"verb\":\"step\"}".into(),
        command_hash: "b".repeat(64),
        seed_source: SeedSource::ServerRandom,
        created_by: "key_ab12".into(),
    };
    assert!(matches!(
        s.add_tree_branch(&new, 9).unwrap(),
        BranchOutcome::Added(_)
    ));
    (root.id, branch.id)
}

/// Asked to delete a tree with only some of its runs, `delete_unit` fails and the rolled-back transaction leaves the
/// whole tree in place.
#[test]
fn a_tree_with_rows_for_runs_outside_the_unit_is_not_deleted() {
    let s = SqliteStore::open_in_memory().unwrap();
    let (tree, branch) = tree_with_a_branch(&s);
    {
        let mut conn = s.conn();
        let tx = write_tx(&mut conn).unwrap();
        let err = Ops { tx: &tx }
            .delete_unit(Some(&tree), std::slice::from_ref(&tree))
            .unwrap_err();
        assert!(err.to_string().contains("not judged with it"), "{err}");
    }
    assert!(s.get_tree(&tree).unwrap().is_some());
    assert!(s.get_tree_branch(&branch).unwrap().is_some());
    assert!(s.get_run(&tree).unwrap().is_some());
    assert!(s.get_tree_node(&tree, 0).unwrap().is_some());
}

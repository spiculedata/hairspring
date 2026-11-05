//! Retention (`Store::retention_scan` and `Store::retention_delete`): every `Store` must delete the same things, keep
//! the same things, and report the same counts.
use super::tree_support::{batch, ingest_batch, passes};
use hairspring_store::*;
use std::sync::Arc;

/// Everything finished before this is old.
const CUTOFF: i64 = 10_000;
const OLD: i64 = 1_000;
const NEW: i64 = 20_000;

pub fn run_all_retention(make: &dyn Fn() -> Arc<dyn Store>) {
    scan_lists_old_runs_of_every_state_with_the_time_they_age_from(&*make());
    only_finished_old_runs_without_a_live_job_are_deleted(&*make());
    the_cutoff_is_exclusive(&*make());
    unnamed_and_unknown_runs_are_left_alone(&*make());
    a_dry_run_changes_nothing_and_reports_what_an_apply_removes(&*make());
    every_row_that_belonged_to_a_run_goes_with_it(&*make());
    shared_blobs_stay_while_anything_refers_to_them(&*make());
    a_blob_named_by_a_kept_check_stays(&*make());
    a_blob_named_only_by_a_kept_tree_edge_stays(&*make());
    deleting_twice_removes_nothing_the_second_time(&*make());
    blobs_shared_across_many_runs_go_once_nothing_left_refers_to_them(&*make());
    a_blob_inside_a_kept_checks_artefacts_stays_wherever_it_appears(&*make());
    a_blob_named_only_by_the_removed_runs_own_check_goes(&*make());
    trial_output_and_detail_a_kept_run_shares_stay(&*make());
    a_tree_is_one_unit(make);
}

/// Stored output and detail are shared by content too: the old run's copy goes with it only when no trial of a run
/// that stays names the same blob, whichever column names it.
fn trial_output_and_detail_a_kept_run_shares_stay(s: &dyn Store) {
    let with_trials = |at: i64, output: &[u8], detail: &[u8]| {
        let run = new_run(s, at - 1);
        let mut b = batch(&passes(2), 0);
        b.trials[0].output = Some(output.to_vec());
        b.trials[1].detail = Some(detail.to_vec());
        let job = ingest_batch(s, &run.id, &b);
        assert!(s.complete_job(&job.id, "w").unwrap());
        assert!(s
            .finish_run(&run.id, RunState::Passed, 1, 2, None, at)
            .unwrap());
        run
    };
    let old = with_trials(OLD, b"output both have", b"detail both have");
    let kept = with_trials(NEW, b"detail both have", b"output both have");
    let lone = with_trials(OLD, b"output of the lone run", b"detail of the lone run");
    let dry = delete(s, &ids(&[&old, &lone, &kept]), true);
    let report = delete(s, &ids(&[&old, &lone, &kept]), false);
    assert_eq!((dry.blobs, report.blobs), (2, 2), "only the lone run's");
    for shared in [b"output both have".as_slice(), b"detail both have"] {
        assert!(s.get_blob(&sha256_hex(shared)).unwrap().is_some());
    }
    assert!(s
        .get_blob(&sha256_hex(b"output of the lone run"))
        .unwrap()
        .is_none());
}

/// More runs than one transaction of an apply takes, sharing blobs with each other and with a run that stays. The dry
/// run (one snapshot, nothing removed) and the apply (several transactions) agree, and only the blobs nothing left
/// refers to go.
fn blobs_shared_across_many_runs_go_once_nothing_left_refers_to_them(s: &dyn Store) {
    const RUNS: usize = 45;
    let kept = finished(s, RunState::Failed, NEW);
    let (_, kept_shared) = failure_with_bundle(s, &kept.id, b"the kept run and two old runs");
    let mut old = Vec::new();
    let mut own = Vec::new();
    let mut shared = String::new();
    for i in 0..RUNS {
        let run = finished(s, RunState::Failed, OLD);
        shared = failure_with_bundle(s, &run.id, b"every old run").1;
        own.push(failure_with_bundle(s, &run.id, format!("old run {i}").as_bytes()).1);
        if i == 0 || i == RUNS - 1 {
            failure_with_bundle(s, &run.id, b"the kept run and two old runs");
        }
        old.push(run.id);
    }
    let mut named = old.clone();
    named.push(kept.id.clone());

    let dry = delete(s, &named, true);
    let done = delete(s, &named, false);

    let sorted = |mut r: RetentionReport| {
        r.applied = false;
        r.bundle_hashes.sort();
        r.run_ids.sort();
        r
    };
    assert_eq!(sorted(done.clone()), sorted(dry), "the dry run matches");
    assert_eq!(done.runs_deleted(), RUNS as u64);
    assert_eq!(done.error, None);
    assert_eq!(
        (done.blobs, done.bundle_blobs),
        (RUNS as u64 + 1, RUNS as u64 + 1),
        "the shared bundle and each run's own"
    );
    assert!(s.get_blob(&shared).unwrap().is_none());
    assert!(own.iter().all(|h| s.get_blob(h).unwrap().is_none()));
    assert!(
        s.get_blob(&kept_shared).unwrap().is_some(),
        "the kept run still names it"
    );
}

/// A stored check's artefact list is searched as text: a hash counts wherever it appears, inside a longer string or
/// a longer run of hex digits too.
fn a_blob_inside_a_kept_checks_artefacts_stays_wherever_it_appears(s: &dyn Store) {
    let old = finished(s, RunState::Failed, OLD);
    let kept = finished(s, RunState::Failed, NEW);
    let (_, first) = failure_with_bundle(s, &old.id, b"first old bundle");
    let (_, second) = failure_with_bundle(s, &old.id, b"second old bundle");
    let (_, free) = failure_with_bundle(s, &old.id, b"named by nothing");
    let (kept_failure, _) = failure_with_bundle(s, &kept.id, b"kept bundle");
    let artefacts = format!("[\"path/{first}.out\",\"a{second}0b\"]");
    confirm(s, &kept_failure.id, "conf_inside", &artefacts);
    let dry = delete(s, &ids(&[&old]), true);
    let report = delete(s, &ids(&[&old]), false);
    assert_eq!(report.bundle_hashes, vec![free.clone()]);
    assert_eq!(dry.bundle_hashes, vec![free.clone()]);
    assert!(s.get_blob(&first).unwrap().is_some());
    assert!(s.get_blob(&second).unwrap().is_some());
    assert!(s.get_blob(&free).unwrap().is_none());
}

/// A check that goes with its failure is not a reference, in the dry run or the apply.
fn a_blob_named_only_by_the_removed_runs_own_check_goes(s: &dyn Store) {
    let old = finished(s, RunState::Failed, OLD);
    let (f, bundle) = failure_with_bundle(s, &old.id, b"named by its own check");
    confirm(s, &f.id, "conf_own", &format!("[\"{bundle}\"]"));
    let dry = delete(s, &ids(&[&old]), true);
    assert_eq!(dry.bundle_hashes, vec![bundle.clone()]);
    let report = delete(s, &ids(&[&old]), false);
    assert_eq!(report.bundle_hashes, vec![bundle.clone()]);
    assert_eq!(report.checks, 1);
    assert!(s.get_blob(&bundle).unwrap().is_none());
}

fn new_run(s: &dyn Store, created: i64) -> RunRecord {
    s.create_run(
        &NewRun {
            target: "t".into(),
            backend: "sim".into(),
            seed: 7,
            config_json: "{}".into(),
        },
        created,
    )
    .unwrap()
}

fn finished(s: &dyn Store, state: RunState, at: i64) -> RunRecord {
    let run = new_run(s, at - 1);
    assert!(s.finish_run(&run.id, state, 5, 1, None, at).unwrap());
    run
}

fn failure_with_bundle(s: &dyn Store, run: &str, bundle: &[u8]) -> (FailureRecord, String) {
    let hash = s.put_blob(bundle).unwrap();
    let f = s
        .add_failure(
            &NewFailure {
                run_id: run.into(),
                target: "t".into(),
                message: "m".into(),
                original_len: 4,
                shrunk_len: 2,
                bundle_hash: hash.clone(),
                steps_json: "[]".into(),
            },
            5,
        )
        .unwrap();
    (f, hash)
}

fn check_with_artefacts(artefacts: &str) -> NewCheck {
    NewCheck {
        kind: "same_seed_rerun".into(),
        status: "pass".into(),
        passed: 1,
        failed: 0,
        inconclusive: 0,
        command: "c".into(),
        detail_json: "{}".into(),
        artefacts_json: artefacts.into(),
        environment_json: "{}".into(),
        started_at: 1,
        finished_at: 2,
    }
}

fn confirm(s: &dyn Store, failure: &str, id: &str, artefacts: &str) {
    let c = NewConfirmation {
        id: id.into(),
        failure_id: failure.into(),
        runner: "r".into(),
        runner_trusted: true,
        checks: vec![check_with_artefacts(artefacts)],
    };
    assert_eq!(
        s.add_confirmation(&c, 7).unwrap(),
        ConfirmationOutcome::Stored
    );
}

fn ids(v: &[&RunRecord]) -> Vec<String> {
    v.iter().map(|r| r.id.clone()).collect()
}

fn delete(s: &dyn Store, run_ids: &[String], dry_run: bool) -> RetentionReport {
    s.retention_delete(run_ids, CUTOFF, dry_run).unwrap()
}

fn scan_lists_old_runs_of_every_state_with_the_time_they_age_from(s: &dyn Store) {
    let passed = finished(s, RunState::Passed, OLD);
    let queued = new_run(s, OLD);
    let running = new_run(s, OLD + 1);
    s.mark_running(&running.id, OLD + 2).unwrap();
    let too_new = finished(s, RunState::Failed, NEW);
    let mut got = s.retention_scan(CUTOFF).unwrap();
    got.sort_by(|a, b| a.id.cmp(&b.id));
    let mut want = vec![
        (passed.id.clone(), RunState::Passed, OLD),
        (queued.id.clone(), RunState::Queued, OLD),
        (running.id.clone(), RunState::Running, OLD + 1),
    ];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    let got: Vec<_> = got
        .into_iter()
        .map(|c| (c.id, c.state, c.aged_from))
        .collect();
    assert_eq!(
        got, want,
        "the new run is not listed; unfinished runs age from creation"
    );
    assert!(!got.iter().any(|(id, _, _)| *id == too_new.id));
}

fn only_finished_old_runs_without_a_live_job_are_deleted(s: &dyn Store) {
    let gone: Vec<RunRecord> = [
        RunState::Passed,
        RunState::Failed,
        RunState::Error,
        RunState::Cancelled,
    ]
    .into_iter()
    .map(|st| finished(s, st, OLD))
    .collect();
    let queued = new_run(s, OLD);
    let running = new_run(s, OLD);
    s.mark_running(&running.id, OLD).unwrap();
    let with_job = finished(s, RunState::Passed, OLD);
    s.enqueue_job(&with_job.id, "run", "{}", OLD).unwrap();
    let too_new = finished(s, RunState::Passed, NEW);
    let all: Vec<String> = gone
        .iter()
        .chain([&queued, &running, &with_job, &too_new])
        .map(|r| r.id.clone())
        .collect();

    let report = delete(s, &all, false);
    assert!(report.applied);
    assert_eq!(report.runs_deleted(), 4);
    assert_eq!(
        report
            .runs_by_state
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect::<Vec<_>>(),
        [("cancelled", 1), ("error", 1), ("failed", 1), ("passed", 1)]
    );
    assert_eq!(
        report
            .kept
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect::<Vec<_>>(),
        [("active", 2), ("active_job", 1), ("too_new", 1)]
    );
    for r in &gone {
        assert!(s.get_run(&r.id).unwrap().is_none());
    }
    for r in [&queued, &running, &with_job, &too_new] {
        assert!(s.get_run(&r.id).unwrap().is_some(), "{} is kept", r.id);
    }
    assert_eq!(
        s.get_run(&running.id).unwrap().unwrap().state,
        RunState::Running
    );
}

fn the_cutoff_is_exclusive(s: &dyn Store) {
    let at = finished(s, RunState::Passed, CUTOFF);
    let before = finished(s, RunState::Passed, CUTOFF - 1);
    let report = delete(s, &ids(&[&at, &before]), false);
    assert_eq!(report.run_ids, vec![before.id.clone()]);
    assert!(s.get_run(&at.id).unwrap().is_some());
}

fn unnamed_and_unknown_runs_are_left_alone(s: &dyn Store) {
    let named = finished(s, RunState::Passed, OLD);
    let unnamed = finished(s, RunState::Passed, OLD);
    let report = delete(s, &[named.id.clone(), "run_missing".into()], false);
    assert_eq!(report.run_ids, vec![named.id.clone()]);
    assert_eq!(report.kept.get("not_found"), Some(&1));
    assert!(s.get_run(&unnamed.id).unwrap().is_some());
    assert_eq!(
        delete(s, &[], false),
        RetentionReport {
            applied: true,
            ..Default::default()
        }
    );
}

/// One finished failed run with every kind of row a run can own, and a run that shares its bundle.
fn busy_run(s: &dyn Store) -> RunRecord {
    let run = new_run(s, OLD - 1);
    let mut b = batch(&passes(3), 0);
    b.trials[0].output = Some(b"stored output one".to_vec());
    b.trials[1].detail = Some(b"detail".to_vec());
    let job = ingest_batch(s, &run.id, &b);
    s.add_event(&run.id, "progress", "{}", OLD).unwrap();
    s.add_event(&run.id, "finished", "{}", OLD).unwrap();
    let (f, _) = failure_with_bundle(s, &run.id, b"the bundle of the busy run");
    confirm(s, &f.id, "conf_busy", "[]");
    s.finish_run(&run.id, RunState::Failed, 5, 3, None, OLD)
        .unwrap();
    // A worker closes its job when it finishes; ingest_batch leaves it running.
    assert!(s.complete_job(&job.id, "w").unwrap());
    run
}

fn count_rows(s: &dyn Store, run: &RunRecord) -> (usize, usize, u64, usize) {
    (
        s.list_failures(&run.id).unwrap().len(),
        s.list_events(&run.id, 0, 100).unwrap().len(),
        s.count_trials(&run.id).unwrap(),
        s.get_catalog(&run.id).unwrap().len(),
    )
}

fn a_dry_run_changes_nothing_and_reports_what_an_apply_removes(s: &dyn Store) {
    let run = busy_run(s);
    let before = count_rows(s, &run);
    assert_eq!(before.0, 1);
    let dry = delete(s, std::slice::from_ref(&run.id), true);
    assert!(!dry.applied);
    assert_eq!(count_rows(s, &run), before, "a dry run leaves every row");
    assert!(s.get_run(&run.id).unwrap().is_some());
    let failure = &s.list_failures(&run.id).unwrap()[0];
    assert!(s.get_blob(&failure.bundle_hash).unwrap().is_some());

    let done = delete(s, std::slice::from_ref(&run.id), false);
    assert!(done.applied);
    let strip = |mut r: RetentionReport| {
        r.applied = false;
        r
    };
    assert_eq!(
        strip(done),
        dry,
        "the dry run's report is what the apply removed"
    );
}

fn every_row_that_belonged_to_a_run_goes_with_it(s: &dyn Store) {
    let run = busy_run(s);
    let failure = s.list_failures(&run.id).unwrap().remove(0);
    let trials = s.list_trials(&run.id, None, 10).unwrap();
    let output_ref = trials[0].output_ref.clone().unwrap();
    let detail_ref = trials[1].detail_ref.clone().unwrap();
    let report = delete(s, std::slice::from_ref(&run.id), false);
    assert_eq!(
        (report.failures, report.checks, report.trials, report.events),
        (1, 1, 3, 2)
    );
    assert_eq!(report.jobs, 1);
    assert_eq!(report.blobs, 3, "bundle, output and detail");
    assert_eq!(report.bundle_blobs, 1);
    assert_eq!(report.bundle_hashes, vec![failure.bundle_hash.clone()]);
    let want_bytes =
        (b"the bundle of the busy run".len() + b"stored output one".len() + b"detail".len()) as u64;
    assert_eq!(report.blob_bytes, want_bytes);
    assert_eq!(
        report.bundle_bytes,
        b"the bundle of the busy run".len() as u64
    );

    assert!(s.get_run(&run.id).unwrap().is_none());
    assert!(s.get_failure(&failure.id).unwrap().is_none());
    assert!(s.list_failures(&run.id).unwrap().is_empty());
    assert!(s.list_failure_checks(&failure.id).unwrap().is_empty());
    assert!(s.list_events(&run.id, 0, 10).unwrap().is_empty());
    assert_eq!(s.count_trials(&run.id).unwrap(), 0);
    assert!(s.get_catalog(&run.id).unwrap().is_empty());
    assert!(s.list_run_properties(&run.id).unwrap().is_empty());
    for hash in [&failure.bundle_hash, &output_ref, &detail_ref] {
        assert!(
            s.get_blob(hash).unwrap().is_none(),
            "no blob outlives its last reference"
        );
    }
}

fn shared_blobs_stay_while_anything_refers_to_them(s: &dyn Store) {
    let old = finished(s, RunState::Failed, OLD);
    let kept = finished(s, RunState::Failed, NEW);
    let (_, shared) = failure_with_bundle(s, &old.id, b"same bundle text");
    let (kept_failure, again) = failure_with_bundle(s, &kept.id, b"same bundle text");
    assert_eq!(shared, again, "content addressed");
    let (_, own) = failure_with_bundle(s, &old.id, b"only the old run has this");
    let report = delete(s, &ids(&[&old, &kept]), false);
    assert_eq!(report.run_ids, vec![old.id.clone()]);
    assert_eq!((report.blobs, report.bundle_blobs), (1, 1));
    assert_eq!(report.bundle_hashes, vec![own.clone()]);
    assert!(
        s.get_blob(&shared).unwrap().is_some(),
        "the kept run still names it"
    );
    assert!(s.get_blob(&own).unwrap().is_none());
    assert_eq!(s.list_failures(&kept.id).unwrap()[0].id, kept_failure.id);
}

fn a_blob_named_by_a_kept_check_stays(s: &dyn Store) {
    let old = finished(s, RunState::Failed, OLD);
    let kept = finished(s, RunState::Failed, NEW);
    let (_, doomed) = failure_with_bundle(s, &old.id, b"old bundle");
    let (kept_failure, _) = failure_with_bundle(s, &kept.id, b"kept bundle");
    confirm(s, &kept_failure.id, "conf_kept", &format!("[\"{doomed}\"]"));
    let report = delete(s, &ids(&[&old]), false);
    assert_eq!(report.runs_deleted(), 1);
    assert_eq!(report.blobs, 0, "a kept check's artefact is a reference");
    assert!(s.get_blob(&doomed).unwrap().is_some());
}

/// The tree edge clause of the blob guard: the blob is the old run's bundle and, by identical content, the edge of a
/// node in a tree that stays. Only that edge refers to it once the old run is gone.
fn a_blob_named_only_by_a_kept_tree_edge_stays(s: &dyn Store) {
    use super::tree_support::{
        add, child, finish, ingest_verdicts, make_tree, root, run_with_identity,
    };
    let old = finished(s, RunState::Failed, OLD);
    let (_, shared) = failure_with_bundle(s, &old.id, b"edge and bundle alike");
    let kept_root = run_with_identity(s, "e\n");
    ingest_verdicts(s, &kept_root.id, &passes(2));
    let tree = make_tree(s, &kept_root, Some(sha256_hex(b"e\n")));
    add(s, &tree.id, &root(&kept_root.id, 0));
    let mut edge = child(&kept_root.id, 1, 0, 1);
    edge.edge_ref = Some(shared.clone());
    add(s, &tree.id, &edge);
    finish(s, &kept_root.id);
    let report = delete(s, &ids(&[&old]), false);
    assert_eq!(report.run_ids, vec![old.id.clone()]);
    assert_eq!(report.blobs, 0, "the kept tree's edge is a reference");
    assert!(s.get_blob(&shared).unwrap().is_some());
    assert!(s.get_tree(&tree.id).unwrap().is_some());
}

fn deleting_twice_removes_nothing_the_second_time(s: &dyn Store) {
    let run = finished(s, RunState::Passed, OLD);
    assert_eq!(
        delete(s, std::slice::from_ref(&run.id), false).runs_deleted(),
        1
    );
    let again = delete(s, std::slice::from_ref(&run.id), false);
    assert_eq!(again.runs_deleted(), 0);
    assert_eq!(again.kept.get("not_found"), Some(&1));
}

fn a_tree_is_one_unit(make: &dyn Fn() -> Arc<dyn Store>) {
    use super::tree_gc_spec::*;
    a_tree_is_kept_while_any_of_its_runs_is_kept(&*make(), &real_gc);
    a_tree_is_deleted_in_one_unit_when_every_run_has_expired(&*make(), &real_gc);
}

/// The routine the tree specification is written against, over the real store call.
pub fn real_gc(s: &dyn Store, expired: &[String]) -> super::tree_gc_spec::GcReport {
    let r = s.retention_delete(expired, i64::MAX, false).unwrap();
    let runs_deleted = r.runs_deleted();
    super::tree_gc_spec::GcReport {
        trees_deleted: r.trees_deleted,
        trees_kept: r.trees_kept,
        runs_deleted,
        nodes_deleted: r.nodes,
    }
}

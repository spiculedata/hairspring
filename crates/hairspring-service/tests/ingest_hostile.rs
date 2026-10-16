//! Hostile input: every case sends a batch a compromised worker might send and expects it refused, with nothing stored.
mod common;

use common::*;
use hairspring_engine::config::MAX_TRIALS_CAP;
use hairspring_engine::trial_log::TrialReport;
use hairspring_props::{AssertKind, DigestEntry, TrialDigest, Witness};
use hairspring_service::{LeasedJob, Service, ServiceError};
use hairspring_store::Store;
use std::sync::Arc;

fn refused(svc: &Service, store: &Arc<dyn Store>, job: &LeasedJob, batch: &[TrialReport]) {
    let err = svc.ingest_trials(&job.job_id, &local(), batch).unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err}");
    assert_eq!(
        store.count_trials(&job.run_id).unwrap(),
        0,
        "nothing stored"
    );
    assert!(
        store.get_catalog(&job.run_id).unwrap().is_empty(),
        "no catalog rows either"
    );
}

fn one(seed: u64) -> TrialReport {
    Recorder::new().pass(seed, &props("a.id", AssertKind::Always, 3, 0))
}

#[test]
fn hostile_digests_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let mut garbage = good.clone();
    garbage.digest = vec![9, 9, 9];
    refused(&svc, &store, &job, &[garbage]);
    let mut forged_version = good.clone();
    forged_version.digest[0] = 7;
    refused(&svc, &store, &job, &[forged_version]);
    let mut empty = good.clone();
    empty.digest = Vec::new();
    refused(&svc, &store, &job, &[empty]);
    // An index the catalog does not hold.
    let mut past = good.clone();
    let mut d = TrialDigest::decode(&past.digest).unwrap();
    d.entries[0].idx = 5;
    past.digest = d.encode();
    refused(&svc, &store, &job, &[past]);
    // Counts that do not add up.
    let mut bad_counts = good.clone();
    let mut d = TrialDigest::decode(&bad_counts.digest).unwrap();
    d.entries[0].hits += 1;
    bad_counts.digest = d.encode();
    refused(&svc, &store, &job, &[bad_counts]);
    // More witnesses than allowed.
    let mut many = good.clone();
    let entries: Vec<DigestEntry> = (0..65)
        .map(|i| DigestEntry {
            idx: if i == 0 { 0 } else { i },
            kind: AssertKind::Always,
            catalogued: true,
            conflicted: false,
            hits: 1,
            trues: 0,
            falses: 1,
            witness: Some(Witness {
                proc: "p1".into(),
                thread: "w".into(),
                ord: 1,
                vals: [0; 4],
            }),
            decl: None,
        })
        .collect();
    many.digest = TrialDigest {
        entries,
        ..TrialDigest::default()
    }
    .encode();
    refused(&svc, &store, &job, &[many]);
    // An excluded trial carries no entries.
    let mut excluded = good.clone();
    let mut d = TrialDigest::decode(&excluded.digest).unwrap();
    d.flags.excluded = true;
    excluded.digest = d.encode();
    refused(&svc, &store, &job, &[excluded]);
}

#[test]
fn hostile_catalog_updates_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let mutate = |f: &dyn Fn(&mut TrialReport)| {
        let mut r = good.clone();
        f(&mut r);
        r
    };
    let oversize = mutate(&|r| r.catalog[0].row.id = "x".repeat(129));
    let control = mutate(&|r| r.catalog[0].row.id = "a\u{1}b".to_string());
    let bidi = mutate(&|r| r.catalog[0].row.msg = "evil \u{202e} text".to_string());
    let long_msg = mutate(&|r| r.catalog[0].row.msg = "m".repeat(513));
    let gap = mutate(&|r| r.catalog[0].idx = 3);
    let nul = mutate(&|r| r.catalog[0].row.msg = "a\0b".to_string());
    for r in [oversize, control, bidi, long_msg, gap, nul] {
        refused(&svc, &store, &job, &[r]);
    }
    let mut twice = good.clone();
    let mut again = twice.catalog[0].clone();
    again.idx = 1;
    twice.catalog.push(again);
    refused(&svc, &store, &job, &[twice]);
}

#[test]
fn hostile_row_fields_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    type Mutation = Box<dyn Fn(&mut TrialReport)>;
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "a sequence number past the budget",
            Box::new(|r| r.seq = MAX_TRIALS_CAP),
        ),
        ("a huge sequence number", Box::new(|r| r.seq = u64::MAX)),
        ("a long reason", Box::new(|r| r.reason = "r".repeat(501))),
        (
            "a reason with a newline",
            Box::new(|r| r.reason = "a\nb".to_string()),
        ),
        (
            "a reason with an escape",
            Box::new(|r| r.reason = "a\u{1b}[31m".to_string()),
        ),
        (
            "a bidi reason",
            Box::new(|r| r.reason = "a\u{202e}b".to_string()),
        ),
        (
            "a hash with punctuation",
            Box::new(|r| r.behaviour_hash = Some("x'; --".to_string())),
        ),
        (
            "a long fingerprint",
            Box::new(|r| r.schedule_fp = Some("a".repeat(65))),
        ),
        (
            "counters past the signed range",
            Box::new(|r| r.syscalls = u64::MAX),
        ),
        (
            "an elapsed time past the signed range",
            Box::new(|r| r.elapsed_ms = u64::MAX),
        ),
        (
            "a pass that timed out",
            Box::new(|r| {
                let mut d = TrialDigest::decode(&r.digest).unwrap();
                d.flags.inconclusive = true;
                r.digest = d.encode();
            }),
        ),
        (
            "a pass whose property failed",
            Box::new(|r| {
                let mut d = TrialDigest::decode(&r.digest).unwrap();
                d.entries[0].falses = 1;
                d.entries[0].hits = d.entries[0].trues + 1;
                r.digest = d.encode();
            }),
        ),
    ];
    for (what, f) in cases {
        let mut r = good.clone();
        f(&mut r);
        let err = svc.ingest_trials(&job.job_id, &local(), &[r]).unwrap_err();
        assert!(
            matches!(err, ServiceError::InvalidRequest(_)),
            "{what}: {err}"
        );
        assert_eq!(store.count_trials(&job.run_id).unwrap(), 0, "{what}");
    }
}

#[test]
fn hostile_detail_and_output_are_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    let detail = |s: String| {
        let mut r = good.clone();
        r.detail = Some(s);
        r
    };
    let output = |s: String| {
        let mut r = good.clone();
        r.output = Some(s);
        r
    };
    let cases = vec![
        detail("not json".to_string()),
        detail("{\"v\":9}".to_string()),
        detail(" ".repeat(256 * 1024 + 1)),
        output("not json".to_string()),
        output("[1,2]".to_string()),
        output("{\"stdout\":1}".to_string()),
        output("{\"stdout\":\"a\",\"stderr\":\"b\",\"extra\":\"c\"}".to_string()),
        output(format!(
            "{{\"stdout\":\"{}\",\"stderr\":\"\"}}",
            "a".repeat(128 * 1024)
        )),
    ];
    for r in cases {
        refused(&svc, &store, &job, &[r]);
    }
    let mut ok = good.clone();
    ok.output = Some("{\"stdout\":\"fine\",\"stderr\":\"\"}".to_string());
    svc.ingest_trials(&job.job_id, &local(), &[ok]).unwrap();
}

#[test]
fn an_oversized_batch_is_refused() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let mut rec = Recorder::new();
    let batch: Vec<_> = (0..257)
        .map(|i| rec.pass(i, &props("a.id", AssertKind::Always, 1, 0)))
        .collect();
    refused(&svc, &store, &job, &batch);
}

#[test]
fn only_the_lease_holder_may_send_trials_and_only_while_the_run_is_open() {
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let stranger = svc.register_worker("stranger", "{}").unwrap();
    let err = svc
        .ingest_trials(&job.job_id, &stranger.id, &[one(1)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 0);
    svc.cancel_run(&job.run_id).unwrap();
    let err = svc
        .ingest_trials(&job.job_id, &local(), &[one(1)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 0);
}

fn rejected_with(svc: &Service, job: &LeasedJob, worker: &str, secret: Option<&str>) {
    let err = svc
        .ingest_trials_with_secret(&job.job_id, worker, secret, &[one(1)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err}");
}

#[test]
fn a_trusted_workers_id_is_useless_without_its_secret() {
    let (svc, store, engine) = setup();
    let (owner, secret) = svc.register_remote_worker("owner", "{}").unwrap();
    svc.set_worker_trust(&owner.id, true).unwrap();
    let job = lease(&svc, &engine, &owner.id);
    // Another caller names the trusted worker's id.
    rejected_with(&svc, &job, &owner.id, None);
    rejected_with(&svc, &job, &owner.id, Some("not-the-secret"));
    rejected_with(&svc, &job, &owner.id, Some(""));
    // A second registered worker's own secret does not work under the owner's id either.
    let (_, other_secret) = svc.register_remote_worker("other", "{}").unwrap();
    rejected_with(&svc, &job, &owner.id, Some(&other_secret));
    assert_eq!(store.count_trials(&job.run_id).unwrap(), 0);
    // The owner, with its secret, is accepted and its rows count as trusted.
    svc.ingest_trials_with_secret(&job.job_id, &owner.id, Some(&secret), &[one(1)])
        .unwrap();
    assert!(svc.run_trust(&job.run_id).unwrap().unwrap().all_trusted);
}

#[test]
fn a_worker_registered_without_a_secret_cannot_send_trials_remotely() {
    let (svc, _, engine) = setup();
    let plain = svc.register_worker("plain", "{}").unwrap();
    let job = lease(&svc, &engine, &plain.id);
    rejected_with(&svc, &job, &plain.id, None);
    rejected_with(&svc, &job, &plain.id, Some("anything"));
}

#[test]
fn another_workers_secret_replayed_under_a_new_worker_id_is_refused() {
    let (svc, _, engine) = setup();
    let (_, victim_secret) = svc.register_remote_worker("victim", "{}").unwrap();
    let (attacker, _) = svc.register_remote_worker("attacker", "{}").unwrap();
    let job = lease(&svc, &engine, &attacker.id);
    rejected_with(&svc, &job, &attacker.id, Some(&victim_secret));
}

#[test]
fn the_local_prefix_cannot_be_claimed_by_a_remote_secret_holder() {
    let (svc, _, engine) = setup();
    let (_, secret) = svc.register_remote_worker("remote", "{}").unwrap();
    let job = lease(&svc, &engine, &local());
    // A worker id that merely starts with the local prefix is not registered and has no secret.
    let err = svc
        .ingest_trials_with_secret(&job.job_id, "local-not-ours", Some(&secret), &[one(1)])
        .unwrap_err();
    assert!(matches!(err, ServiceError::InvalidRequest(_)), "{err}");
}

#[test]
fn failure_text_in_a_declaration_is_held_to_the_same_limits_as_event_text() {
    use hairspring_props::{Decl, Loc};
    let (svc, store, engine) = setup();
    let job = lease(&svc, &engine, &local());
    let good = one(1);
    for (msg, file) in [
        ("bidi \u{202e} text", "f.c"),
        ("line\nbreak", "f.c"),
        ("ansi \u{1b}[31m", "f.c"),
        ("ok", "evil\u{202e}.c"),
    ] {
        let mut r = good.clone();
        let mut d = TrialDigest::decode(&r.digest).unwrap();
        d.entries[0].decl = Some(Decl {
            msg: msg.to_string(),
            loc: Some(Loc {
                file: file.to_string(),
                line: 1,
                func: None,
            }),
        });
        // The codec refuses what it is asked to read; build the bytes by hand-encoding then corrupting nothing:
        // decoding must fail, so the service must too.
        r.digest = d.encode();
        refused(&svc, &store, &job, &[r]);
    }
}

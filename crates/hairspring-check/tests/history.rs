mod common;

use common::Hb;
use hairspring_check::*;

#[test]
fn pairs_invokes_with_completions_and_orders_by_invoke() {
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(5));
    b.invoke("c2", 1, "read", "k", None);
    b.ok("c2", 1, "read", "k", Some(5));
    b.ok("c1", 1, "write", "k", None);
    let h = b.build();
    let ops = h.operations();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].client, "c1");
    assert_eq!(ops[0].arg, Some(5));
    assert_eq!(ops[0].outcome, Outcome::Ok(None));
    assert_eq!(ops[1].outcome, Outcome::Ok(Some(5)));
    assert!(ops[0].invoke_seq < ops[1].invoke_seq);
}

#[test]
fn input_order_does_not_matter() {
    let mut b = Hb::new();
    b.seq_ok("c1", 1, "write", "k", Some(1), None);
    b.seq_ok("c2", 1, "read", "k", None, Some(1));
    let mut reversed: Vec<OpRecord> = b.records().to_vec();
    reversed.reverse();
    let a = History::from_records(b.records(), Completeness::Complete).unwrap();
    let r = History::from_records(&reversed, Completeness::Complete).unwrap();
    assert_eq!(a, r);
}

#[test]
fn invoke_without_completion_is_pending_and_unknown_outcome() {
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    let h = b.build();
    assert!(h.operations()[0].is_pending());
    assert_eq!(h.operations()[0].outcome, Outcome::Info);
}

#[test]
fn fail_and_info_completions_are_kept() {
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    b.fail("c1", 1, "write", "k");
    b.invoke("c1", 2, "write", "k", Some(2));
    b.info("c1", 2, "write", "k");
    let h = b.build();
    assert_eq!(h.operations()[0].outcome, Outcome::Fail);
    assert_eq!(h.operations()[1].outcome, Outcome::Info);
    assert!(!h.operations()[1].is_pending());
}

#[test]
fn rejects_duplicate_sequence_numbers() {
    let mut b = Hb::new();
    b.seq_ok("c1", 1, "write", "k", Some(1), None);
    let mut recs = b.records().to_vec();
    recs[1].seq = recs[0].seq;
    assert_eq!(
        History::from_records(&recs, Completeness::Complete),
        Err(HistoryError::DuplicateSeq { seq: recs[0].seq })
    );
}

#[test]
fn rejects_double_invoke_orphan_and_double_completion() {
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    b.invoke("c1", 1, "write", "k", Some(2));
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::DuplicateInvoke { .. })
    ));

    let mut b = Hb::new();
    b.ok("c1", 7, "write", "k", None);
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::CompletionWithoutInvoke { id: 7, .. })
    ));

    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    b.ok("c1", 1, "write", "k", None);
    b.ok("c1", 1, "write", "k", None);
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::DuplicateCompletion { .. })
    ));
}

#[test]
fn rejects_completion_that_names_another_function_or_key() {
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    b.ok("c1", 1, "read", "k", None);
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::MismatchedCompletion { .. })
    ));
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    b.ok("c1", 1, "write", "other", None);
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::MismatchedCompletion { .. })
    ));
}

#[test]
fn completion_before_its_invoke_in_sequence_order_is_rejected() {
    let mut b = Hb::new();
    b.invoke("c1", 1, "write", "k", Some(1));
    b.ok("c1", 1, "write", "k", None);
    let mut recs = b.records().to_vec();
    let (a, c) = (recs[0].seq, recs[1].seq);
    recs[0].seq = c;
    recs[1].seq = a;
    assert!(matches!(
        History::from_records(&recs, Completeness::Complete),
        Err(HistoryError::CompletionBeforeInvoke { .. })
    ));
}

#[test]
fn keys_are_sorted_and_per_key_views_filter() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "write", "b", Some(1), None);
    b.seq_ok("c", 2, "write", "a", Some(1), None);
    b.seq_ok("c", 3, "write", "b", Some(2), None);
    let h = b.build();
    assert_eq!(h.keys(), vec!["a", "b"]);
    assert_eq!(h.operations_for_key("b").count(), 2);
}

#[test]
fn every_checker_answers_unknown_on_an_incomplete_history() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "add", "s", Some(1), None);
    b.seq_ok("c", 2, "enqueue", "q", Some(1), None);
    b.seq_ok("c", 3, "append", "w", Some(1), None);
    b.seq_ok("c", 4, "write", "r", Some(1), None);
    let h = b.build().marked_incomplete();
    let limits = Limits::default();
    let unknown = Verdict::Unknown(UnknownReason::IncompleteHistory);
    // A violation here would be a lost acknowledged write, but dropped lines could explain it: still Unknown.
    assert_eq!(check_set(&h, "s", &[], &limits), unknown);
    assert_eq!(check_queue(&h, "q", &[], &limits), unknown);
    assert_eq!(check_wal_prefix(&h, "w", &[], &limits), unknown);
    assert_eq!(check_register_linearizable(&h, None, &limits), unknown);
    assert!(!unknown.is_pass());
    assert!(!unknown.is_conclusive());
}

#[test]
fn too_many_records_is_an_error_not_a_panic() {
    let rec = OpRecord {
        client: "c".into(),
        id: 0,
        kind: OpType::Invoke,
        f: "write".into(),
        key: "k".into(),
        value: None,
        expect: None,
        seq: 0,
    };
    let recs = vec![rec; hairspring_check::history::MAX_RECORDS + 1];
    assert!(matches!(
        History::from_records(&recs, Completeness::Complete),
        Err(HistoryError::TooManyRecords { .. })
    ));
}

#[test]
fn checker_ids_are_the_configuration_names() {
    assert_eq!(CheckerId::SetNoLostAcks.as_str(), "set-no-lost-acks");
    assert_eq!(CheckerId::QueueNoLossNoDup.as_str(), "queue-no-loss-no-dup");
    assert_eq!(CheckerId::WalPrefix.as_str(), "wal-prefix");
    assert_eq!(
        CheckerId::RegisterLinearizable.as_str(),
        "register-linearizable"
    );
}

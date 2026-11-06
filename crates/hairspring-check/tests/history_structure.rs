mod common;

use common::Hb;
use hairspring_check::*;

#[test]
fn a_client_cannot_invoke_while_it_has_another_operation_open() {
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(1));
    b.invoke("a", 2, "write", "k", Some(2));
    b.ok("a", 2, "write", "k", None);
    b.ok("a", 1, "write", "k", None);
    assert_eq!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::ClientOverlap {
            client: "a".into(),
            id: 2
        })
    );
}

#[test]
fn a_pending_operation_blocks_the_same_client_but_not_others() {
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(1));
    b.invoke("b", 1, "write", "k", Some(2));
    assert!(History::from_records(b.records(), Completeness::Complete).is_ok());
    b.invoke("a", 2, "write", "k", Some(3));
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::ClientOverlap { .. })
    ));
}

#[test]
fn an_info_completion_frees_the_client() {
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(1));
    b.info("a", 1, "write", "k");
    b.invoke("a", 2, "write", "k", Some(2));
    assert!(History::from_records(b.records(), Completeness::Complete).is_ok());
}

#[test]
fn a_completion_with_no_invoke_anywhere_is_not_reported_as_a_sequence_regression() {
    let mut b = Hb::new();
    b.ok("a", 9, "write", "k", None);
    assert!(matches!(
        History::from_records(b.records(), Completeness::Complete),
        Err(HistoryError::CompletionWithoutInvoke { id: 9, .. })
    ));
}

#[test]
fn operations_by_key_groups_in_one_pass_in_invoke_order() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "write", "b", Some(1), None);
    b.seq_ok("c", 2, "write", "a", Some(1), None);
    b.seq_ok("c", 3, "write", "b", Some(2), None);
    let h = b.build();
    let groups = h.operations_by_key();
    assert_eq!(groups.keys().copied().collect::<Vec<_>>(), vec!["a", "b"]);
    assert_eq!(
        groups["b"].iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![1, 3]
    );
}

#[test]
fn an_empty_history_is_valid_and_every_checker_passes_it_when_complete() {
    let h = Hb::new().build();
    let l = Limits::default();
    assert_eq!(check_set(&h, "k", &[], &l), Verdict::Ok);
    assert_eq!(check_queue(&h, "k", &[], &l), Verdict::Ok);
    assert_eq!(check_wal_prefix(&h, "k", &[], &l), Verdict::Ok);
    assert_eq!(check_register_linearizable(&h, None, &l), Verdict::Ok);
}

#[test]
fn an_empty_incomplete_history_is_unknown_for_every_checker() {
    let h = Hb::new().build().marked_incomplete();
    let l = Limits::default();
    let unknown = Verdict::Unknown(UnknownReason::IncompleteHistory);
    assert_eq!(check_set(&h, "k", &[], &l), unknown);
    assert_eq!(check_queue(&h, "k", &[], &l), unknown);
    assert_eq!(check_wal_prefix(&h, "k", &[], &l), unknown);
    assert_eq!(check_register_linearizable(&h, None, &l), unknown);
}

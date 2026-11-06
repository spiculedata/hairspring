mod common;

use common::Hb;
use hairspring_check::*;

fn appends(outcomes: &[(i64, &str)]) -> History {
    let mut b = Hb::new();
    for (i, (v, how)) in outcomes.iter().enumerate() {
        let id = i as u64;
        b.invoke("w", id, "append", "log", Some(*v));
        match *how {
            "ok" => b.ok("w", id, "append", "log", None),
            "fail" => b.fail("w", id, "append", "log"),
            "info" => b.info("w", id, "append", "log"),
            _ => &mut b,
        };
    }
    b.build()
}

fn run(h: &History, recovered: &[i64]) -> Verdict {
    check_wal_prefix(h, "log", recovered, &Limits::default())
}

#[test]
fn full_recovery_is_ok() {
    let h = appends(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    assert_eq!(run(&h, &[1, 2, 3]), Verdict::Ok);
}

#[test]
fn recovery_missing_an_acknowledged_tail_is_a_violation() {
    let h = appends(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    let v = run(&h, &[1, 2]);
    let v = v.violation().unwrap();
    assert_eq!(v.checker, CheckerId::WalPrefix);
    assert_eq!(
        v.kind,
        ViolationKind::TruncatedBelowAcknowledged {
            recovered: 2,
            required: 3
        }
    );
    assert_eq!(v.witness.len(), 1);
    assert_eq!(v.witness[0].arg, Some(3));
}

#[test]
fn recovery_may_drop_unacknowledged_tail() {
    let h = appends(&[(1, "ok"), (2, "info")]);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
    assert_eq!(run(&h, &[1, 2]), Verdict::Ok);
}

#[test]
fn pending_append_is_unacknowledged() {
    let h = appends(&[(1, "ok"), (2, "pending")]);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
    assert_eq!(run(&h, &[1, 2]), Verdict::Ok);
}

#[test]
fn a_gap_is_not_a_prefix() {
    let h = appends(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    let v = run(&h, &[1, 3]);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::NotPrefix { index: 1 }
    );
}

#[test]
fn an_unknown_append_followed_by_later_appends_is_never_a_violation() {
    // A timed-out append may never land, or may land late, so any recovered log could be explained.
    let h = appends(&[(1, "info"), (2, "ok")]);
    for recovered in [&[2][..], &[2, 1], &[1, 2], &[], &[1]] {
        let v = run(&h, recovered);
        assert!(
            matches!(v, Verdict::Unknown(UnknownReason::UnsupportedShape { .. })),
            "{recovered:?} gave {v:?}"
        );
    }
    let h = appends(&[(1, "ok"), (2, "info"), (3, "ok")]);
    assert!(matches!(
        run(&h, &[1, 3]),
        Verdict::Unknown(UnknownReason::UnsupportedShape { .. })
    ));
}

#[test]
fn a_pending_append_by_one_client_followed_by_another_clients_ack_is_unknown() {
    let mut b = Hb::new();
    b.invoke("a", 1, "append", "log", Some(1));
    b.seq_ok("b", 1, "append", "log", Some(2), None);
    for recovered in [&[2][..], &[1, 2], &[2, 1]] {
        assert!(
            matches!(
                run(&b.build(), recovered),
                Verdict::Unknown(UnknownReason::UnsupportedShape { .. })
            ),
            "{recovered:?}"
        );
    }
}

#[test]
fn an_unknown_append_followed_only_by_a_failed_append_is_still_decided() {
    let h = appends(&[(1, "ok"), (2, "info"), (3, "fail")]);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
    assert_eq!(run(&h, &[1, 2]), Verdict::Ok);
    assert!(run(&h, &[1, 3]).violation().is_some());
}

#[test]
fn phantom_names_a_witness_operation() {
    let h = appends(&[(1, "ok"), (4, "fail")]);
    let v = run(&h, &[1, 4]);
    let v = v.violation().unwrap();
    assert!(!v.witness.is_empty());
    assert!(v.witness.iter().any(|o| o.arg == Some(4)));

    let h = appends(&[(1, "ok")]);
    let v = run(&h, &[1, 9]);
    assert!(!v.violation().unwrap().witness.is_empty());
}

#[test]
fn reordered_or_torn_record_is_not_a_prefix() {
    let h = appends(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    assert_eq!(
        run(&h, &[2, 1, 3]).violation().unwrap().kind,
        ViolationKind::NotPrefix { index: 0 }
    );
    assert_eq!(
        run(&h, &[1, 2, 9]).violation().unwrap().kind,
        ViolationKind::NotPrefix { index: 2 }
    );
}

#[test]
fn a_failed_append_that_was_recovered_is_not_a_prefix() {
    let h = appends(&[(1, "ok"), (2, "fail"), (3, "ok")]);
    assert_eq!(run(&h, &[1, 3]), Verdict::Ok);
    assert_eq!(
        run(&h, &[1, 2, 3]).violation().unwrap().kind,
        ViolationKind::NotPrefix { index: 1 }
    );
}

#[test]
fn extra_records_after_the_attempted_log_are_phantoms() {
    let h = appends(&[(1, "ok")]);
    assert_eq!(
        run(&h, &[1, 4]).violation().unwrap().kind,
        ViolationKind::Phantom { value: 4 }
    );
}

#[test]
fn empty_history_with_empty_recovery_is_ok() {
    assert_eq!(run(&Hb::new().build(), &[]), Verdict::Ok);
}

#[test]
fn overlapping_appends_on_one_log_are_not_modelled() {
    let mut b = Hb::new();
    b.invoke("w1", 1, "append", "log", Some(1));
    b.invoke("w2", 1, "append", "log", Some(2));
    b.ok("w1", 1, "append", "log", None);
    b.ok("w2", 1, "append", "log", None);
    assert!(matches!(
        run(&b.build(), &[1, 2]),
        Verdict::Unknown(UnknownReason::UnsupportedShape { .. })
    ));
}

#[test]
fn append_after_an_unknown_append_by_the_same_client_is_unknown() {
    let mut b = Hb::new();
    b.invoke("w", 1, "append", "log", Some(1));
    b.info("w", 1, "append", "log");
    b.seq_ok("w", 2, "append", "log", Some(2), None);
    assert!(matches!(
        run(&b.build(), &[1, 2]),
        Verdict::Unknown(UnknownReason::UnsupportedShape { .. })
    ));
}

#[test]
fn unsupported_function_is_unknown() {
    let mut b = Hb::new();
    b.seq_ok("w", 1, "write", "log", Some(1), None);
    assert!(matches!(
        run(&b.build(), &[]),
        Verdict::Unknown(UnknownReason::UnsupportedOperation { .. })
    ));
}

mod common;

use common::Hb;
use hairspring_check::*;

fn run(h: &History) -> Verdict {
    check_register_linearizable(h, None, &Limits::default())
}

fn write(b: &mut Hb, c: &str, id: u64, key: &str, v: i64) {
    b.seq_ok(c, id, "write", key, Some(v), None);
}

fn read(b: &mut Hb, c: &str, id: u64, key: &str, v: Option<i64>) {
    b.seq_ok(c, id, "read", key, None, v);
}

#[test]
fn empty_history_is_ok() {
    assert_eq!(run(&Hb::new().build()), Verdict::Ok);
}

#[test]
fn sequential_writes_and_reads_are_ok() {
    let mut b = Hb::new();
    read(&mut b, "a", 1, "k", None);
    write(&mut b, "a", 2, "k", 1);
    read(&mut b, "a", 3, "k", Some(1));
    write(&mut b, "a", 4, "k", 2);
    read(&mut b, "a", 5, "k", Some(2));
    assert_eq!(run(&b.build()), Verdict::Ok);
}

#[test]
fn stale_read_is_a_violation() {
    let mut b = Hb::new();
    write(&mut b, "a", 1, "k", 1);
    write(&mut b, "a", 2, "k", 2);
    read(&mut b, "b", 1, "k", Some(1));
    let v = run(&b.build());
    let v = v.violation().expect("stale read");
    assert_eq!(v.checker, CheckerId::RegisterLinearizable);
    assert_eq!(v.key, "k");
    assert_eq!(v.kind, ViolationKind::NotLinearizable);
}

#[test]
fn read_of_a_value_that_was_never_written_is_a_violation() {
    let mut b = Hb::new();
    write(&mut b, "a", 1, "k", 1);
    read(&mut b, "b", 1, "k", Some(7));
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn read_from_the_future_is_a_violation() {
    let mut b = Hb::new();
    read(&mut b, "b", 1, "k", Some(2));
    write(&mut b, "a", 1, "k", 2);
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn overlapping_write_and_read_may_see_either_value() {
    for seen in [None, Some(1)] {
        let mut b = Hb::new();
        b.invoke("a", 1, "write", "k", Some(1));
        b.invoke("b", 1, "read", "k", None);
        b.ok("b", 1, "read", "k", seen);
        b.ok("a", 1, "write", "k", None);
        assert_eq!(run(&b.build()), Verdict::Ok, "seen {seen:?}");
    }
}

#[test]
fn two_reads_that_disagree_with_any_single_order_are_a_violation() {
    // w1 and w2 overlap both reads; r1 sees 2 then r2 (strictly later) sees 1, and 1 was written before 2 was read...
    // Writes: w(1) done first, then w(2) done; later r->1 cannot happen.
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(1));
    b.ok("a", 1, "write", "k", None);
    b.invoke("a", 2, "write", "k", Some(2));
    b.ok("a", 2, "write", "k", None);
    read(&mut b, "b", 1, "k", Some(2));
    read(&mut b, "b", 2, "k", Some(1));
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn lost_update_with_cas_is_a_violation() {
    // Both clients compare-and-set 0 -> 1 and 0 -> 2 and both are told it succeeded.
    let mut b = Hb::new();
    write(&mut b, "a", 1, "k", 0);
    b.invoke_cas("a", 2, "k", 0, 1);
    b.invoke_cas("b", 1, "k", 0, 2);
    b.ok("a", 2, "cas", "k", None);
    b.ok("b", 1, "cas", "k", None);
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn cas_that_fails_and_cas_that_succeeds_in_sequence_are_ok() {
    let mut b = Hb::new();
    write(&mut b, "a", 1, "k", 0);
    b.invoke_cas("a", 2, "k", 0, 1);
    b.ok("a", 2, "cas", "k", None);
    b.invoke_cas("b", 1, "k", 0, 2);
    b.fail("b", 1, "cas", "k");
    read(&mut b, "a", 3, "k", Some(1));
    assert_eq!(run(&b.build()), Verdict::Ok);
}

#[test]
fn a_successful_cas_whose_expectation_cannot_hold_is_a_violation() {
    let mut b = Hb::new();
    write(&mut b, "a", 1, "k", 5);
    b.invoke_cas("a", 2, "k", 0, 1);
    b.ok("a", 2, "cas", "k", None);
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn unknown_write_may_or_may_not_have_happened() {
    for seen in [None, Some(9)] {
        let mut b = Hb::new();
        b.invoke("a", 1, "write", "k", Some(9));
        b.info("a", 1, "write", "k");
        read(&mut b, "b", 1, "k", seen);
        assert_eq!(run(&b.build()), Verdict::Ok, "seen {seen:?}");
    }
}

#[test]
fn pending_write_may_be_seen_late() {
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(9));
    read(&mut b, "b", 1, "k", None);
    read(&mut b, "b", 2, "k", Some(9));
    assert_eq!(run(&b.build()), Verdict::Ok);
}

#[test]
fn unknown_write_cannot_be_seen_then_unseen_without_another_writer() {
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(9));
    b.info("a", 1, "write", "k");
    read(&mut b, "b", 1, "k", Some(9));
    read(&mut b, "b", 2, "k", None);
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn failed_write_must_not_be_visible() {
    let mut b = Hb::new();
    b.invoke("a", 1, "write", "k", Some(5));
    b.fail("a", 1, "write", "k");
    read(&mut b, "b", 1, "k", Some(5));
    assert!(run(&b.build()).violation().is_some());
}

#[test]
fn initial_value_is_a_parameter() {
    let mut b = Hb::new();
    read(&mut b, "a", 1, "k", Some(0));
    let h = b.build();
    assert!(check_register_linearizable(&h, None, &Limits::default())
        .violation()
        .is_some());
    assert_eq!(
        check_register_linearizable(&h, Some(0), &Limits::default()),
        Verdict::Ok
    );
}

#[test]
fn keys_are_checked_independently_and_the_violating_key_is_named() {
    let mut b = Hb::new();
    write(&mut b, "a", 1, "good", 1);
    read(&mut b, "a", 2, "good", Some(1));
    write(&mut b, "a", 3, "bad", 1);
    write(&mut b, "a", 4, "bad", 2);
    read(&mut b, "a", 5, "bad", Some(1));
    let h = b.build();
    let v = run(&h);
    assert_eq!(v.violation().unwrap().key, "bad");
    assert_eq!(
        check_register_key(&h, "good", None, &Limits::default()),
        Verdict::Ok
    );
    assert!(check_register_key(&h, "bad", None, &Limits::default())
        .violation()
        .is_some());
}

#[test]
fn witness_is_a_one_minimal_violating_sub_history() {
    let mut b = Hb::new();
    // Noise on the same key that has nothing to do with the stale read.
    write(&mut b, "n", 1, "k", 10);
    read(&mut b, "n", 2, "k", Some(10));
    write(&mut b, "a", 1, "k", 1);
    read(&mut b, "n", 3, "k", Some(1));
    write(&mut b, "a", 2, "k", 2);
    read(&mut b, "n", 4, "k", Some(2));
    read(&mut b, "b", 1, "k", Some(1)); // the stale read
    write(&mut b, "n", 5, "k", 11);
    let v = run(&b.build());
    let v = v.violation().unwrap();
    assert!(v.witness_minimal);
    assert!(
        v.witness.len() <= 4,
        "witness too long: {}",
        v.witness.len()
    );

    // The witness alone is a violating history, and removing any one operation makes it linearizable.
    let as_history = |ops: &[Operation]| {
        let mut hb = Hb::new();
        let mut events: Vec<(u64, usize, bool)> = Vec::new();
        for (i, o) in ops.iter().enumerate() {
            events.push((o.invoke_seq, i, true));
            if let Some(c) = o.complete_seq {
                events.push((c, i, false));
            }
        }
        events.sort();
        for (_, i, is_invoke) in events {
            let o = &ops[i];
            let c = format!("w{i}");
            match (is_invoke, o.outcome) {
                (true, _) => {
                    hb.invoke(&c, 0, &o.f, &o.key, o.arg);
                }
                (false, Outcome::Ok(r)) => {
                    hb.ok(&c, 0, &o.f, &o.key, r);
                }
                (false, Outcome::Fail) => {
                    hb.fail(&c, 0, &o.f, &o.key);
                }
                (false, Outcome::Info) => {
                    hb.info(&c, 0, &o.f, &o.key);
                }
            }
        }
        hb.build()
    };
    assert!(run(&as_history(&v.witness)).violation().is_some());
    for skip in 0..v.witness.len() {
        let rest: Vec<Operation> = v
            .witness
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, o)| o.clone())
            .collect();
        assert_eq!(
            run(&as_history(&rest)),
            Verdict::Ok,
            "witness not minimal without op {skip}"
        );
    }
}

#[test]
fn results_and_witnesses_are_deterministic() {
    let mut b = Hb::new();
    for i in 0..6u64 {
        b.invoke(&format!("c{i}"), 0, "write", "k", Some(i as i64));
    }
    for i in 0..6u64 {
        b.ok(&format!("c{i}"), 0, "write", "k", None);
    }
    read(&mut b, "r", 1, "k", Some(1));
    read(&mut b, "r", 2, "k", Some(2));
    let h = b.build();
    assert_eq!(run(&h), run(&h));
    assert!(run(&h).violation().is_some());
}

/// Many overlapping writes followed by two sequential reads that no order can satisfy: the search has to try a lot.
fn hard_history(writers: u64) -> History {
    let mut b = Hb::new();
    for i in 0..writers {
        b.invoke(&format!("c{i}"), 0, "write", "k", Some(i as i64));
    }
    for i in 0..writers {
        b.ok(&format!("c{i}"), 0, "write", "k", None);
    }
    read(&mut b, "r", 1, "k", Some(1));
    read(&mut b, "r", 2, "k", Some(2));
    b.build()
}

#[test]
fn step_budget_exhaustion_is_unknown_never_ok() {
    let h = hard_history(10);
    let tight = Limits {
        max_steps: 50,
        ..Limits::default()
    };
    let v = check_register_linearizable(&h, None, &tight);
    assert!(
        matches!(
            v,
            Verdict::Unknown(UnknownReason::StepBudgetExhausted { max_steps: 50 })
        ),
        "{v:?}"
    );
    assert!(!v.is_pass());
    // With the default budget the same history is decided.
    assert!(run(&h).violation().is_some());
}

#[test]
fn memo_budget_exhaustion_is_unknown_never_ok() {
    let h = hard_history(10);
    let tight = Limits {
        max_memo_entries: 1,
        ..Limits::default()
    };
    let v = check_register_linearizable(&h, None, &tight);
    assert!(
        matches!(
            v,
            Verdict::Unknown(UnknownReason::MemoBudgetExhausted { max_entries: 1 })
        ),
        "{v:?}"
    );
}

#[test]
fn exhausted_budget_on_a_linearizable_history_is_still_not_ok() {
    // Satisfiable but needs search: the right order of many overlapping writes is only found late.
    let mut b = Hb::new();
    for i in 0..10u64 {
        b.invoke(&format!("c{i}"), 0, "write", "k", Some(i as i64));
    }
    for i in 0..10u64 {
        b.ok(&format!("c{i}"), 0, "write", "k", None);
    }
    read(&mut b, "r", 1, "k", Some(0));
    let h = b.build();
    let tight = Limits {
        max_steps: 1,
        ..Limits::default()
    };
    assert!(!check_register_linearizable(&h, None, &tight).is_pass());
    assert_eq!(run(&h), Verdict::Ok);
}

#[test]
fn too_many_operations_on_a_key_is_unknown() {
    let h = hard_history(10);
    let limits = Limits {
        max_ops_per_key: 5,
        ..Limits::default()
    };
    assert!(matches!(
        check_register_linearizable(&h, None, &limits),
        Verdict::Unknown(UnknownReason::TooManyOperations {
            operations: 12,
            max: 5,
            ..
        })
    ));
}

#[test]
fn witness_budget_exhaustion_still_returns_a_violation_flagged_not_minimal() {
    let h = hard_history(8);
    let limits = Limits {
        max_witness_steps: 1,
        ..Limits::default()
    };
    let v = check_register_linearizable(&h, None, &limits);
    let v = v.violation().expect("still a violation");
    assert!(!v.witness_minimal);
    assert!(!v.witness.is_empty());
}

#[test]
fn unsupported_function_is_unknown() {
    let mut b = Hb::new();
    b.seq_ok("a", 1, "increment", "k", Some(1), None);
    assert!(matches!(
        run(&b.build()),
        Verdict::Unknown(UnknownReason::UnsupportedOperation { .. })
    ));
}

#[test]
fn a_violation_on_one_key_outranks_unknown_on_another() {
    let mut b = Hb::new();
    write(&mut b, "a", 1, "a_unsupported_first", 1);
    b.seq_ok("a", 2, "increment", "a_unsupported_first", Some(1), None);
    read(&mut b, "a", 3, "z_bad", Some(5));
    let v = run(&b.build());
    assert_eq!(v.violation().unwrap().key, "z_bad");
}

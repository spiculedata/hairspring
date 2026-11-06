mod common;

use common::{Hb, Rng};
use hairspring_check::*;

fn adds(outcomes: &[(i64, &str)]) -> History {
    let mut b = Hb::new();
    for (i, (v, how)) in outcomes.iter().enumerate() {
        let id = i as u64;
        b.invoke("c", id, "add", "s", Some(*v));
        match *how {
            "ok" => b.ok("c", id, "add", "s", None),
            "fail" => b.fail("c", id, "add", "s"),
            "info" => b.info("c", id, "add", "s"),
            _ => &mut b,
        };
    }
    b.build()
}

fn run(h: &History, fin: &[i64]) -> Verdict {
    check_set(h, "s", fin, &Limits::default())
}

#[test]
fn all_acknowledged_adds_present_is_ok() {
    let h = adds(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    assert_eq!(run(&h, &[3, 1, 2]), Verdict::Ok);
}

#[test]
fn empty_history_and_empty_final_state_is_ok() {
    assert_eq!(run(&Hb::new().build(), &[]), Verdict::Ok);
}

#[test]
fn lost_acknowledged_add_is_a_violation_with_the_add_as_witness() {
    let h = adds(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    let v = run(&h, &[1, 3]);
    let v = v.violation().expect("violation");
    assert_eq!(v.checker, CheckerId::SetNoLostAcks);
    assert_eq!(v.kind, ViolationKind::LostAcknowledgedWrite { value: 2 });
    assert_eq!(v.witness.len(), 1);
    assert_eq!(v.witness[0].arg, Some(2));
}

#[test]
fn unknown_adds_may_or_may_not_be_present() {
    let h = adds(&[(1, "info"), (2, "info")]);
    assert_eq!(run(&h, &[]), Verdict::Ok);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
    assert_eq!(run(&h, &[1, 2]), Verdict::Ok);
}

#[test]
fn pending_add_counts_as_unknown() {
    let h = adds(&[(1, "pending")]);
    assert_eq!(run(&h, &[]), Verdict::Ok);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
}

#[test]
fn failed_add_that_shows_up_is_a_phantom() {
    let h = adds(&[(1, "fail")]);
    let v = run(&h, &[1]);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::Phantom { value: 1 }
    );
    assert_eq!(v.violation().unwrap().witness.len(), 1);
}

#[test]
fn value_never_added_is_a_phantom() {
    let h = adds(&[(1, "ok")]);
    let v = run(&h, &[1, 99]);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::Phantom { value: 99 }
    );
    assert!(v.violation().unwrap().witness.is_empty());
}

#[test]
fn duplicate_in_final_state_is_reported() {
    let h = adds(&[(1, "ok")]);
    let v = run(&h, &[1, 1]);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::Duplicate {
            value: 1,
            seen: 2,
            at_most: 1
        }
    );
}

#[test]
fn adding_the_same_value_twice_is_fine() {
    let h = adds(&[(1, "ok"), (1, "ok")]);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
}

#[test]
fn other_keys_do_not_matter() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "add", "s", Some(1), None);
    b.seq_ok("c", 2, "add", "t", Some(2), None);
    let h = b.build();
    assert_eq!(check_set(&h, "s", &[1], &Limits::default()), Verdict::Ok);
    assert!(check_set(&h, "t", &[], &Limits::default())
        .violation()
        .is_some());
}

#[test]
fn unknown_function_or_missing_argument_is_unknown_not_ok() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "frobnicate", "s", Some(1), None);
    let v = run(&b.build(), &[]);
    assert!(matches!(
        v,
        Verdict::Unknown(UnknownReason::UnsupportedOperation { .. })
    ));

    let mut b = Hb::new();
    b.seq_ok("c", 1, "add", "s", None, None);
    assert!(matches!(
        run(&b.build(), &[]),
        Verdict::Unknown(UnknownReason::UnsupportedOperation { .. })
    ));
}

#[test]
fn reads_of_the_set_are_ignored_but_allowed() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "add", "s", Some(1), None);
    b.seq_ok("c", 2, "read", "s", None, Some(1));
    assert_eq!(run(&b.build(), &[1]), Verdict::Ok);
}

#[test]
fn too_many_operations_gives_unknown() {
    let h = adds(&[(1, "ok"), (2, "ok"), (3, "ok")]);
    let limits = Limits {
        max_ops_per_key: 2,
        ..Limits::default()
    };
    assert!(matches!(
        check_set(&h, "s", &[1, 2, 3], &limits),
        Verdict::Unknown(UnknownReason::TooManyOperations {
            operations: 3,
            max: 2,
            ..
        })
    ));
}

#[test]
fn property_simulated_good_runs_pass_and_mutations_are_caught() {
    let mut rng = Rng(0x5E7);
    for _ in 0..300 {
        let n = 1 + rng.below(12);
        let mut outcomes = Vec::new();
        let mut final_state = Vec::new();
        let mut acked = Vec::new();
        for v in 0..n as i64 {
            let how = match rng.below(3) {
                0 => "ok",
                1 => "info",
                _ => "fail",
            };
            outcomes.push((v, how));
            match how {
                "ok" => {
                    final_state.push(v);
                    acked.push(v);
                }
                "info" if rng.chance(50) => final_state.push(v),
                _ => {}
            }
        }
        let h = adds(&outcomes);
        assert_eq!(
            run(&h, &final_state),
            Verdict::Ok,
            "good run {outcomes:?} {final_state:?}"
        );

        if let Some(&lost) = acked.first() {
            let mutated: Vec<i64> = final_state.iter().copied().filter(|v| *v != lost).collect();
            let v = run(&h, &mutated);
            assert_eq!(
                v.violation().unwrap().kind,
                ViolationKind::LostAcknowledgedWrite { value: lost }
            );
        }
        let mut with_phantom = final_state.clone();
        with_phantom.push(1000);
        assert_eq!(
            run(&h, &with_phantom).violation().unwrap().kind,
            ViolationKind::Phantom { value: 1000 }
        );
    }
}

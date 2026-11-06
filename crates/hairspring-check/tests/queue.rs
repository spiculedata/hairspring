mod common;

use common::{Hb, Rng};
use hairspring_check::*;

fn run(h: &History, remaining: &[i64]) -> Verdict {
    check_queue(h, "q", remaining, &Limits::default())
}

fn enq_ok(b: &mut Hb, id: u64, v: i64) {
    b.seq_ok("p", id, "enqueue", "q", Some(v), None);
}

fn deq_ok(b: &mut Hb, id: u64, v: Option<i64>) {
    b.seq_ok("c", id, "dequeue", "q", None, v);
}

#[test]
fn deliveries_plus_remaining_cover_every_acknowledged_enqueue() {
    let mut b = Hb::new();
    for v in 1..=3 {
        enq_ok(&mut b, v as u64, v);
    }
    deq_ok(&mut b, 10, Some(1));
    assert_eq!(run(&b.build(), &[2, 3]), Verdict::Ok);
}

#[test]
fn lost_message_is_a_violation() {
    let mut b = Hb::new();
    for v in 1..=3 {
        enq_ok(&mut b, v as u64, v);
    }
    deq_ok(&mut b, 10, Some(1));
    let v = run(&b.build(), &[3]);
    let v = v.violation().unwrap();
    assert_eq!(v.checker, CheckerId::QueueNoLossNoDup);
    assert_eq!(v.kind, ViolationKind::LostMessage { value: 2 });
    assert_eq!(v.witness.len(), 1);
    assert_eq!(v.witness[0].f, "enqueue");
}

#[test]
fn duplicate_delivery_is_a_violation() {
    let mut b = Hb::new();
    enq_ok(&mut b, 1, 7);
    deq_ok(&mut b, 10, Some(7));
    deq_ok(&mut b, 11, Some(7));
    let v = run(&b.build(), &[]);
    let v = v.violation().unwrap();
    assert_eq!(
        v.kind,
        ViolationKind::Duplicate {
            value: 7,
            seen: 2,
            at_most: 1
        }
    );
    assert_eq!(v.witness.len(), 3);
}

#[test]
fn delivered_and_still_queued_is_a_duplicate() {
    let mut b = Hb::new();
    enq_ok(&mut b, 1, 7);
    deq_ok(&mut b, 10, Some(7));
    let v = run(&b.build(), &[7]);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::Duplicate {
            value: 7,
            seen: 2,
            at_most: 1
        }
    );
}

#[test]
fn delivery_of_a_value_never_enqueued_is_a_phantom() {
    let mut b = Hb::new();
    enq_ok(&mut b, 1, 7);
    deq_ok(&mut b, 10, Some(9));
    let v = run(&b.build(), &[7]);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::Phantom { value: 9 }
    );
}

#[test]
fn failed_enqueue_that_is_delivered_is_a_phantom() {
    let mut b = Hb::new();
    b.invoke("p", 1, "enqueue", "q", Some(5));
    b.fail("p", 1, "enqueue", "q");
    deq_ok(&mut b, 10, Some(5));
    assert_eq!(
        run(&b.build(), &[]).violation().unwrap().kind,
        ViolationKind::Phantom { value: 5 }
    );
}

#[test]
fn unknown_enqueue_may_be_delivered_or_absent() {
    let mut b = Hb::new();
    b.invoke("p", 1, "enqueue", "q", Some(5));
    b.info("p", 1, "enqueue", "q");
    let h = b.build();
    assert_eq!(run(&h, &[]), Verdict::Ok);
    assert_eq!(run(&h, &[5]), Verdict::Ok);
}

#[test]
fn unknown_dequeue_can_excuse_one_missing_value_but_not_two() {
    let mut b = Hb::new();
    enq_ok(&mut b, 1, 1);
    enq_ok(&mut b, 2, 2);
    b.invoke("c", 10, "dequeue", "q", None);
    b.info("c", 10, "dequeue", "q");
    let h = b.build();
    assert_eq!(run(&h, &[2]), Verdict::Ok);
    assert_eq!(run(&h, &[1]), Verdict::Ok);
    assert_eq!(
        run(&h, &[]).violation().unwrap().kind,
        ViolationKind::LostMessage { value: 1 }
    );
}

#[test]
fn dequeue_of_empty_queue_is_ignored() {
    let mut b = Hb::new();
    deq_ok(&mut b, 10, None);
    assert_eq!(run(&b.build(), &[]), Verdict::Ok);
}

#[test]
fn same_value_enqueued_twice_is_counted_not_deduplicated() {
    let mut b = Hb::new();
    enq_ok(&mut b, 1, 4);
    enq_ok(&mut b, 2, 4);
    deq_ok(&mut b, 10, Some(4));
    assert_eq!(run(&b.build(), &[4]), Verdict::Ok);
    assert_eq!(
        run(&b.build(), &[]).violation().unwrap().kind,
        ViolationKind::LostMessage { value: 4 }
    );
}

#[test]
fn unsupported_function_is_unknown() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "push", "q", Some(1), None);
    assert!(matches!(
        run(&b.build(), &[]),
        Verdict::Unknown(UnknownReason::UnsupportedOperation { .. })
    ));
}

#[test]
fn property_simulated_runs_pass_and_mutations_are_caught() {
    let mut rng = Rng(0xC0FFEE);
    for _ in 0..300 {
        let n = 1 + rng.below(10);
        let mut b = Hb::new();
        let mut queue: Vec<i64> = Vec::new();
        for v in 0..n as i64 {
            enq_ok(&mut b, v as u64, v);
            queue.push(v);
        }
        let mut delivered = Vec::new();
        let mut id = 100;
        while !queue.is_empty() && rng.chance(60) {
            let v = queue.remove(0);
            deq_ok(&mut b, id, Some(v));
            delivered.push(v);
            id += 1;
        }
        let h = b.build();
        assert_eq!(run(&h, &queue), Verdict::Ok);

        if let Some(&lost) = queue.first() {
            let rest: Vec<i64> = queue[1..].to_vec();
            assert_eq!(
                run(&h, &rest).violation().unwrap().kind,
                ViolationKind::LostMessage { value: lost }
            );
        }
        let mut dup = queue.clone();
        let again = delivered
            .first()
            .copied()
            .or(queue.first().copied())
            .unwrap();
        dup.push(again);
        assert!(matches!(
            run(&h, &dup).violation().unwrap().kind,
            ViolationKind::Duplicate { value, .. } if value == again
        ));
    }
}

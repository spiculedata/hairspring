mod common;

use common::Hb;
use hairspring_check::*;
use std::time::{Duration, Instant};

#[test]
fn each_key_can_have_its_own_initial_value() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "read", "a", None, Some(1));
    b.seq_ok("c", 2, "read", "b", None, None);
    b.seq_ok("c", 3, "read", "z", None, Some(7));
    let h = b.build();
    let limits = Limits::default();

    // One initial for every key: "a" and "z" cannot both be satisfied.
    assert!(check_register_linearizable(&h, None, &limits)
        .violation()
        .is_some());

    let initials = RegisterInitials::all(None)
        .with_key("a", Some(1))
        .with_key("z", Some(7));
    assert_eq!(
        check_register_linearizable(&h, initials.clone(), &limits),
        Verdict::Ok
    );
    assert_eq!(initials.for_key("a"), Some(1));
    assert_eq!(initials.for_key("b"), None);

    // A per-key initial does not leak to other keys.
    let only_a = RegisterInitials::all(None).with_key("a", Some(1));
    assert_eq!(
        check_register_linearizable(&h, only_a, &limits)
            .violation()
            .unwrap()
            .key,
        "z"
    );
}

#[test]
fn a_default_initial_applies_to_keys_without_an_entry() {
    let mut b = Hb::new();
    b.seq_ok("c", 1, "read", "a", None, Some(0));
    b.seq_ok("c", 2, "read", "b", None, Some(5));
    let initials = RegisterInitials::all(Some(0)).with_key("b", Some(5));
    assert_eq!(
        check_register_linearizable(&b.build(), initials, &Limits::default()),
        Verdict::Ok
    );
}

#[test]
fn many_keys_do_not_make_the_aggregate_check_quadratic() {
    // 100,000 keys with two operations each. Grouping once is linear; filtering all operations per key would take
    // roughly 2 * 10^10 steps. The bound is loose on purpose: it only separates the two.
    let keys = 100_000u64;
    let mut b = Hb::new();
    for k in 0..keys {
        let key = format!("k{k}");
        b.seq_ok("c", 2 * k, "write", &key, Some(k as i64), None);
        b.seq_ok("c", 2 * k + 1, "read", &key, None, Some(k as i64));
    }
    let h = b.build();
    let started = Instant::now();
    let verdict = check_register_linearizable(&h, None, &Limits::default());
    let took = started.elapsed();
    assert_eq!(verdict, Verdict::Ok);
    assert!(took < Duration::from_secs(60), "took {took:?}");
}

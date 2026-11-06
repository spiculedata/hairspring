//! Adversarial input: random records (valid or not) must never panic, and the checkers must stay inside their limits,
//! stay deterministic, and never turn an `Unknown` into a pass.

mod common;

use common::Rng;
use hairspring_check::*;
use std::time::{Duration, Instant};

const FUNCTIONS: [&str; 10] = [
    "read",
    "write",
    "cas",
    "add",
    "enqueue",
    "dequeue",
    "append",
    "",
    "x",
    "\u{0}\u{ffff}",
];
const KEYS: [&str; 4] = ["a", "b", "", "k\u{1f600}"];
const VALUES: [Option<i64>; 7] = [
    None,
    Some(0),
    Some(1),
    Some(2),
    Some(-1),
    Some(i64::MAX),
    Some(i64::MIN),
];

fn random_record(rng: &mut Rng) -> OpRecord {
    OpRecord {
        client: format!("c{}", rng.below(4)),
        id: rng.below(6) as u64,
        kind: match rng.below(4) {
            0 => OpType::Invoke,
            1 => OpType::Ok,
            2 => OpType::Fail,
            _ => OpType::Info,
        },
        f: FUNCTIONS[rng.below(FUNCTIONS.len())].to_string(),
        key: KEYS[rng.below(KEYS.len())].to_string(),
        value: VALUES[rng.below(VALUES.len())],
        expect: VALUES[rng.below(VALUES.len())],
        seq: match rng.below(10) {
            0 => u64::MAX,
            1 => 0,
            _ => rng.next() % 64,
        },
    }
}

/// A structurally valid history built from random operations, so the checkers run on it.
fn random_valid_history(rng: &mut Rng, ops: usize) -> History {
    let mut recs = Vec::new();
    let mut seq = 0u64;
    let mut pending: Vec<(String, u64, String, String)> = Vec::new();
    let complete = |rng: &mut Rng,
                    recs: &mut Vec<OpRecord>,
                    seq: &mut u64,
                    at: usize,
                    pending: &mut Vec<(String, u64, String, String)>| {
        let (client, id, f, key) = pending.remove(at);
        *seq += 1;
        let kind = [OpType::Ok, OpType::Fail, OpType::Info][rng.below(3)];
        recs.push(OpRecord {
            client,
            id,
            kind,
            f,
            key,
            value: VALUES[rng.below(VALUES.len())],
            expect: None,
            seq: *seq,
        });
    };
    for id in 0..ops as u64 {
        let f = FUNCTIONS[rng.below(7)];
        let key = KEYS[rng.below(2)];
        let client = format!("c{}", rng.below(4));
        // One operation at a time per client: finish its open one first.
        if let Some(at) = pending.iter().position(|p| p.0 == client) {
            complete(rng, &mut recs, &mut seq, at, &mut pending);
        }
        let id = id + 1000 * rng.below(3) as u64;
        seq += 1;
        recs.push(OpRecord {
            client: client.clone(),
            id,
            kind: OpType::Invoke,
            f: f.to_string(),
            key: key.to_string(),
            value: VALUES[rng.below(VALUES.len())],
            expect: VALUES[rng.below(VALUES.len())],
            seq,
        });
        pending.push((client, id, f.to_string(), key.to_string()));
        while !pending.is_empty() && rng.chance(60) {
            let at = rng.below(pending.len());
            complete(rng, &mut recs, &mut seq, at, &mut pending);
        }
    }
    match History::from_records(&recs, Completeness::Complete) {
        Ok(h) => h,
        Err(_) => History::from_records(&[], Completeness::Complete).unwrap(),
    }
}

fn run_all(h: &History, limits: &Limits, finals: &[i64]) -> Vec<Verdict> {
    let mut out = Vec::new();
    for key in ["a", "b", "", "k\u{1f600}", "missing"] {
        out.push(check_set(h, key, finals, limits));
        out.push(check_queue(h, key, finals, limits));
        out.push(check_wal_prefix(h, key, finals, limits));
        out.push(check_register_key(h, key, None, limits));
    }
    out.push(check_register_linearizable(h, Some(0), limits));
    out
}

#[test]
fn arbitrary_records_never_panic_the_history_builder() {
    let mut rng = Rng(0xF022);
    for _ in 0..3000 {
        let n = rng.below(30);
        let recs: Vec<OpRecord> = (0..n).map(|_| random_record(&mut rng)).collect();
        let a = History::from_records(&recs, Completeness::Complete);
        let b = History::from_records(&recs, Completeness::Complete);
        assert_eq!(a, b);
        if let Ok(h) = a {
            let _ = h.keys();
            let _ = run_all(&h, &Limits::default(), &[1]);
        }
    }
}

#[test]
fn checkers_on_random_valid_histories_are_total_and_deterministic() {
    let mut rng = Rng(0xFA11);
    let tight = Limits {
        max_steps: 200,
        max_memo_entries: 50,
        max_ops_per_key: 40,
        max_witness_steps: 200,
    };
    for _ in 0..800 {
        let ops = 1 + rng.below(25);
        let h = random_valid_history(&mut rng, ops);
        let finals: Vec<i64> = (0..rng.below(4)).map(|_| rng.below(3) as i64).collect();
        for limits in [tight, Limits::default()] {
            let first = run_all(&h, &limits, &finals);
            assert_eq!(first, run_all(&h, &limits, &finals));
        }
    }
}

#[test]
fn unknown_is_never_a_pass_whatever_the_limits() {
    let mut rng = Rng(0x0DD);
    for _ in 0..300 {
        let h = random_valid_history(&mut rng, 12);
        let none = Limits {
            max_steps: 0,
            max_memo_entries: 0,
            max_ops_per_key: 0,
            max_witness_steps: 0,
        };
        for v in run_all(&h, &none, &[]) {
            assert_eq!(v.is_pass(), matches!(v, Verdict::Ok));
            assert_eq!(v.is_conclusive(), !matches!(v, Verdict::Unknown(_)));
        }
    }
}

#[test]
fn a_large_adversarial_register_history_stays_inside_its_limits() {
    // 3000 overlapping writes then contradictory reads: the worst case for the search.
    let mut recs = Vec::new();
    let mut seq = 0u64;
    let n = 3000u64;
    for i in 0..n {
        seq += 1;
        recs.push(OpRecord {
            client: format!("c{i}"),
            id: 0,
            kind: OpType::Invoke,
            f: "write".into(),
            key: "k".into(),
            value: Some(i as i64),
            expect: None,
            seq,
        });
    }
    for i in 0..n {
        seq += 1;
        recs.push(OpRecord {
            client: format!("c{i}"),
            id: 0,
            kind: OpType::Ok,
            f: "write".into(),
            key: "k".into(),
            value: None,
            expect: None,
            seq,
        });
    }
    for (j, v) in [1i64, 2].into_iter().enumerate() {
        for kind in [OpType::Invoke, OpType::Ok] {
            seq += 1;
            recs.push(OpRecord {
                client: "r".into(),
                id: j as u64,
                kind,
                f: "read".into(),
                key: "k".into(),
                value: if kind == OpType::Ok { Some(v) } else { None },
                expect: None,
                seq,
            });
        }
    }
    let h = History::from_records(&recs, Completeness::Complete).unwrap();
    let limits = Limits {
        max_steps: 100_000,
        max_memo_entries: 20_000,
        max_ops_per_key: 5_000,
        max_witness_steps: 20_000,
    };
    let started = Instant::now();
    let v = check_register_linearizable(&h, None, &limits);
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "took {:?}",
        started.elapsed()
    );
    assert!(!v.is_pass(), "{v:?}");
    assert_eq!(v, check_register_linearizable(&h, None, &limits));
}

#[test]
fn extreme_values_and_sequence_numbers_do_not_overflow() {
    let recs = vec![
        OpRecord {
            client: "c".into(),
            id: u64::MAX,
            kind: OpType::Invoke,
            f: "write".into(),
            key: "k".into(),
            value: Some(i64::MIN),
            expect: None,
            seq: u64::MAX - 1,
        },
        OpRecord {
            client: "c".into(),
            id: u64::MAX,
            kind: OpType::Ok,
            f: "write".into(),
            key: "k".into(),
            value: None,
            expect: None,
            seq: u64::MAX,
        },
        OpRecord {
            client: "d".into(),
            id: 0,
            kind: OpType::Invoke,
            f: "read".into(),
            key: "k".into(),
            value: None,
            expect: None,
            seq: 0,
        },
        OpRecord {
            client: "d".into(),
            id: 0,
            kind: OpType::Ok,
            f: "read".into(),
            key: "k".into(),
            value: Some(i64::MAX),
            expect: None,
            seq: 1,
        },
    ];
    let h = History::from_records(&recs, Completeness::Complete).unwrap();
    assert!(check_register_linearizable(&h, None, &Limits::default())
        .violation()
        .is_some());
    let _ = check_set(&h, "k", &[i64::MIN, i64::MAX], &Limits::default());
    let _ = check_queue(&h, "k", &[i64::MIN], &Limits::default());
    let _ = check_wal_prefix(&h, "k", &[i64::MAX], &Limits::default());
}

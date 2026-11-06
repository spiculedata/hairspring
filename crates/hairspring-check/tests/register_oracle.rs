//! The register checker against a brute-force oracle: every permutation of every subset of the optional operations.
//! The oracle shares no code with the checker.

mod common;

use common::{Hb, Rng};
use hairspring_check::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct Spec {
    key: String,
    invoke: u64,
    complete: u64, // u64::MAX when the operation never has to finish
    required: bool,
    kind: Kind,
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Write(i64),
    Read(Option<i64>),
    Cas(i64, i64),
}

fn specs(h: &History) -> Vec<Spec> {
    let mut out = Vec::new();
    for o in h.operations() {
        let (kind, required) = match (o.f.as_str(), o.outcome) {
            ("write", Outcome::Ok(_)) => (Kind::Write(o.arg.unwrap()), true),
            ("write", Outcome::Info) => (Kind::Write(o.arg.unwrap()), false),
            ("read", Outcome::Ok(r)) => (Kind::Read(r), true),
            ("cas", Outcome::Ok(_)) => (Kind::Cas(o.expect.unwrap(), o.arg.unwrap()), true),
            ("cas", Outcome::Info) => (Kind::Cas(o.expect.unwrap(), o.arg.unwrap()), false),
            _ => continue, // failed operations and unknown reads have no effect
        };
        let complete = if required {
            o.complete_seq.unwrap()
        } else {
            u64::MAX
        };
        out.push(Spec {
            key: o.key.clone(),
            invoke: o.invoke_seq,
            complete,
            required,
            kind,
        });
    }
    out
}

fn permutations(items: &mut Vec<usize>, k: usize, visit: &mut dyn FnMut(&[usize]) -> bool) -> bool {
    if k == items.len() {
        return visit(items);
    }
    for i in k..items.len() {
        items.swap(k, i);
        if permutations(items, k + 1, visit) {
            items.swap(k, i);
            return true;
        }
        items.swap(k, i);
    }
    false
}

fn legal(order: &[usize], ops: &[Spec], initial: Option<i64>) -> bool {
    // Real time: if a finished before b started, a must come first.
    for (pa, &a) in order.iter().enumerate() {
        for &b in &order[..pa] {
            if ops[a].complete < ops[b].invoke {
                return false; // a must precede b but b comes first
            }
        }
    }
    let mut state: BTreeMap<&str, Option<i64>> = BTreeMap::new();
    for &i in order {
        let op = &ops[i];
        let cur = *state.entry(op.key.as_str()).or_insert(initial);
        match op.kind {
            Kind::Write(v) => {
                state.insert(op.key.as_str(), Some(v));
            }
            Kind::Read(r) => {
                if cur != r {
                    return false;
                }
            }
            Kind::Cas(e, n) => {
                if cur != Some(e) {
                    return false;
                }
                state.insert(op.key.as_str(), Some(n));
            }
        }
    }
    true
}

fn oracle_linearizable(ops: &[Spec], initial: Option<i64>) -> bool {
    let required: Vec<usize> = (0..ops.len()).filter(|&i| ops[i].required).collect();
    let optional: Vec<usize> = (0..ops.len()).filter(|&i| !ops[i].required).collect();
    for mask in 0..(1u32 << optional.len()) {
        let mut chosen = required.clone();
        for (b, &o) in optional.iter().enumerate() {
            if mask & (1 << b) != 0 {
                chosen.push(o);
            }
        }
        if permutations(&mut chosen, 0, &mut |order| legal(order, ops, initial)) {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy)]
struct Variant {
    f: &'static str,
    arg: Option<i64>,
    expect: Option<i64>,
    result: Option<i64>,
    info: bool,
}

const VARIANTS: [Variant; 11] = [
    Variant {
        f: "write",
        arg: Some(1),
        expect: None,
        result: None,
        info: false,
    },
    Variant {
        f: "write",
        arg: Some(1),
        expect: None,
        result: None,
        info: true,
    },
    Variant {
        f: "write",
        arg: Some(2),
        expect: None,
        result: None,
        info: false,
    },
    Variant {
        f: "write",
        arg: Some(2),
        expect: None,
        result: None,
        info: true,
    },
    Variant {
        f: "read",
        arg: None,
        expect: None,
        result: None,
        info: false,
    },
    Variant {
        f: "read",
        arg: None,
        expect: None,
        result: Some(1),
        info: false,
    },
    Variant {
        f: "read",
        arg: None,
        expect: None,
        result: Some(2),
        info: false,
    },
    Variant {
        f: "cas",
        arg: Some(2),
        expect: Some(1),
        result: None,
        info: false,
    },
    Variant {
        f: "cas",
        arg: Some(2),
        expect: Some(1),
        result: None,
        info: true,
    },
    Variant {
        f: "cas",
        arg: Some(1),
        expect: Some(2),
        result: None,
        info: false,
    },
    Variant {
        f: "cas",
        arg: Some(1),
        expect: Some(2),
        result: None,
        info: true,
    },
];

/// Build a history from variants and an event order: each entry of `order` is an op index; the first occurrence is its
/// invoke, the second its completion.
fn build(variants: &[Variant], order: &[usize], pending_mask: u32) -> History {
    let mut b = Hb::new();
    let mut seen = vec![false; variants.len()];
    for &i in order {
        let v = variants[i];
        let client = format!("c{i}");
        if !seen[i] {
            seen[i] = true;
            if v.f == "cas" {
                b.invoke_cas(&client, 0, "k", v.expect.unwrap(), v.arg.unwrap());
            } else {
                b.invoke(&client, 0, v.f, "k", v.arg);
            }
        } else if v.info {
            if pending_mask & (1 << i) == 0 {
                b.info(&client, 0, v.f, "k");
            }
        } else {
            b.ok(&client, 0, v.f, "k", v.result);
        }
    }
    b.build()
}

fn event_orders(n: usize) -> Vec<Vec<usize>> {
    fn go(counts: &mut Vec<u8>, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if counts.iter().all(|&c| c == 2) {
            out.push(cur.clone());
            return;
        }
        for i in 0..counts.len() {
            if counts[i] < 2 {
                counts[i] += 1;
                cur.push(i);
                go(counts, cur, out);
                cur.pop();
                counts[i] -= 1;
            }
        }
    }
    let mut out = Vec::new();
    go(&mut vec![0; n], &mut Vec::new(), &mut out);
    out
}

fn agrees(h: &History, initial: Option<i64>, label: &str) {
    let expected = oracle_linearizable(&specs(h), initial);
    let got = check_register_linearizable(h, initial, &Limits::default());
    match (expected, &got) {
        (true, Verdict::Ok) => {}
        (false, Verdict::Violation(_)) => {}
        _ => panic!(
            "oracle says linearizable={expected}, checker says {got:?} for {label}: {:#?}",
            h.operations()
        ),
    }
}

#[test]
fn exhaustive_histories_of_up_to_three_operations_agree_with_the_oracle() {
    let mut checked = 0u64;
    for n in 1..=3usize {
        let orders = event_orders(n);
        let combos = VARIANTS.len().pow(n as u32);
        for code in 0..combos {
            let mut c = code;
            let mut chosen = Vec::new();
            for _ in 0..n {
                chosen.push(VARIANTS[c % VARIANTS.len()]);
                c /= VARIANTS.len();
            }
            for order in &orders {
                let h = build(&chosen, order, 0);
                agrees(&h, None, "exhaustive");
                checked += 1;
            }
        }
    }
    assert!(checked > 100_000, "only {checked}");
}

#[test]
fn random_histories_of_up_to_seven_operations_agree_with_the_oracle() {
    let mut rng = Rng(0xA11CE);
    for round in 0..4000 {
        let n = 4 + rng.below(4);
        let variants: Vec<Variant> = (0..n)
            .map(|_| VARIANTS[rng.below(VARIANTS.len())])
            .collect();
        // Random valid event order.
        let mut remaining: Vec<usize> = (0..n).flat_map(|i| [i, i]).collect();
        let mut order = Vec::new();
        while !remaining.is_empty() {
            order.push(remaining.remove(rng.below(remaining.len())));
        }
        let pending = rng.next() as u32;
        let initial = if rng.chance(20) { Some(1) } else { None };
        let h = build(&variants, &order, pending);
        agrees(&h, initial, &format!("round {round}"));
    }
}

#[test]
fn random_violations_have_witnesses_the_oracle_confirms_and_cannot_shrink() {
    let mut rng = Rng(0xBEEF);
    let mut violations = 0;
    for _ in 0..3000 {
        let n = 4 + rng.below(3);
        let variants: Vec<Variant> = (0..n)
            .map(|_| VARIANTS[rng.below(VARIANTS.len())])
            .collect();
        let mut remaining: Vec<usize> = (0..n).flat_map(|i| [i, i]).collect();
        let mut order = Vec::new();
        while !remaining.is_empty() {
            order.push(remaining.remove(rng.below(remaining.len())));
        }
        let h = build(&variants, &order, 0);
        let Verdict::Violation(v) = check_register_linearizable(&h, None, &Limits::default())
        else {
            continue;
        };
        violations += 1;
        assert!(v.witness_minimal);
        let all = specs(&h);
        let in_witness = |o: &Spec| v.witness.iter().any(|w| w.invoke_seq == o.invoke);
        let witness_specs: Vec<Spec> = all.iter().filter(|o| in_witness(o)).cloned().collect();
        assert!(
            !oracle_linearizable(&witness_specs, None),
            "witness is linearizable"
        );
        for skip in 0..witness_specs.len() {
            let rest: Vec<Spec> = witness_specs
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, o)| o.clone())
                .collect();
            assert!(oracle_linearizable(&rest, None), "witness not 1-minimal");
        }
    }
    assert!(
        violations > 200,
        "too few violations generated: {violations}"
    );
}

#[test]
fn per_key_decomposition_matches_the_joint_oracle() {
    let mut rng = Rng(0xD0D0);
    for _ in 0..1500 {
        let n = 4 + rng.below(3);
        let variants: Vec<Variant> = (0..n)
            .map(|_| VARIANTS[rng.below(VARIANTS.len())])
            .collect();
        let mut remaining: Vec<usize> = (0..n).flat_map(|i| [i, i]).collect();
        let mut order = Vec::new();
        while !remaining.is_empty() {
            order.push(remaining.remove(rng.below(remaining.len())));
        }
        // Spread operations over two keys.
        let keys: Vec<&str> = (0..n)
            .map(|_| if rng.chance(50) { "a" } else { "b" })
            .collect();
        let mut b = Hb::new();
        let mut seen = vec![false; n];
        for &i in &order {
            let v = variants[i];
            let c = format!("c{i}");
            if !seen[i] {
                seen[i] = true;
                if v.f == "cas" {
                    b.invoke_cas(&c, 0, keys[i], v.expect.unwrap(), v.arg.unwrap());
                } else {
                    b.invoke(&c, 0, v.f, keys[i], v.arg);
                }
            } else if v.info {
                b.info(&c, 0, v.f, keys[i]);
            } else {
                b.ok(&c, 0, v.f, keys[i], v.result);
            }
        }
        let h = b.build();
        let expected = oracle_linearizable(&specs(&h), None);
        let got = check_register_linearizable(&h, None, &Limits::default());
        assert_eq!(expected, got.is_pass(), "{got:?}");
        assert!(got.is_conclusive());
    }
}

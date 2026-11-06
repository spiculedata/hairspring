//! The write-ahead-log checker against its own brute-force oracle, on random multi-client histories.
//!
//! The model the oracle encodes, independently of the checker: an acknowledged (`ok`) append is in the recovered log;
//! a failed append is not; an unknown (`info` or never completed) append may be absent, or present at any position
//! after its invoke; the recovered log is a prefix of the log that landed, so it holds every acknowledged append and
//! may omit a tail of unknown ones. An append that finished before another began lands before it.

mod common;

use common::Rng;
use hairspring_check::*;

const HISTORIES: u64 = 300_000;
const MAX_OPS: usize = 7;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fate {
    Ok,
    Fail,
    Info,
    Pending,
}

#[derive(Clone, Debug)]
struct Spec {
    value: i64,
    invoke: u64,
    complete: Option<u64>,
    fate: Fate,
}

impl Spec {
    fn unknown(&self) -> bool {
        matches!(self.fate, Fate::Info | Fate::Pending)
    }
}

fn record(client: usize, id: usize, kind: OpType, value: Option<i64>, seq: u64) -> OpRecord {
    OpRecord {
        client: format!("c{client}"),
        id: id as u64,
        kind,
        f: "append".into(),
        key: "log".into(),
        value,
        expect: None,
        seq,
    }
}

/// Random interleaving of three clients, each running one append at a time. A client whose append never completes
/// stays silent for the rest of the run.
fn generate(rng: &mut Rng) -> (Vec<Spec>, Vec<OpRecord>) {
    let mut specs: Vec<Spec> = Vec::new();
    let mut records = Vec::new();
    let mut open: [Option<usize>; 3] = [None; 3];
    let mut silent = [false; 3];
    let (mut t, mut last) = (0u64, 0usize);
    for _ in 0..2 + rng.below(12) {
        let c = if rng.chance(70) { last } else { rng.below(3) };
        last = c;
        if silent[c] {
            continue;
        }
        t += 1;
        match open[c].take() {
            Some(i) => {
                let roll = rng.below(100);
                let (fate, kind) = match roll {
                    0..=49 => (Fate::Ok, Some(OpType::Ok)),
                    50..=64 => (Fate::Fail, Some(OpType::Fail)),
                    65..=84 => (Fate::Info, Some(OpType::Info)),
                    _ => (Fate::Pending, None),
                };
                specs[i].fate = fate;
                match kind {
                    Some(kind) => {
                        specs[i].complete = Some(t);
                        records.push(record(c, i, kind, None, t));
                    }
                    None => silent[c] = true,
                }
            }
            None if specs.len() < MAX_OPS => {
                let value = 1 + rng.below(4) as i64;
                specs.push(Spec {
                    value,
                    invoke: t,
                    complete: None,
                    fate: Fate::Pending,
                });
                open[c] = Some(specs.len() - 1);
                records.push(record(c, specs.len() - 1, OpType::Invoke, Some(value), t));
            }
            None => {}
        }
    }
    (specs, records)
}

/// Can `recovered` be the log after some valid execution?
fn oracle_valid(specs: &[Spec], recovered: &[i64]) -> bool {
    fn go(specs: &[Spec], recovered: &[i64], pos: usize, placed: u32) -> bool {
        let is_placed = |j: usize| placed >> j & 1 == 1;
        if pos == recovered.len() {
            return specs
                .iter()
                .enumerate()
                .all(|(i, s)| s.fate != Fate::Ok || is_placed(i));
        }
        for (i, s) in specs.iter().enumerate() {
            if is_placed(i) || s.fate == Fate::Fail || s.value != recovered[pos] {
                continue;
            }
            let blocked = specs.iter().enumerate().any(|(j, y)| {
                j != i && !is_placed(j) && y.fate == Fate::Ok && y.complete.unwrap() < s.invoke
            });
            if !blocked && go(specs, recovered, pos + 1, placed | 1 << i) {
                return true;
            }
        }
        false
    }
    go(specs, recovered, 0, 0)
}

/// Recovered logs to try: plausible ones (acknowledged appends in order with some unknown ones mixed in) and
/// mutations of them (a loss, a duplicate, a swap), plus noise.
fn candidates(rng: &mut Rng, specs: &[Spec]) -> Vec<Vec<i64>> {
    let mut plausible: Vec<i64> = specs
        .iter()
        .filter(|s| s.fate == Fate::Ok)
        .map(|s| s.value)
        .collect();
    for s in specs.iter().filter(|s| s.unknown()) {
        if rng.chance(50) {
            let at = rng.below(plausible.len() + 1);
            plausible.insert(at, s.value);
        }
    }
    let mut out = vec![plausible.clone()];
    if !plausible.is_empty() {
        let mut dropped = plausible.clone();
        dropped.remove(rng.below(dropped.len()));
        out.push(dropped);
        let mut duplicated = plausible.clone();
        duplicated.push(plausible[rng.below(plausible.len())]);
        out.push(duplicated);
        let mut swapped = plausible.clone();
        let (a, b) = (rng.below(swapped.len()), rng.below(swapped.len()));
        swapped.swap(a, b);
        out.push(swapped);
    }
    out.push((0..rng.below(5)).map(|_| 1 + rng.below(5) as i64).collect());
    out
}

/// An unknown append followed (in invoke order) by any non-failed append.
fn has_unordered_shape(specs: &[Spec]) -> bool {
    specs.iter().enumerate().any(|(i, a)| {
        a.unknown()
            && specs
                .iter()
                .enumerate()
                .any(|(j, b)| j != i && b.invoke > a.invoke && b.fate != Fate::Fail)
    })
}

/// A completed append (ok or failed) that was still running when a later one began.
fn has_overlap(specs: &[Spec]) -> bool {
    specs.iter().any(|a| {
        matches!(a.fate, Fate::Ok | Fate::Fail)
            && specs
                .iter()
                .any(|b| b.invoke > a.invoke && b.invoke < a.complete.unwrap())
    })
}

#[derive(Default)]
struct Stats {
    checked: u64,
    conclusive: u64,
    unordered_violations: u64,
    unordered_phantoms: u64,
    unordered_losses: u64,
    unordered_unknown: u64,
}

#[test]
fn no_verdict_the_oracle_disagrees_with_on_random_multi_client_histories() {
    let mut rng = Rng(0x0DA1_5EED);
    let limits = Limits::default();
    let mut stats = Stats::default();
    for round in 0..HISTORIES {
        let (specs, records) = generate(&mut rng);
        let history = History::from_records(&records, Completeness::Complete)
            .expect("generated histories are well formed");
        let unordered = has_unordered_shape(&specs);
        let overlap = has_overlap(&specs);
        for recovered in candidates(&mut rng, &specs) {
            let valid = oracle_valid(&specs, &recovered);
            let verdict = check_wal_prefix(&history, "log", &recovered, &limits);
            stats.checked += 1;
            let context =
                || format!("round {round}: {specs:?} recovered {recovered:?} -> {verdict:?}");
            match &verdict {
                Verdict::Ok => assert!(valid && !unordered, "unsound Ok: {}", context()),
                Verdict::Violation(_) => assert!(!valid, "unsound violation: {}", context()),
                Verdict::Unknown(_) => {
                    assert!(unordered || overlap, "needless Unknown: {}", context());
                }
            }
            if !unordered && !overlap {
                assert_eq!(verdict.is_pass(), valid, "{}", context());
                assert!(verdict.is_conclusive(), "{}", context());
            }
            if verdict.is_conclusive() {
                stats.conclusive += 1;
            }
            if unordered && !overlap {
                match &verdict {
                    Verdict::Violation(v) => {
                        stats.unordered_violations += 1;
                        assert!(!v.witness.is_empty(), "{}", context());
                        match v.kind {
                            ViolationKind::Phantom { .. } => stats.unordered_phantoms += 1,
                            ViolationKind::LostAcknowledgedWrite { .. } => {
                                stats.unordered_losses += 1
                            }
                            _ => panic!("unexpected kind in the unordered shape: {}", context()),
                        }
                    }
                    Verdict::Unknown(_) => stats.unordered_unknown += 1,
                    Verdict::Ok => unreachable!(),
                }
            }
        }
    }
    // The run must actually exercise the narrowed rule, not only agree vacuously.
    assert!(
        stats.conclusive > stats.checked / 4,
        "{}/{}",
        stats.conclusive,
        stats.checked
    );
    assert!(
        stats.unordered_phantoms > 1_000,
        "{}",
        stats.unordered_phantoms
    );
    assert!(stats.unordered_losses > 1_000, "{}", stats.unordered_losses);
    assert!(
        stats.unordered_unknown > 1_000,
        "{}",
        stats.unordered_unknown
    );
    assert!(stats.unordered_violations > 2_000);
}

fn e1_history(retry: bool) -> (Vec<Spec>, History) {
    // Writer w1 appends 1 (acknowledged), appends 2 and is killed (the client records info), then a restarted writer
    // under a NEW name appends 3, preceded in the retry variant by a retry of 2.
    let mut records = vec![
        record(1, 1, OpType::Invoke, Some(1), 1),
        record(1, 1, OpType::Ok, None, 2),
        record(1, 2, OpType::Invoke, Some(2), 3),
        record(1, 2, OpType::Info, None, 4),
    ];
    let mut specs = vec![
        Spec {
            value: 1,
            invoke: 1,
            complete: Some(2),
            fate: Fate::Ok,
        },
        Spec {
            value: 2,
            invoke: 3,
            complete: Some(4),
            fate: Fate::Info,
        },
    ];
    let mut t = 5;
    if retry {
        records.push(record(2, 3, OpType::Invoke, Some(2), t));
        records.push(record(2, 3, OpType::Ok, None, t + 1));
        specs.push(Spec {
            value: 2,
            invoke: t,
            complete: Some(t + 1),
            fate: Fate::Ok,
        });
        t += 2;
    }
    records.push(record(2, 4, OpType::Invoke, Some(3), t));
    records.push(record(2, 4, OpType::Ok, None, t + 1));
    specs.push(Spec {
        value: 3,
        invoke: t,
        complete: Some(t + 1),
        fate: Fate::Ok,
    });
    (
        specs,
        History::from_records(&records, Completeness::Complete).unwrap(),
    )
}

#[test]
fn e1_shapes_get_the_verdicts_the_oracle_allows() {
    let limits = Limits::default();
    let (specs, history) = e1_history(false);
    let expect = |recovered: &[i64], valid: bool, check: &dyn Fn(&Verdict)| {
        assert_eq!(oracle_valid(&specs, recovered), valid, "{recovered:?}");
        check(&check_wal_prefix(&history, "log", recovered, &limits));
    };
    let unknown = |v: &Verdict| {
        assert!(
            matches!(v, Verdict::Unknown(UnknownReason::UnsupportedShape { .. })),
            "{v:?}"
        )
    };
    expect(&[1, 2, 3], true, &unknown);
    expect(&[1, 3], true, &unknown);
    expect(&[3], false, &|v| {
        assert_eq!(
            v.violation().unwrap().kind,
            ViolationKind::LostAcknowledgedWrite { value: 1 }
        );
        assert!(!v.violation().unwrap().witness.is_empty());
    });
    expect(&[1], false, &|v| {
        assert_eq!(
            v.violation().unwrap().kind,
            ViolationKind::LostAcknowledgedWrite { value: 3 }
        );
    });
    expect(&[9], false, &|v| {
        let v = v.violation().unwrap();
        assert_eq!(v.kind, ViolationKind::Phantom { value: 9 });
        assert!(!v.witness.is_empty());
    });
}

#[test]
fn e1_retry_shapes_agree_with_the_oracle() {
    let limits = Limits::default();
    let (specs, history) = e1_history(true);
    for recovered in [
        &[1, 2, 2, 3][..],
        &[1, 2, 3],
        &[1, 3],
        &[1, 2, 2],
        &[3],
        &[1],
        &[9],
        &[1, 2, 2, 2, 3],
        &[2, 1, 2, 3],
    ] {
        let verdict = check_wal_prefix(&history, "log", recovered, &limits);
        let valid = oracle_valid(&specs, recovered);
        match verdict {
            Verdict::Ok => panic!("an unknown append precedes later appends: {recovered:?}"),
            Verdict::Violation(_) => assert!(!valid, "{recovered:?}"),
            Verdict::Unknown(_) => {}
        }
    }
    // A third copy of 2 is beyond the two non-failed appends of 2.
    let v = check_wal_prefix(&history, "log", &[1, 2, 2, 2, 3], &limits);
    assert_eq!(
        v.violation().unwrap().kind,
        ViolationKind::Phantom { value: 2 }
    );
}

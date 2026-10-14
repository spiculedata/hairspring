//! Exp 3: "snapshot and branch from interesting states" vs restart-from-zero random.
//! Metric: total simulation steps executed until the first violation.
use anthill_experiments::*;
use std::collections::HashMap;

const TRIALS: u64 = 300;
const WALK: usize = 60;
const BRANCH_LEN: usize = 12;
const BUDGET: u64 = 5_000_000;

fn random_restarts(seed: u64) -> Option<u64> {
    let mut rng = Rng(seed);
    let mut steps = 0;
    while steps < BUDGET {
        let mut w = World::new();
        for _ in 0..WALK {
            let en = w.enabled();
            if en.is_empty() {
                break;
            }
            w.apply(en[rng.below(en.len())]);
            steps += 1;
            if w.violated {
                return Some(steps);
            }
        }
    }
    None
}
fn guided_branching(seed: u64) -> Option<u64> {
    let mut rng = Rng(seed);
    let mut steps = 0;
    let mut corpus: Vec<World> = vec![World::new()];
    let mut seen: HashMap<u64, u32> = HashMap::new();
    seen.insert(corpus[0].signature(), 1);
    while steps < BUDGET {
        // favour snapshots whose abstract state has been visited least
        let k = {
            let a = rng.below(corpus.len());
            let b = rng.below(corpus.len());
            if seen[&corpus[a].signature()] <= seen[&corpus[b].signature()] {
                a
            } else {
                b
            }
        };
        let mut w = corpus[k].clone(); // <- the "time travel": restore snapshot k
        for _ in 0..BRANCH_LEN {
            let en = w.enabled();
            if en.is_empty() {
                break;
            }
            w.apply(en[rng.below(en.len())]);
            steps += 1;
            if w.violated {
                return Some(steps);
            }
            let e = seen.entry(w.signature()).or_insert(0);
            if *e == 0 {
                corpus.push(w.clone());
            }
            *e += 1;
        }
    }
    None
}
fn stats(name: &str, mut v: Vec<Option<u64>>) {
    let miss = v.iter().filter(|x| x.is_none()).count();
    let mut f: Vec<u64> = v.drain(..).flatten().collect();
    f.sort();
    if f.is_empty() {
        println!("{name:18} never found in budget");
        return;
    }
    println!(
        "{name:18} found {}/{TRIALS}  median={:>8}  p90={:>8}  mean={:>10.0} steps",
        TRIALS as usize - miss,
        f[f.len() / 2],
        f[f.len() * 9 / 10],
        f.iter().sum::<u64>() as f64 / f.len() as f64
    );
}
fn main() {
    stats(
        "random restarts",
        (1..=TRIALS).map(random_restarts).collect(),
    );
    stats(
        "snapshot branching",
        (1..=TRIALS).map(guided_branching).collect(),
    );
}

//! Exp 1: how non-deterministic is an ordinary program, in-process?
use anthill_experiments::fnv;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

fn thread_interleaving() -> u64 {
    let log = Arc::new(Mutex::new(Vec::<u8>::new()));
    let hs: Vec<_> = (0..4u8)
        .map(|id| {
            let l = log.clone();
            thread::spawn(move || {
                for _ in 0..200 {
                    l.lock().unwrap().push(id);
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let v = log.lock().unwrap().clone();
    fnv(&v)
}
fn hashmap_order() -> u64 {
    let m: HashMap<String, i32> = (0..20).map(|i| (format!("k{i}"), i)).collect();
    fnv(m.keys().cloned().collect::<Vec<_>>().join(",").as_bytes())
}
fn main() {
    const RUNS: usize = 100;
    let t: HashSet<u64> = (0..RUNS).map(|_| thread_interleaving()).collect();
    let m: HashSet<u64> = (0..RUNS).map(|_| hashmap_order()).collect();
    let clocks: HashSet<u128> = (0..RUNS)
        .map(|_| Instant::now().elapsed().as_nanos())
        .collect();
    println!("{RUNS} runs of the *same* code, same inputs:");
    println!("  thread interleavings : {} distinct outcomes", t.len());
    println!(
        "  HashMap iteration    : {} distinct outcomes (RandomState seeded per map)",
        m.len()
    );
    println!("  clock readings       : {} distinct values", clocks.len());
}

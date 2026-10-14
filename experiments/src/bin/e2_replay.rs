//! Exp 2: seed -> trace -> hash is a pure function; replay from the recorded
//! choices reproduces the run bit-for-bit, and a failing run can be found/shrunk.
use hairspring_experiments::*;

fn run(seed: u64, max_steps: usize) -> (World, Vec<usize>, u64) {
    let (mut w, mut rng, mut choices, mut h) =
        (World::new(), Rng(seed), vec![], 0xcbf29ce484222325u64);
    for _ in 0..max_steps {
        let en = w.enabled();
        if en.is_empty() || w.violated {
            break;
        }
        let c = rng.below(en.len());
        choices.push(c);
        h = fnv(&[
            h.to_le_bytes().as_slice(),
            format!("{:?}", en[c]).as_bytes(),
        ]
        .concat());
        w.apply(en[c]);
    }
    (w, choices, h)
}
fn replay(choices: &[usize]) -> (World, u64) {
    let (mut w, mut h) = (World::new(), 0xcbf29ce484222325u64);
    for &c in choices {
        let en = w.enabled();
        h = fnv(&[
            h.to_le_bytes().as_slice(),
            format!("{:?}", en[c]).as_bytes(),
        ]
        .concat());
        w.apply(en[c]);
    }
    (w, h)
}
fn main() {
    let (_, _, h1) = run(42, 60);
    let (_, _, h2) = run(42, 60);
    let (_, _, h3) = run(43, 60);
    println!("seed 42 twice : {h1:016x} == {h2:016x} ? {}", h1 == h2);
    println!("seed 43       : {h3:016x} differs ? {}", h1 != h3);

    let found = (0..5_000_000u64).find(|s| run(*s, 60).0.violated);
    let Some(seed) = found else {
        println!("no violation found");
        return;
    };
    let (w, choices, h) = run(seed, 60);
    let (w2, h2) = replay(&choices);
    println!(
        "\nfirst failing seed: {seed} ({} steps), violated={}",
        choices.len(),
        w.violated
    );
    println!(
        "replay from recorded choices: hash match={} violated={}",
        h == h2,
        w2.violated
    );

    // Shrink: greedily drop single choices while the failure persists (by re-simulating prefixes).
    let mut best = choices.clone();
    let still_fails = |cs: &[usize]| {
        let mut w = World::new();
        for &c in cs {
            let en = w.enabled();
            if c >= en.len() {
                return false;
            }
            w.apply(en[c]);
            if w.violated {
                return true;
            }
        }
        false
    };
    let mut i = 0;
    while i < best.len() {
        let mut t = best.clone();
        t.remove(i);
        if still_fails(&t) {
            best = t
        } else {
            i += 1
        }
    }
    println!(
        "shrunk {} -> {} steps. minimal trace:",
        choices.len(),
        best.len()
    );
    let mut w = World::new();
    for &c in &best {
        let en = w.enabled();
        println!("  t={} {:?}", w.time, en[c]);
        w.apply(en[c]);
        if w.violated {
            println!("  => INVARIANT VIOLATED (stale lease holder wrote)");
            break;
        }
    }
}

//! End to end: explore -> shrink -> bundle -> decode -> strict replay.
use anthill_core::bundle::Bundle;
use anthill_core::demo::LockLease;
use anthill_core::doctor::doctor;
use anthill_core::explore::{branching, Config};
use anthill_core::replay::{find_action, replay};
use anthill_core::shrink::shrink;
use anthill_core::Sim;

fn main() {
    let root = LockLease::new();
    let cfg = Config {
        seed: 7,
        ..Config::default()
    };

    let ok = doctor(&root, cfg.seed, 100).expect("target must be deterministic");
    println!("doctor: deterministic, trace hash {ok:016x}");

    let failure = branching(&root, &cfg).expect("bug should be found");
    println!(
        "found violation after {} sim steps, path length {}",
        failure.steps_executed,
        failure.path.len()
    );

    let minimal = shrink(&root, &failure.path);
    println!("shrunk to {} steps", minimal.len());

    let text = Bundle::new(cfg.seed, env!("CARGO_PKG_VERSION"), minimal)
        .with_target("lock-lease")
        .encode();
    println!("\n--- repro.anthill ---\n{text}---------------------\n");

    let bundle = Bundle::decode(&text).expect("bundle decodes");
    let mut state = root.clone();
    for (n, id) in bundle.path.iter().enumerate() {
        let action = find_action(&state, *id).expect("enabled");
        println!("{:>2}. t={} {:?}", n + 1, state.time, action);
        state = state.apply(action);
    }
    let replayed = replay(&root, &bundle.path).expect("strict replay");
    println!(
        "\nreplay violated={} (applied {})",
        replayed.state.violated(),
        replayed.applied
    );
}

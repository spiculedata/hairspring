use anthill_core::bundle::Bundle;
use anthill_core::doctor::doctor;
use anthill_core::explore::{branching, random_restarts, Config};
use anthill_core::replay::replay;
use anthill_core::shrink::shrink;
use anthill_core::Sim;
use anthill_pubsub::{Mode, Order, PubSub};

#[test]
fn doctor_flags_the_naive_hashmap_variant() {
    // Detection is probabilistic: with two deliverable messages a hash-order leak is visible
    // only about half the time per opportunity, so a single seed can miss it. Like the CLI
    // (`anthill doctor --seeds N`), check many seeds; a leak must show up in at least one.
    let root = PubSub::new(Mode::Buggy, Order::HashMap);
    let detected = (1..=40).filter(|seed| doctor(&root, *seed, 40).is_err()).count();
    assert!(detected >= 5, "expected the leak to be detected on many seeds, got {detected}/40");
}

#[test]
fn doctor_passes_the_sorted_variant() {
    let root = PubSub::new(Mode::Buggy, Order::Sorted);
    assert!(doctor(&root, 1, 40).is_ok());
    assert!(doctor(&root, 2, 40).is_ok());
}

#[test]
fn explorer_finds_duplicate_effect_in_buggy_mode_and_it_replays_and_shrinks() {
    let root = PubSub::new(Mode::Buggy, Order::Sorted);
    let f = branching(
        &root,
        &Config {
            seed: 5,
            ..Config::default()
        },
    )
    .expect("duplicate effect must be found");
    assert!(replay(&root, &f.path).unwrap().state.violated());
    let small = shrink(&root, &f.path);
    assert!(small.len() <= f.path.len());
    assert!(replay(&root, &small).unwrap().state.violated());
    let b = Bundle::new(5, "t", small);
    assert_eq!(Bundle::decode(&b.encode()).unwrap(), b);
}

#[test]
fn atomic_mode_has_no_violation_within_a_large_budget() {
    let root = PubSub::new(Mode::Atomic, Order::Sorted);
    let cfg = Config {
        seed: 9,
        max_steps: 300_000,
        ..Config::default()
    };
    assert!(branching(&root, &cfg).is_none());
    assert!(random_restarts(&root, &cfg).is_none());
}

#[test]
fn atomic_mode_is_also_deterministic() {
    assert!(doctor(&PubSub::new(Mode::Atomic, Order::Sorted), 3, 40).is_ok());
}

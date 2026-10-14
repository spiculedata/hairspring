use hairspring_core::doctor::doctor;
use hairspring_core::explore::{branching, Config};
use hairspring_core::replay::find_action;
use hairspring_core::shrink::shrink;
use hairspring_core::Sim;
use hairspring_pubsub::{Mode, Order, PubSub};

fn main() {
    println!(
        "doctor, naive HashMap enumeration : {:?}",
        doctor(&PubSub::new(Mode::Buggy, Order::HashMap), 1, 40).map(|_| "ok")
    );
    println!(
        "doctor, sorted enumeration        : {:?}\n",
        doctor(&PubSub::new(Mode::Buggy, Order::Sorted), 1, 40).map(|_| "ok")
    );

    let root = PubSub::new(Mode::Buggy, Order::Sorted);
    let f = branching(
        &root,
        &Config {
            seed: 5,
            ..Config::default()
        },
    )
    .expect("found");
    let small = shrink(&root, &f.path);
    println!(
        "duplicate effect found in {} steps; path {} -> shrunk {}",
        f.steps_executed,
        f.path.len(),
        small.len()
    );
    let mut s = root.clone();
    for (n, id) in small.iter().enumerate() {
        let a = find_action(&s, *id).unwrap();
        println!("{:>2}. t={} {:?}", n + 1, s.time, a);
        s = s.apply(a);
    }
    println!(
        "effects per message: {:?}  violated={}",
        s.effects,
        s.violated()
    );
}

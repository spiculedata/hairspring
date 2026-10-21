//! `hairspring_sdk::exit` records the status in the `end` line.

use hairspring_sdk::reachable;

fn main() {
    reachable!("exit.reach", "reached");
    hairspring_sdk::exit(3);
}

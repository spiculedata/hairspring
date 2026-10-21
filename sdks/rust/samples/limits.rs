//! Ids and messages at their limits (128 and 512 bytes) fold cleanly.

use hairspring_sdk::reachable;

macro_rules! sixteen {
    () => {
        "0123456789abcdef"
    };
}

macro_rules! id_128 {
    () => {
        concat!(
            sixteen!(),
            sixteen!(),
            sixteen!(),
            sixteen!(),
            sixteen!(),
            sixteen!(),
            sixteen!(),
            sixteen!()
        )
    };
}

macro_rules! msg_512 {
    () => {
        concat!(id_128!(), id_128!(), id_128!(), id_128!())
    };
}

fn main() {
    reachable!(id_128!(), msg_512!());
    reachable!("limits.plain", "");
}

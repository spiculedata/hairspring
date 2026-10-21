//! Mirrors the C SDK's `basic` scenario (sdks/c/tests/fixtures/hs_sample.c): the same ids, messages, values and order,
//! so the two SDKs' reports can be compared line by line.

use hairspring_sdk::{
    always, always_or_unreachable, reachable, sometimes, unreachable_point, work,
};

fn main() {
    let argc = std::env::args().count();
    for i in 0..5 {
        always!(true, "basic.always.ok", "always holds", i, 2);
    }
    always!(false, "basic.always.bad", "always violated", 7, 8);
    for i in 0..4 {
        sometimes!(i % 2 == 0, "basic.sometimes", "sometimes holds", i);
    }
    reachable!("basic.reach.hit", "is reached");
    if argc > 99 {
        // Never taken: these sites exist only in the catalog.
        reachable!("basic.reach.never", "never reached");
        unreachable_point!("basic.unreach.clean", "never reached either");
        always_or_unreachable!(true, "basic.aou.never", "never evaluated");
    }
    work(300);
}

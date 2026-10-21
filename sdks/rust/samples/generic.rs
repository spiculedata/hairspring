//! A call site in a generic function is one record however many ways the function is instantiated. A second call
//! site sits in a module without the implicit prelude: the macros must not depend on names the caller has in scope.

use hairspring_sdk::sometimes;

fn check<T: Into<i64>>(value: T) {
    let value = value.into();
    sometimes!(
        value > 0,
        "generic.positive",
        "the value was positive",
        value
    );
}

mod bare {
    #![no_implicit_prelude]

    pub fn check() {
        ::hairspring_sdk::reachable!("generic.bare", "no implicit prelude");
    }
}

fn main() {
    check(1u8);
    check(-2i32);
    check(3i64);
    bare::check();
}

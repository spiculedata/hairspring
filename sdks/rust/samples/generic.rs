//! A call site in a generic function is one record however many ways the function is instantiated.

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

fn main() {
    check(1u8);
    check(-2i32);
    check(3i64);
}

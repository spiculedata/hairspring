//! The fixed build: the same step, with a mutex around the whole read-modify-write.

#[path = "counter_common.rs"]
mod common;

use std::sync::Mutex;

fn main() {
    let counter = Mutex::new(0u64);
    common::run(
        &|| {
            let mut guard = counter.lock().unwrap_or_else(|e| e.into_inner());
            let seen = *guard;
            common::busy(common::WINDOW);
            *guard = seen + 1;
        },
        &|| *counter.lock().unwrap_or_else(|e| e.into_inner()),
    );
}

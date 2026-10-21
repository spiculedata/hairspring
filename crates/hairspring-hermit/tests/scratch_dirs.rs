//! Tests that run side by side must never be handed the same scratch directory: they write fake executables into it,
//! and one overwriting another's makes a comparison of two different binaries compare a file with itself.
mod common;
use common::scratch;
use std::collections::BTreeSet;
use std::sync::{Arc, Barrier};

const THREADS: usize = 8;
const PER_THREAD: usize = 100;

#[test]
fn concurrent_callers_get_distinct_directories() {
    let start = Arc::new(Barrier::new(THREADS));
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                (0..PER_THREAD).map(|_| scratch("bin")).collect::<Vec<_>>()
            })
        })
        .collect();
    let all: Vec<_> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    let distinct = all.iter().collect::<BTreeSet<_>>().len();
    // Leave nothing behind, whether or not the check below holds.
    for dir in &all {
        let _ = std::fs::remove_dir_all(dir);
    }
    assert_eq!(
        distinct,
        all.len(),
        "some scratch directories were handed out twice"
    );
}

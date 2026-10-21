//! Usage: hs_sample_threads <n>. Starts n threads one after another; each names itself and evaluates once.

use hairspring_sdk::{sometimes, thread, work};

fn main() {
    let n: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(0);
    work(1);
    for idx in 0..n {
        let handle = std::thread::spawn(move || {
            // Past the table the shared slot cannot be named; the thread carries on under its overflow key.
            let _ = thread("w", idx);
            sometimes!(true, "threads.each", "every thread evaluates", idx);
        });
        handle.join().expect("thread joins");
    }
}

//! A raw `fork` whose child exits normally. The child shares the parent's SDK state, so it must not repeat the
//! parent's report: the process that started the SDK reports, and a forked child reports nothing.

use hairspring_sdk::reachable;

extern "C" {
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
}

fn main() {
    reachable!("fork.parent", "parent reaches this");
    // SAFETY: the process is single-threaded here, and the child only exits.
    let pid = unsafe { fork() };
    if pid == 0 {
        std::process::exit(0);
    }
    let mut status = 0;
    // SAFETY: waits for the child just forked, into a valid status slot.
    unsafe { waitpid(pid, &mut status, 0) };
}

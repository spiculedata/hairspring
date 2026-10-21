//! Usage: hs_sample_spawn <image>. Runs another instrumented program as a child process: the child must tell itself
//! apart from its parent, so it takes a derived tag (`<parent>.s<pid>`) and says it is not stable.

use hairspring_sdk::reachable;

fn main() {
    let Some(image) = std::env::args().nth(1) else {
        std::process::exit(2);
    };
    reachable!("spawn.parent", "parent reaches this");
    let status = std::process::Command::new(image)
        .status()
        .expect("child starts");
    if !status.success() {
        std::process::exit(3);
    }
}

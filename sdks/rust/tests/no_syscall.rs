//! Call sites make no system call: a seccomp filter that allows only exit kills the process on any other. The
//! check itself is the sample program; it prints "skip" and succeeds where it cannot run (macOS, other CPUs).

mod common;

use common::run;

#[test]
fn a_million_evaluations_of_every_kind_make_no_system_call() {
    let r = run(env!("CARGO_BIN_EXE_hs_sample_nosyscall"), &[], &[]);
    assert!(r.status.success(), "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.starts_with("ok ") || r.stdout.starts_with("skip:"),
        "{}",
        r.stdout
    );
}

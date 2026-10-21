//! One id at two call sites is one catalogued id with summed counts.

use hairspring_sdk::{reachable, sometimes};

#[inline(never)]
fn merge_a(ok: bool) {
    sometimes!(ok, "merge.shared", "one id, two sites", 1);
}

#[inline(never)]
fn merge_b(ok: bool) {
    sometimes!(ok, "merge.shared", "one id, two sites", 2);
}

fn main() {
    merge_a(true);
    merge_a(false);
    merge_b(true);
    merge_b(true);
    reachable!("merge.other", "a second, distinct id");
}

//! One id declared with two kinds is a catalog conflict the engine reports.

use hairspring_sdk::{always, sometimes};

fn main() {
    always!(true, "conflict.id", "declared as always");
    sometimes!(true, "conflict.id", "declared as sometimes");
}

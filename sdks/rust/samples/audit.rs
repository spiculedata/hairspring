//! The positive control for the evaluation audit: a call-site record that is not in the catalog section. Its
//! evaluations are counted by the thread but attributed to no catalogued record, so the report must say so.

use hairspring_sdk::__private::{eval, Kind, Site};

/// Deliberately a plain static: the linker never collects it into the catalog section.
static OUTSIDE: Site = Site::new(
    "audit.outside",
    "not in the section",
    file!(),
    line!(),
    Kind::Always,
);

fn main() {
    for _ in 0..3 {
        eval(&OUTSIDE, true, [0; 4]);
    }
}

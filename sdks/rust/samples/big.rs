//! 64 ids with long messages, so that two processes writing to one output at once have a real chance of interleaving
//! if a line were ever split across `write` calls. Every id is hit three times per process.

use hairspring_sdk::sometimes;

macro_rules! long_msg {
    () => {
        concat!(
            "a deliberately long message so that each catalog line is large and two processes writing at ",
            "once have a real chance of interleaving if a line were ever split across write calls; padding ",
            "padding padding padding padding padding padding padding padding padding padding padding ",
            "padding padding padding padding padding padding padding padding padding padding padding"
        )
    };
}

macro_rules! row {
    ($rnd:expr; $($n:literal),*) => {
        $( sometimes!($rnd % 2 == 0, concat!("big.site.", $n), long_msg!(), $rnd); )*
    };
}

fn main() {
    for rnd in 0..3 {
        row!(rnd; "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7");
        row!(rnd; "b0", "b1", "b2", "b3", "b4", "b5", "b6", "b7");
        row!(rnd; "c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7");
        row!(rnd; "d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7");
        row!(rnd; "e0", "e1", "e2", "e3", "e4", "e5", "e6", "e7");
        row!(rnd; "f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7");
        row!(rnd; "g0", "g1", "g2", "g3", "g4", "g5", "g6", "g7");
        row!(rnd; "h0", "h1", "h2", "h3", "h4", "h5", "h6", "h7");
    }
}

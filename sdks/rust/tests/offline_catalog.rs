//! The offline catalog: each call site also places the JSON of its `catalog` line (without `proc`) in a section of
//! the binary, so a tool can read the catalog without running the program.

mod common;

use common::{events, of_kind, run};

const BASIC: &str = env!("CARGO_BIN_EXE_hs_sample_basic");
const CATALOG_PREFIX: &[u8] = b"{\"v\":1,\"kind\":\"catalog\",";

/// Where `chunk` starts a catalog text, if it holds one. The first text of a section can follow bytes of whatever
/// precedes the section in the file with no NUL between them, so the text is looked for inside the chunk.
fn twin_start(chunk: &[u8]) -> Option<usize> {
    chunk
        .windows(CATALOG_PREFIX.len())
        .position(|w| w == CATALOG_PREFIX)
}

/// The NUL-terminated catalog texts found anywhere in the binary.
fn twins_in(path: &str) -> Vec<String> {
    let bytes = std::fs::read(path).expect("binary is readable");
    let mut twins: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter_map(|chunk| twin_start(chunk).map(|at| &chunk[at..]))
        .map(|text| String::from_utf8(text.to_vec()).expect("twin is utf-8"))
        .collect();
    twins.sort();
    twins.dedup();
    twins
}

#[test]
fn the_binary_carries_a_twin_of_every_live_catalog_line() {
    let live: Vec<String> = {
        let r = run(BASIC, &[], &[]);
        let ev = events(&r.stderr);
        let mut lines: Vec<String> = of_kind(&ev, "catalog")
            .iter()
            .map(|l| l.replace("\"proc\":\"p1\",", ""))
            .collect();
        lines.sort();
        lines
    };
    assert_eq!(live.len(), 7);
    let twins = twins_in(BASIC);
    for line in &live {
        assert!(
            twins.contains(line),
            "no twin for {line}\nfound: {twins:#?}"
        );
    }
}

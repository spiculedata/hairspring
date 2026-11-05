//! The `hairspring` binary turns SQLite's memory statistics off before anything else runs, because the setting only
//! takes effect before SQLite starts and must be made while no other thread uses SQLite.
const MAIN: &str = include_str!("../src/bin/hairspring.rs");

#[test]
fn main_turns_memory_statistics_off_in_its_first_statement() {
    let body = MAIN
        .split_once("fn main() -> std::process::ExitCode {")
        .expect("main is declared")
        .1;
    let first = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("//"))
        .expect("main has a statement");
    assert!(
        first.contains("hairspring_store::disable_sqlite_memory_statistics()"),
        "main's first statement is `{first}`"
    );
    // The result is reported, not dropped.
    assert!(first.starts_with("let statistics_off"), "{first}");
    assert!(body.contains("if !statistics_off"));
}

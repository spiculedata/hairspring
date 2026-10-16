//! Builders for event lines used across the integration tests.
#![allow(dead_code)]

use hairspring_props::{AssertKind, TrialEvents, TrialProps};

pub fn ev(json: &str) -> String {
    format!("HAIRSPRING_EVENT {json}\n")
}

pub fn hello(proc: &str) -> String {
    ev(&format!(
        r#"{{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"{proc}","catalog_hash":"abc","signal_handlers":false}}"#
    ))
}

pub fn catalog(proc: &str, id: &str, kind: AssertKind) -> String {
    ev(&format!(
        r#"{{"v":1,"kind":"catalog","proc":"{proc}","id":"{id}","type":"{}","must_hit":{},"msg":"m {id}","loc":{{"file":"f.c","line":7,"func":"fn"}}}}"#,
        kind.as_str(),
        kind.must_hit()
    ))
}

/// An `assert` line with optional witnesses `(thread, ord)`.
/// A catalog line without message or location, to keep many-id tests under the byte budget.
pub fn catalog_min(proc: &str, id: &str, kind: AssertKind) -> String {
    ev(&format!(
        r#"{{"v":1,"kind":"catalog","proc":"{proc}","id":"{id}","type":"{}","must_hit":{}}}"#,
        kind.as_str(),
        kind.must_hit()
    ))
}

pub fn assert_line(
    proc: &str,
    id: &str,
    trues: u64,
    falses: u64,
    first_false: Option<(&str, u64)>,
    first_true: Option<(&str, u64)>,
) -> String {
    let w = |w: Option<(&str, u64)>| match w {
        Some((t, o)) => format!(r#"{{"thread":"{t}","ord":{o},"vals":[1,2,3,4]}}"#),
        None => "null".to_string(),
    };
    ev(&format!(
        r#"{{"v":1,"kind":"assert","proc":"{proc}","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":{},"first_true":{}}}"#,
        trues + falses,
        w(first_false),
        w(first_true)
    ))
}

pub fn work(proc: &str, ops: u64) -> String {
    ev(&format!(
        r#"{{"v":1,"kind":"work","proc":"{proc}","ops":{ops}}}"#
    ))
}

pub fn end(proc: &str) -> String {
    ev(&format!(
        r#"{{"v":1,"kind":"end","proc":"{proc}","reason":"exit","status":0,"sites":1,"evals":3,"dropped":0,"threads":[{{"thread":"w-0","evals":2,"tape_consumed":10}},{{"thread":"t#1","evals":1,"tape_consumed":4,"key_stable":false}}]}}"#
    ))
}

pub fn fold(output: &str) -> TrialProps {
    TrialEvents::from_output(output.as_bytes()).fold()
}

/// A complete one-process report for a single id.
pub fn single(kind: AssertKind, trues: u64, falses: u64) -> String {
    let first_false = (falses > 0).then_some(("w-0", 5));
    let first_true = (trues > 0).then_some(("w-0", 1));
    [
        hello("p1"),
        catalog("p1", "x.id", kind),
        assert_line("p1", "x.id", trues, falses, first_false, first_true),
        work("p1", 9),
        end("p1"),
    ]
    .concat()
}

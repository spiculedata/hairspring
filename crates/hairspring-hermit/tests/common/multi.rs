//! Reports with several ids or several processes.
#![allow(dead_code)]
use super::props::ev;

const WITNESS: &str = r#"{"thread":"writer-0","ord":675,"vals":[1,2,3,4]}"#;

fn witness(on: bool) -> &'static str {
    if on {
        WITNESS
    } else {
        "null"
    }
}

/// One process's report for several ids: `(id, kind, trues, falses)`.
pub fn report_many(proc: &str, ids: &[(&str, &str, u64, u64)], end: bool) -> String {
    let mut out = ev(&format!(
        r#"{{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"{proc}","catalog_hash":"abc","signal_handlers":false}}"#
    ));
    for (id, kind, _, _) in ids {
        let must_hit = matches!(*kind, "always" | "sometimes" | "reachable");
        out += &ev(&format!(
            r#"{{"v":1,"kind":"catalog","proc":"{proc}","id":"{id}","type":"{kind}","must_hit":{must_hit},"msg":"msg of {id}"}}"#
        ));
    }
    for (id, _, trues, falses) in ids {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"assert","proc":"{proc}","id":"{id}","hits":{},"trues":{trues},"falses":{falses},"first_false":{},"first_true":{}}}"#,
            trues + falses,
            witness(*falses > 0),
            witness(*trues > 0)
        ));
    }
    out += &ev(&format!(
        r#"{{"v":1,"kind":"work","proc":"{proc}","ops":10}}"#
    ));
    if end {
        out += &ev(&format!(
            r#"{{"v":1,"kind":"end","proc":"{proc}","reason":"exit","status":0,"sites":{},"evals":3,"dropped":0,"threads":0}}"#,
            ids.len()
        ));
    }
    out
}

//! Builders for guest reports and rigs shared by the property verdict tests.
#![allow(dead_code)]
use super::events::{rig, Rig};
use hairspring_core::bundle::Bundle;
use hairspring_engine::config::TargetKind;
use hairspring_engine::engine::RunRequest;
use hairspring_engine::report::RunOutcome;

pub fn ev(json: &str) -> String {
    format!("HAIRSPRING_EVENT {json}\n")
}

/// One process's report for one id.
pub fn report(id: &str, kind: &str, trues: u64, falses: u64, end: bool) -> String {
    let hits = trues + falses;
    let must_hit = matches!(kind, "always" | "sometimes" | "reachable");
    let witness = |on: bool| {
        if on {
            r#"{"thread":"writer-0","ord":675,"vals":[1,2,3,4]}"#
        } else {
            "null"
        }
    };
    let mut out = ev(
        r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"abc","signal_handlers":false}"#,
    );
    out += &ev(&format!(
        r#"{{"v":1,"kind":"catalog","proc":"p1","id":"{id}","type":"{kind}","must_hit":{must_hit},"msg":"msg of {id}"}}"#
    ));
    out += &ev(&format!(
        r#"{{"v":1,"kind":"assert","proc":"p1","id":"{id}","hits":{hits},"trues":{trues},"falses":{falses},"first_false":{},"first_true":{}}}"#,
        witness(falses > 0),
        witness(trues > 0)
    ));
    out += &ev(r#"{"v":1,"kind":"work","proc":"p1","ops":10}"#);
    if end {
        out += &ev(
            r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":1,"evals":3,"dropped":0,"threads":0}"#,
        );
    }
    out
}

pub fn emit(text: &str) -> String {
    format!("cat <<'EOF'\n{text}EOF\n")
}

/// A guest script that prints a different report for the listed seeds.
pub fn by_seed(arms: &[(u64, String)], otherwise: &str) -> String {
    let mut s = String::from("case \"$HAIRSPRING_SEED\" in\n");
    for (seed, text) in arms {
        s += &format!("{seed})\n{}\n;;\n", emit(text));
    }
    s += &format!("*)\n{}\n;;\nesac\n", emit(otherwise));
    s
}

pub fn configure(r: &mut Rig, trials: u64, parallelism: u32) {
    r.spec.budget.max_trials = trials;
    if let TargetKind::Process { parallelism: p, .. } = &mut r.spec.kind {
        *p = parallelism;
    }
}

pub fn run(r: Rig) -> RunOutcome {
    let (engine, _) = r.engine();
    engine
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .expect("the run starts")
}

pub fn run_script(name: &str, script: &str, trials: u64) -> RunOutcome {
    let mut r = rig(name, script, false, vec![]);
    configure(&mut r, trials, 1);
    run(r)
}

pub fn bundle_of(o: &RunOutcome) -> Bundle {
    Bundle::decode(&o.failure.as_ref().expect("a failure").bundle).unwrap()
}

//! A bundle records which scheme hashed its output. Output over the capture cap is hashed differently from before, so
//! a bundle from before cannot be compared on such output, and says so instead of reporting "not reproduced".
mod common;

use common::{engine, scratch, Opts, PASSTHROUGH};
use hairspring_core::bundle::Bundle;
use hairspring_engine::engine::{Engine, RunRequest};
use hairspring_engine::report::Step;

const NOT_COMPARED: &str = "hash scheme changed; output not compared";
const LONG: &str =
    "echo 'HAIRSPRING_VIOLATION: boom'; head -c 1500000 /dev/zero | tr '\\0' x; echo; exit 1";
const SHORT: &str = "echo 'HAIRSPRING_VIOLATION: boom'; exit 1";

fn recorded(name: &str, script: &str) -> (Engine, Bundle) {
    let d = scratch(name);
    let e = engine(&d, PASSTHROUGH, script, Opts::default(), &[]);
    let f = e
        .run(&RunRequest::new("svc", 1), &mut |_| true)
        .unwrap()
        .failure
        .unwrap();
    (e, Bundle::decode(&f.bundle).unwrap())
}

fn old_style(bundle: &Bundle, hash: Option<&str>) -> Bundle {
    let mut old = bundle.clone();
    old.params.remove("output_hash_scheme");
    if let Some(h) = hash {
        old.params.insert("output_hash".into(), h.into());
    }
    old
}

fn mentions(steps: &[Step], text: &str) -> bool {
    steps.iter().any(|s| s.label.contains(text))
}

#[test]
fn a_new_bundle_records_its_hash_scheme_and_replays_as_the_same_output_on_long_output() {
    let (e, bundle) = recorded("scheme-new", LONG);
    assert!(bundle.params.contains_key("output_hash_scheme"));
    let r = e.replay(&bundle).unwrap();
    assert!(r.reproduced, "{:?}", r.steps);
    assert!(mentions(&r.steps, "output matches the recording: yes"));
}

#[test]
fn a_new_bundle_whose_long_output_changed_is_not_reproduced() {
    let (e, bundle) = recorded("scheme-changed", LONG);
    let tampered = bundle.with_param("output_hash", "0123456789abcdef");
    let r = e.replay(&tampered).unwrap();
    assert!(!r.reproduced, "{:?}", r.steps);
    assert!(mentions(&r.steps, "output matches the recording: no"));
}

#[test]
fn an_older_bundle_is_not_compared_on_output_that_is_now_cut() {
    let (e, bundle) = recorded("scheme-old-long", LONG);
    let old = old_style(&bundle, Some("0123456789abcdef"));
    let r = e.replay(&old).unwrap();
    assert!(r.reproduced, "{:?}", r.steps);
    assert!(mentions(&r.steps, NOT_COMPARED), "{:?}", r.steps);
    assert!(!mentions(&r.steps, "output matches the recording"));
}

#[test]
fn an_older_bundle_is_still_compared_on_output_that_fits_the_cap() {
    let (e, bundle) = recorded("scheme-old-short", SHORT);
    let same = e.replay(&old_style(&bundle, None)).unwrap();
    assert!(same.reproduced);
    assert!(mentions(&same.steps, "output matches the recording: yes"));
    assert!(!mentions(&same.steps, NOT_COMPARED));

    let differs = e
        .replay(&old_style(&bundle, Some("0123456789abcdef")))
        .unwrap();
    assert!(!differs.reproduced);
    assert!(mentions(&differs.steps, "output matches the recording: no"));
}

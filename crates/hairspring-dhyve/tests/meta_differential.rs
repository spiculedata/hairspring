//! `restored_rip` and `check_module` against a reference decoder (written in Python), on cases whose answers were
//! generated once by running that decoder. A case the reference refuses or cannot decode, the port must not decode.
use hairspring_core::point::encoding::sha256_hex;
use hairspring_dhyve::ckptmeta::{check_module, restored_rip, MetaError};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/meta_cases.json");
const FIXTURE_SHA256: &str = "a534727e84a34392e537faae1345299197f3f45d1f8ce370993ed7d6ca87c8cb";

fn from_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn the_fixture_is_the_one_that_was_generated() {
    assert_eq!(sha256_hex(FIXTURE.as_bytes()), FIXTURE_SHA256);
}

#[test]
fn the_port_agrees_with_the_reference_decoder_on_every_case() {
    let cases: Vec<Value> = serde_json::from_str(FIXTURE).unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let meta = case["meta"].as_str().unwrap();
        let kern = from_hex(case["kern_hex"].as_str().unwrap());
        let rip = restored_rip(meta, &kern);
        let module = check_module(meta);
        if case["parse"] == "error" {
            assert!(
                matches!(
                    rip,
                    Err(MetaError::NotJson | MetaError::NoEntries | MetaError::BadEntry)
                ),
                "{name}: {rip:?}"
            );
            assert!(module.is_err(), "{name}");
            continue;
        }
        match case["rip"].as_u64() {
            Some(expected) => assert_eq!(rip, Ok(expected), "{name}"),
            None => assert!(
                rip.is_err(),
                "{name}: the reference has no pointer but the port gave {rip:?}"
            ),
        }
        match case["vmcx"].as_str().unwrap() {
            "OK" => assert_eq!(module, Ok(()), "{name}"),
            "WRONG-SIZE" => assert_eq!(
                module,
                Err(MetaError::ModuleEntry(Some(
                    case["vmcx_size"].as_u64().unwrap()
                ))),
                "{name}"
            ),
            "MISSING" | "DUPLICATE" => {
                assert_eq!(module, Err(MetaError::ModuleEntry(None)), "{name}")
            }
            other => panic!("{name}: {other}"),
        }
    }
}

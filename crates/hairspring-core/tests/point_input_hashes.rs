//! Per-field hashes of a run's inputs: what the store keeps to tell whether two runs used the same instrument. Only
//! digests leave the draft, never a value.
mod point_common;

use hairspring_core::point::input_hashes::{known_fields, MAX_INPUT_FIELDS};
use hairspring_core::point::{HermitInputs, IdentityDraft, Recorded, TmpPolicy};
use point_common::*;
use std::collections::BTreeSet;

/// SHA-256 of `lock_demo`, from `printf lock_demo | shasum -a 256`, independent of the library.
const LOCK_DEMO_SHA: &str = "34748cc791627f5f2602d51121972837d63eca82dc7f6cf6e3a2afc49fa73b80";

fn hashes_of(draft: &IdentityDraft) -> hairspring_core::point::input_hashes::InputHashes {
    draft.input_hashes()
}

#[test]
fn a_complete_draft_hashes_every_field_the_scheme_knows_and_misses_none() {
    for (draft, scheme) in [
        (sim_draft(), "sim/1"),
        (wasm_draft(), "wasm/1"),
        (hermit_draft(), "hermit/1"),
    ] {
        let hashes = hashes_of(&draft);
        assert_eq!(hashes.scheme, scheme);
        assert!(hashes.missing.is_empty(), "{scheme}: {:?}", hashes.missing);
        let keys: BTreeSet<&str> = hashes.fields.keys().map(String::as_str).collect();
        let known: BTreeSet<&str> = known_fields(scheme).unwrap().iter().copied().collect();
        assert_eq!(
            keys, known,
            "{scheme}: the list of known fields has drifted"
        );
        assert!(known.len() <= MAX_INPUT_FIELDS);
        assert!(!keys.contains("scheme"), "the scheme has its own column");
        for digest in hashes.fields.values() {
            assert_eq!(digest.len(), 64);
            assert!(digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        }
    }
    assert_eq!(known_fields("nope/1"), None);
}

#[test]
fn a_field_hash_is_the_sha256_of_its_rendered_value() {
    let hashes = hashes_of(&sim_draft());
    assert_eq!(hashes.fields["target"], LOCK_DEMO_SHA);
    // The same bytes the identity text carries for that line.
    let hermit = hashes_of(&hermit_draft());
    assert_eq!(
        hermit.fields["target_argv"],
        hairspring_core::point::encoding::sha256_hex(b"2:/usr/bin/queue,--fast")
    );
}

#[test]
fn changing_one_input_changes_only_that_fields_hash() {
    let base = hashes_of(&hermit_draft());
    let mut inputs = hermit_inputs();
    inputs.sched_skid_margin = v(Some(51));
    let changed = hashes_of(&IdentityDraft::Hermit(common("queue"), inputs));
    let differing: Vec<&String> = base
        .fields
        .keys()
        .filter(|k| base.fields[*k] != changed.fields[*k])
        .collect();
    assert_eq!(differing, ["sched_skid_margin"]);
}

#[test]
fn an_input_that_was_not_recorded_is_named_and_has_no_hash() {
    let mut inputs = hermit_inputs();
    inputs.loader_libc_sha = Recorded::Missing;
    inputs.hermit_sha = v("not a digest".to_string());
    let hashes = hashes_of(&IdentityDraft::Hermit(common("queue"), inputs));
    assert_eq!(hashes.missing, ["hermit_sha", "loader_libc_sha"]);
    assert!(!hashes.fields.contains_key("loader_libc_sha"));
    assert!(!hashes.fields.contains_key("hermit_sha"));
    assert_eq!(hashes.fields.len() + hashes.missing.len(), 24);
}

#[test]
fn an_invalid_input_is_reported_as_not_recorded_and_not_hashed() {
    let mut inputs = hermit_inputs();
    inputs.cwd = v("relative/path".to_string());
    inputs.guest_env = v(vec![("A=B".to_string(), "x".to_string())]);
    let hashes = hashes_of(&IdentityDraft::Hermit(common("queue"), inputs));
    assert_eq!(hashes.missing, ["cwd", "guest_env"]);
    assert!(!hashes.fields.contains_key("cwd"));
    assert!(!hashes.fields.contains_key("guest_env"));
}

#[test]
fn no_plaintext_value_reaches_the_hashes() {
    let secret = "TOPSECRET-value-1234";
    let inputs = HermitInputs {
        guest_env: v(vec![("API_TOKEN".to_string(), secret.to_string())]),
        cwd: v(format!("/home/{secret}")),
        tmp_policy: v(TmpPolicy::Path(format!("/tmp/{secret}"))),
        target_argv: v(vec!["/bin/x".to_string(), secret.to_string()]),
        ..hermit_inputs()
    };
    let hashes = hashes_of(&IdentityDraft::Hermit(common("queue"), inputs));
    let shown = format!("{hashes:?}");
    assert!(!shown.contains(secret), "{shown}");
    assert!(!shown.contains("API_TOKEN"), "{shown}");
    assert!(hashes.missing.is_empty());
}

#[test]
fn the_environment_hash_follows_a_changed_value() {
    let base = hashes_of(&hermit_draft());
    let mut inputs = hermit_inputs();
    inputs.guest_env = v(vec![
        ("HOME".to_string(), "/root".to_string()),
        ("LANG".to_string(), "C.UTF-8".to_string()),
    ]);
    let changed = hashes_of(&IdentityDraft::Hermit(common("queue"), inputs));
    assert_ne!(base.fields["guest_env"], changed.fields["guest_env"]);
}

mod point_common;

use hairspring_core::point::hash::HashError;
use hairspring_core::point::{
    point_hash, BackendKind, CommonInputs, Cut, HermitInputs, IdentityDraft, Prefix, WasmInputs,
};
use point_common::*;
use std::collections::HashSet;

const SIM_PATH_HASH: &str = "107baabb4434970617d69291598e3ee50e883b14a3d57bc77894fbd968e98d65";
const SIM_EMPTY_PATH_HASH: &str =
    "19c2edb585681b26398b9344b9485e03c2d5ee9fc8753b8309576e9460adc029";
const WASM_CHOICES_HASH: &str = "50107c5e51ec99bf4d8a4533cf2af5e16a1bed9d59dda785c0972469bec351ce";
const HERMIT_END_HASH: &str = "80c6a223c1ff0bee23090342aa87d33ab894540fb49217058c26fa482c6d2367";

fn hash(draft: &IdentityDraft, seed: u64, prefix: Prefix<'_>) -> String {
    point_hash(&draft.finish().unwrap(), seed, &prefix).unwrap()
}

#[test]
fn golden_point_hashes() {
    let sim = sim_draft();
    assert_eq!(hash(&sim, 42, Prefix::Sim(&[1, 255])), SIM_PATH_HASH);
    assert_eq!(hash(&sim, 42, Prefix::Sim(&[])), SIM_EMPTY_PATH_HASH);
    let wasm = wasm_draft();
    assert_eq!(
        hash(&wasm, 7, Prefix::Wasm(&[3, 0, u64::MAX])),
        WASM_CHOICES_HASH
    );
    assert_eq!(
        hash(&hermit_draft(), 99, Prefix::HermitEnd),
        HERMIT_END_HASH
    );
}

#[test]
fn the_hash_is_lowercase_hex_of_a_sha256() {
    let h = hash(&sim_draft(), 1, Prefix::Sim(&[1]));
    assert_eq!(h.len(), 64);
    assert!(h
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
}

#[test]
fn changing_the_seed_or_the_prefix_changes_the_hash() {
    let sim = sim_draft();
    let base = hash(&sim, 42, Prefix::Sim(&[1, 255]));
    let variants = [
        hash(&sim, 43, Prefix::Sim(&[1, 255])),
        hash(&sim, 42, Prefix::Sim(&[1])),
        hash(&sim, 42, Prefix::Sim(&[1, 255, 0])),
        hash(&sim, 42, Prefix::Sim(&[255, 1])),
        hash(&sim, 42, Prefix::Sim(&[1, 254])),
        hash(&sim, 42, Prefix::Sim(&[2, 255])),
        hash(&sim, 42, Prefix::Sim(&[0x10, 255])),
        hash(&sim, 42, Prefix::Sim(&[1, 0x1ff])),
    ];
    let mut seen = HashSet::from([base]);
    for v in variants {
        assert!(seen.insert(v), "a changed seed or prefix collided");
    }
}

#[test]
fn choice_values_are_unambiguous() {
    let wasm = wasm_draft();
    let split = hash(&wasm, 7, Prefix::Wasm(&[1, 23]));
    let joined = hash(&wasm, 7, Prefix::Wasm(&[12, 3]));
    let single = hash(&wasm, 7, Prefix::Wasm(&[123]));
    assert_ne!(split, joined);
    assert_ne!(split, single);
    assert_ne!(joined, single);
}

#[test]
fn changing_any_one_identity_input_changes_the_point_hash() {
    let seed = 5;
    let sim_base = hash(&sim_draft(), seed, Prefix::Sim(&[1]));
    let other_sim = IdentityDraft::Sim(CommonInputs {
        target: v("other".to_string()),
        ..common("x")
    });
    assert_ne!(sim_base, hash(&other_sim, seed, Prefix::Sim(&[1])));

    let wasm_base = hash(&wasm_draft(), seed, Prefix::Wasm(&[1]));
    let fuel = WasmInputs {
        fuel: v(7),
        ..wasm_inputs()
    };
    let other_wasm = IdentityDraft::Wasm(common("counter"), fuel);
    assert_ne!(wasm_base, hash(&other_wasm, seed, Prefix::Wasm(&[1])));

    let hermit_base = hash(&hermit_draft(), seed, Prefix::HermitEnd);
    let chaos = HermitInputs {
        sched_chaos: v(false),
        ..hermit_inputs()
    };
    let other_hermit = IdentityDraft::Hermit(common("queue"), chaos);
    assert_ne!(hermit_base, hash(&other_hermit, seed, Prefix::HermitEnd));
}

#[test]
fn the_same_inputs_give_one_hash_whichever_run_they_came_from() {
    // The hash takes no run id: two runs built from equal inputs share it.
    let first = hash(&hermit_draft(), 3, Prefix::HermitEnd);
    let second = hash(&hermit_draft(), 3, Prefix::HermitEnd);
    assert_eq!(first, second);
}

#[test]
fn a_prefix_of_the_wrong_backend_is_refused() {
    let sim = sim_draft().finish().unwrap();
    let wasm = wasm_draft().finish().unwrap();
    let hermit = hermit_draft().finish().unwrap();
    let mismatch = Err(HashError::BackendMismatch);
    assert_eq!(point_hash(&sim, 1, &Prefix::Wasm(&[])), mismatch);
    assert_eq!(point_hash(&sim, 1, &Prefix::HermitEnd), mismatch);
    assert_eq!(point_hash(&wasm, 1, &Prefix::Sim(&[])), mismatch);
    assert_eq!(point_hash(&hermit, 1, &Prefix::Sim(&[])), mismatch);
}

#[test]
fn for_cut_keeps_the_first_k_entries() {
    let trace = [10, 20, 30];
    let sim = |cut| Prefix::for_cut(BackendKind::Sim, cut, &trace);
    assert_eq!(sim(Cut::End), Ok(Prefix::Sim(&[10, 20, 30])));
    assert_eq!(sim(Cut::Step(2)), Ok(Prefix::Sim(&[10, 20])));
    assert_eq!(sim(Cut::Step(0)), Ok(Prefix::Sim(&[])));
    assert_eq!(sim(Cut::Step(3)), Ok(Prefix::Sim(&[10, 20, 30])));
    assert_eq!(
        sim(Cut::Step(4)),
        Err(HashError::CutBeyondTrace {
            cut: 4,
            available: 3
        })
    );
    assert!(matches!(
        sim(Cut::Step(u64::MAX)),
        Err(HashError::CutBeyondTrace { .. })
    ));
    let wasm = |cut| Prefix::for_cut(BackendKind::Wasm, cut, &trace);
    assert_eq!(wasm(Cut::Choice(1)), Ok(Prefix::Wasm(&[10])));
    assert_eq!(wasm(Cut::End), Ok(Prefix::Wasm(&[10, 20, 30])));
}

#[test]
fn for_cut_refuses_cut_kinds_the_backend_cannot_have() {
    let trace = [1];
    let bad = Err(HashError::CutNotValidForBackend);
    assert_eq!(
        Prefix::for_cut(BackendKind::Sim, Cut::Choice(0), &trace),
        bad
    );
    assert_eq!(
        Prefix::for_cut(BackendKind::Wasm, Cut::Step(0), &trace),
        bad
    );
    assert_eq!(
        Prefix::for_cut(BackendKind::Hermit, Cut::Step(0), &trace),
        bad
    );
    assert_eq!(
        Prefix::for_cut(BackendKind::Hermit, Cut::Choice(0), &trace),
        bad
    );
    assert_eq!(
        Prefix::for_cut(BackendKind::Hermit, Cut::End, &trace),
        Ok(Prefix::HermitEnd)
    );
}

#[test]
fn a_cut_at_step_zero_equals_an_empty_path() {
    let sim = sim_draft();
    let cut = Prefix::for_cut(BackendKind::Sim, Cut::Step(0), &[9, 9]).unwrap();
    assert_eq!(hash(&sim, 42, cut), SIM_EMPTY_PATH_HASH);
}

#[test]
fn hash_errors_have_readable_messages() {
    assert_eq!(
        HashError::CutBeyondTrace {
            cut: 4,
            available: 3
        }
        .to_string(),
        "cut 4 is beyond the 3 recorded entries"
    );
}

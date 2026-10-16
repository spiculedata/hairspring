mod point_common;

use hairspring_core::point::hash::HashError;
use hairspring_core::point::{
    point_hash, seed_from_stored, BackendKind, CommonInputs, Cut, HermitInputs, IdentityDraft,
    Prefix, WasmInputs,
};
use point_common::*;
use std::collections::HashSet;

const SIM_END_HASH: &str = "57dea784b44b3feb66998d72fe5f527b160500291aaf0094c95a1ac066e772b7";
const SIM_S2_HASH: &str = "2ebbd5902badba426e4c7f9f4a6b3da9adce871d14f443fcdd08f0ece4ed994b";
const SIM_S0_HASH: &str = "eb03ccee64f634be705a9da321bd8b406aa80ae9ee59ab8846d5eae988d12966";
const WASM_END_HASH: &str = "61650b38c593144475adb81771a3f39ad078d7e6049274de6771e03cc86bd595";
const WASM_C3_HASH: &str = "e1b8ec84c52f77ddbcdd391f14acd4a6cdee1a04c246a56913dfcfaa91a509ec";
const HERMIT_END_HASH: &str = "72924af9a328acea940737dcd632d1d488572d5e44cb3636b5a5df979e778cf2";
const SIM_TOP_BIT_SEED_HASH: &str =
    "74003ee27ac7ec1742f68e25229c397f4e92ff47598bb0172d1b3294647377a9";
const SIM_MAX_SEED_HASH: &str = "683c437e71206022a3e88a4bfd994717dc8e02b265df389269f19f112eaae12f";

fn hash(draft: &IdentityDraft, seed: u64, cut: Cut, trace: &[u64]) -> String {
    point_hash(&draft.finish().unwrap(), seed, cut, trace).unwrap()
}

#[test]
fn golden_point_hashes() {
    let sim = sim_draft();
    assert_eq!(hash(&sim, 42, Cut::End, &[1, 255]), SIM_END_HASH);
    assert_eq!(hash(&sim, 42, Cut::Step(2), &[1, 255]), SIM_S2_HASH);
    assert_eq!(hash(&sim, 42, Cut::Step(0), &[1, 255]), SIM_S0_HASH);
    let wasm = wasm_draft();
    let choices = [3, 0, u64::MAX];
    assert_eq!(hash(&wasm, 7, Cut::End, &choices), WASM_END_HASH);
    assert_eq!(hash(&wasm, 7, Cut::Choice(3), &choices), WASM_C3_HASH);
    assert_eq!(hash(&hermit_draft(), 99, Cut::End, &[]), HERMIT_END_HASH);
}

#[test]
fn end_differs_from_a_cut_at_the_same_length() {
    let sim = sim_draft();
    let path = [1, 255];
    let end = hash(&sim, 42, Cut::End, &path);
    let step = hash(&sim, 42, Cut::Step(2), &path);
    assert_ne!(end, step);
    let wasm = wasm_draft();
    let choices = [3, 0, 9];
    let end = hash(&wasm, 7, Cut::End, &choices);
    let choice = hash(&wasm, 7, Cut::Choice(3), &choices);
    assert_ne!(end, choice);
}

#[test]
fn the_seed_is_the_decimal_u64_of_the_stored_bit_pattern() {
    assert_eq!(seed_from_stored(0), 0);
    assert_eq!(seed_from_stored(42), 42);
    assert_eq!(seed_from_stored(-1), u64::MAX);
    assert_eq!(seed_from_stored(i64::MIN), 1 << 63);
    assert_eq!(seed_from_stored(i64::MIN + 1), (1 << 63) + 1);
    let sim = sim_draft();
    let stored = i64::MIN + 1;
    let top_bit = hash(&sim, seed_from_stored(stored), Cut::End, &[1, 255]);
    assert_eq!(top_bit, SIM_TOP_BIT_SEED_HASH);
    let max = hash(&sim, seed_from_stored(-1), Cut::End, &[1, 255]);
    assert_eq!(max, SIM_MAX_SEED_HASH);
}

#[test]
fn the_hash_is_lowercase_hex_of_a_sha256() {
    let h = hash(&sim_draft(), 1, Cut::End, &[1]);
    assert_eq!(h.len(), 64);
    assert!(h
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
}

#[test]
fn changing_the_seed_or_the_prefix_changes_the_hash() {
    let sim = sim_draft();
    let base = hash(&sim, 42, Cut::End, &[1, 255]);
    let variants = [
        hash(&sim, 43, Cut::End, &[1, 255]),
        hash(&sim, 42, Cut::End, &[1]),
        hash(&sim, 42, Cut::End, &[1, 255, 0]),
        hash(&sim, 42, Cut::End, &[255, 1]),
        hash(&sim, 42, Cut::End, &[1, 254]),
        hash(&sim, 42, Cut::End, &[2, 255]),
        hash(&sim, 42, Cut::End, &[0x10, 255]),
        hash(&sim, 42, Cut::End, &[1, 0x1ff]),
        hash(&sim, 42, Cut::Step(1), &[1, 255]),
    ];
    let mut seen = HashSet::from([base]);
    for v in variants {
        assert!(seen.insert(v), "a changed seed or prefix collided");
    }
}

#[test]
fn choice_values_are_unambiguous() {
    let wasm = wasm_draft();
    let split = hash(&wasm, 7, Cut::End, &[1, 23]);
    let joined = hash(&wasm, 7, Cut::End, &[12, 3]);
    let single = hash(&wasm, 7, Cut::End, &[123]);
    assert_ne!(split, joined);
    assert_ne!(split, single);
    assert_ne!(joined, single);
}

#[test]
fn changing_any_one_identity_input_changes_the_point_hash() {
    let seed = 5;
    let sim_base = hash(&sim_draft(), seed, Cut::End, &[1]);
    let other_sim = IdentityDraft::Sim(CommonInputs {
        target: v("other".to_string()),
        ..common("x")
    });
    assert_ne!(sim_base, hash(&other_sim, seed, Cut::End, &[1]));

    let wasm_base = hash(&wasm_draft(), seed, Cut::End, &[1]);
    let fuel = WasmInputs {
        fuel: v(7),
        ..wasm_inputs()
    };
    let other_wasm = IdentityDraft::Wasm(common("counter"), fuel);
    assert_ne!(wasm_base, hash(&other_wasm, seed, Cut::End, &[1]));

    let hermit_base = hash(&hermit_draft(), seed, Cut::End, &[]);
    let chaos = HermitInputs {
        sched_chaos: v(false),
        ..hermit_inputs()
    };
    let other_hermit = IdentityDraft::Hermit(common("queue"), chaos);
    assert_ne!(hermit_base, hash(&other_hermit, seed, Cut::End, &[]));
}

#[test]
fn the_same_inputs_give_one_hash_whichever_run_they_came_from() {
    // The hash takes no run id: two runs built from equal inputs share it.
    let first = hash(&hermit_draft(), 3, Cut::End, &[]);
    let second = hash(&hermit_draft(), 3, Cut::End, &[]);
    assert_eq!(first, second);
}

#[test]
fn a_cut_the_backend_cannot_have_is_refused() {
    let sim = sim_draft().finish().unwrap();
    let wasm = wasm_draft().finish().unwrap();
    let hermit = hermit_draft().finish().unwrap();
    let bad = Err(HashError::CutNotValidForBackend);
    assert_eq!(point_hash(&sim, 1, Cut::Choice(0), &[]), bad);
    assert_eq!(point_hash(&wasm, 1, Cut::Step(0), &[]), bad);
    assert_eq!(point_hash(&hermit, 1, Cut::Step(0), &[]), bad);
    assert_eq!(point_hash(&hermit, 1, Cut::Choice(0), &[]), bad);
}

#[test]
fn a_cut_beyond_the_trace_is_refused() {
    let sim = sim_draft().finish().unwrap();
    assert_eq!(
        point_hash(&sim, 1, Cut::Step(3), &[1, 2]),
        Err(HashError::CutBeyondTrace {
            cut: 3,
            available: 2
        })
    );
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

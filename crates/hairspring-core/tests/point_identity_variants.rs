//! Changing exactly one identity input changes the identity hash.
mod point_common;

use hairspring_core::point::{CommonInputs, HermitInputs, IdentityDraft, TmpPolicy, WasmInputs};
use point_common::*;
use std::collections::HashSet;

fn assert_each_change_is_visible(base: &IdentityDraft, variants: Vec<(&str, IdentityDraft)>) {
    let base_hash = base.finish().unwrap().hash();
    let mut seen = HashSet::from([base_hash.clone()]);
    for (name, draft) in variants {
        let hash = draft
            .finish()
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .hash();
        assert_ne!(hash, base_hash, "changing `{name}` did not change the hash");
        assert!(
            seen.insert(hash),
            "changing `{name}` collided with another variant"
        );
    }
}

#[test]
fn every_common_input_matters() {
    let base = sim_draft();
    let variants = vec![
        (
            "hairspring_sha",
            IdentityDraft::Sim(CommonInputs {
                hairspring_sha: v(sha('9')),
                ..common("lock_demo")
            }),
        ),
        ("target", IdentityDraft::Sim(common("lock_demo2"))),
    ];
    assert_each_change_is_visible(&base, variants);
}

#[test]
fn every_wasm_input_matters() {
    let b = wasm_inputs();
    let wasm = |w: WasmInputs| IdentityDraft::Wasm(common("counter"), w);
    let variants = vec![
        (
            "module_sha",
            wasm(WasmInputs {
                module_sha: v(sha('9')),
                ..b.clone()
            }),
        ),
        (
            "fuel",
            wasm(WasmInputs {
                fuel: v(1_000_001),
                ..b.clone()
            }),
        ),
        (
            "max_memory_bytes",
            wasm(WasmInputs {
                max_memory_bytes: v(16_777_217),
                ..b.clone()
            }),
        ),
        (
            "vm_semantics",
            wasm(WasmInputs {
                vm_semantics: v(2),
                ..b.clone()
            }),
        ),
        (
            "hairspring_sha",
            IdentityDraft::Wasm(
                CommonInputs {
                    hairspring_sha: v(sha('9')),
                    ..common("counter")
                },
                b.clone(),
            ),
        ),
        ("target", IdentityDraft::Wasm(common("counter2"), b.clone())),
    ];
    assert_each_change_is_visible(&wasm_draft(), variants);
}

#[test]
fn every_hermit_input_matters() {
    let b = hermit_inputs();
    let h = |i: HermitInputs| IdentityDraft::Hermit(common("queue"), i);
    let variants = vec![
        (
            "hermit_sha",
            h(HermitInputs {
                hermit_sha: v(sha('9')),
                ..b.clone()
            }),
        ),
        (
            "target_argv",
            h(HermitInputs {
                target_argv: v(strings(&["/usr/bin/queue"])),
                ..b.clone()
            }),
        ),
        (
            "target_exe_sha",
            h(HermitInputs {
                target_exe_sha: v(sha('9')),
                ..b.clone()
            }),
        ),
        (
            "epoch",
            h(HermitInputs {
                epoch: v("2026-01-02T00:00:00Z".to_string()),
                ..b.clone()
            }),
        ),
        (
            "sched_heuristic",
            h(HermitInputs {
                sched_heuristic: v(None),
                ..b.clone()
            }),
        ),
        (
            "sched_sticky_permille",
            h(HermitInputs {
                sched_sticky_permille: v(Some(501)),
                ..b.clone()
            }),
        ),
        (
            "sched_chaos",
            h(HermitInputs {
                sched_chaos: v(false),
                ..b.clone()
            }),
        ),
        (
            "sched_max_timeslice_ns",
            h(HermitInputs {
                sched_max_timeslice_ns: v(Some(1)),
                ..b.clone()
            }),
        ),
        (
            "sched_skid_margin",
            h(HermitInputs {
                sched_skid_margin: v(Some(51)),
                ..b.clone()
            }),
        ),
        (
            "sched_rbc_overshoot",
            h(HermitInputs {
                sched_rbc_overshoot: v(false),
                ..b.clone()
            }),
        ),
        (
            "hermit_args",
            h(HermitInputs {
                hermit_args: v(strings(&[])),
                ..b.clone()
            }),
        ),
        (
            "guest_env",
            h(HermitInputs {
                guest_env: v(vec![("HOME".to_string(), "/tmp".to_string())]),
                ..b.clone()
            }),
        ),
        (
            "cwd",
            h(HermitInputs {
                cwd: v("/work2".to_string()),
                ..b.clone()
            }),
        ),
        (
            "tmp_policy",
            h(HermitInputs {
                tmp_policy: v(TmpPolicy::Path("/scratch".to_string())),
                ..b.clone()
            }),
        ),
        (
            "events_transport",
            h(HermitInputs {
                events_transport: v("stream".to_string()),
                ..b.clone()
            }),
        ),
        (
            "loader_ld_so_cache_sha",
            h(HermitInputs {
                loader_ld_so_cache_sha: v(sha('9')),
                ..b.clone()
            }),
        ),
        (
            "loader_libc_sha",
            h(HermitInputs {
                loader_libc_sha: v(sha('9')),
                ..b.clone()
            }),
        ),
        (
            "loader_ld_linux_sha",
            h(HermitInputs {
                loader_ld_linux_sha: v(sha('9')),
                ..b.clone()
            }),
        ),
        (
            "hang_is_violation",
            h(HermitInputs {
                hang_is_violation: v(true),
                ..b.clone()
            }),
        ),
        (
            "trial_timeout_secs",
            h(HermitInputs {
                trial_timeout_secs: v(Some(61)),
                ..b.clone()
            }),
        ),
        (
            "trial_timeout_secs unset",
            h(HermitInputs {
                trial_timeout_secs: v(None),
                ..b.clone()
            }),
        ),
        (
            "parallelism",
            h(HermitInputs {
                parallelism: v(5),
                ..b.clone()
            }),
        ),
        (
            "hairspring_sha",
            IdentityDraft::Hermit(
                CommonInputs {
                    hairspring_sha: v(sha('9')),
                    ..common("queue")
                },
                b.clone(),
            ),
        ),
        ("target", IdentityDraft::Hermit(common("queue2"), b.clone())),
    ];
    assert_each_change_is_visible(&hermit_draft(), variants);
}

#[test]
fn an_unset_flag_and_a_zero_value_differ() {
    let b = hermit_inputs();
    let unset = IdentityDraft::Hermit(
        common("queue"),
        HermitInputs {
            sched_skid_margin: v(None),
            ..b.clone()
        },
    );
    let zero = IdentityDraft::Hermit(
        common("queue"),
        HermitInputs {
            sched_skid_margin: v(Some(0)),
            ..b
        },
    );
    assert_ne!(
        unset.finish().unwrap().hash(),
        zero.finish().unwrap().hash()
    );
}

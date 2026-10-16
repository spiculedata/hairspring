//! Shared fixtures for the point tests. Each test binary uses a different subset.
#![allow(dead_code)]
use hairspring_core::point::{
    CommonInputs, HermitInputs, IdentityDraft, Recorded, TmpPolicy, WasmInputs,
};

pub fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

pub fn v<T>(value: T) -> Recorded<T> {
    Recorded::Value(value)
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn common(target: &str) -> CommonInputs {
    CommonInputs {
        hairspring_sha: v(sha('a')),
        target: v(target.to_string()),
    }
}

pub fn wasm_inputs() -> WasmInputs {
    WasmInputs {
        module_sha: v(sha('b')),
        fuel: v(1_000_000),
        max_memory_bytes: v(16_777_216),
        vm_semantics: v(1),
    }
}

pub fn hermit_inputs() -> HermitInputs {
    HermitInputs {
        hermit_sha: v(sha('c')),
        target_argv: v(strings(&["/usr/bin/queue", "--fast"])),
        target_exe_sha: v(sha('d')),
        epoch: v("2026-01-01T00:00:00Z".to_string()),
        sched_heuristic: v(Some("sticky-random".to_string())),
        sched_sticky_permille: v(Some(500)),
        sched_chaos: v(true),
        sched_max_timeslice_ns: v(None),
        sched_skid_margin: v(Some(50)),
        sched_rbc_overshoot: v(true),
        hermit_args: v(strings(&["--verbose"])),
        guest_env: v(vec![
            ("HOME".to_string(), "/root".to_string()),
            ("LANG".to_string(), "C".to_string()),
        ]),
        cwd: v("/work".to_string()),
        tmp_policy: v(TmpPolicy::None),
        events_transport: v("file".to_string()),
        loader_ld_so_cache_sha: v(sha('e')),
        loader_libc_sha: v(sha('f')),
        loader_ld_linux_sha: v(sha('1')),
        hang_is_violation: v(false),
        trial_timeout_secs: v(Some(60)),
        parallelism: v(4),
    }
}

pub fn sim_draft() -> IdentityDraft {
    IdentityDraft::Sim(common("lock_demo"))
}

pub fn wasm_draft() -> IdentityDraft {
    IdentityDraft::Wasm(common("counter"), wasm_inputs())
}

pub fn hermit_draft() -> IdentityDraft {
    IdentityDraft::Hermit(common("queue"), hermit_inputs())
}

pub const SIM_TEXT: &str = "scheme=sim/1\n\
hairspring_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
target=lock_demo\n\
backend=sim\n";

pub const WASM_TEXT: &str = "scheme=wasm/1\n\
hairspring_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
target=counter\n\
backend=wasm\n\
module_sha=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n\
fuel=1000000\n\
max_memory_bytes=16777216\n\
vm_semantics=1\n";

pub const HERMIT_TEXT: &str = "scheme=hermit/1\n\
hairspring_sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
target=queue\n\
backend=hermit\n\
hermit_sha=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\n\
target_argv=2:/usr/bin/queue,--fast\n\
target_exe_sha=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\n\
epoch=2026-01-01T00:00:00Z\n\
sched_heuristic=some:sticky-random\n\
sched_sticky_permille=some:500\n\
sched_chaos=true\n\
sched_max_timeslice_ns=none\n\
sched_skid_margin=some:50\n\
sched_rbc_overshoot=true\n\
hermit_args=1:--verbose\n\
guest_env=2:HOME=/root,LANG=C\n\
cwd=/work\n\
tmp_policy=none\n\
events_transport=file\n\
loader_ld_so_cache_sha=eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\n\
loader_libc_sha=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n\
loader_ld_linux_sha=1111111111111111111111111111111111111111111111111111111111111111\n\
hang_is_violation=false\n\
trial_timeout_secs=some:60\n\
parallelism=4\n";

/// SHA-256 of the texts above, computed with `shasum -a 256` independently of the library.
pub const SIM_IDENTITY_HASH: &str =
    "bb8835394934fdd83b131f19a474f132b3e8e7a2bed534cdde804379fefb0de2";
pub const WASM_IDENTITY_HASH: &str =
    "b7154302c6fb9e7de2d74941cda7666958f9c2110b58be6d5766816a707b73d6";
pub const HERMIT_IDENTITY_HASH: &str =
    "910de9cd76f719c837da53a172ebe2bbdf5e613e22720f0b7c5fa4a49e065f8c";

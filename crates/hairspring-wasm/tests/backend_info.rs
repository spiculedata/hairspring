//! What the WebAssembly backend says about itself.
use hairspring_engine::backend::Backend;
use hairspring_wasm::{WasmBackend, WasmSettings};

#[test]
fn wasm_states_what_it_has_and_nothing_more() {
    let info = WasmBackend::new(WasmSettings::default()).info();
    assert_eq!(info.id, "wasm");
    assert_eq!(info.display_name, "WebAssembly");
    let caps = info.capabilities;
    assert!(caps.replay && caps.shrink);
    assert!(!caps.confirm && !caps.strict_verify && !caps.snapshot_branch);
    assert!(!caps.fault_injection && !caps.multi_process);
    assert_eq!(info.limits.max_parallelism, None);
}

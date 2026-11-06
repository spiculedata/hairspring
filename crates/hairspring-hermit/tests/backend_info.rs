//! What the Hermit backend says about itself, without starting Hermit.
use hairspring_engine::backend::Backend;
use hairspring_hermit::{HermitBackend, HermitSettings};

#[test]
fn hermit_states_what_it_has_and_nothing_more() {
    let info = HermitBackend::new(HermitSettings::default()).info();
    assert_eq!(info.id, "hermit");
    assert_eq!(info.display_name, "Hermit");
    let caps = info.capabilities;
    assert!(caps.replay && caps.confirm && caps.strict_verify && caps.multi_process);
    assert!(!caps.shrink && !caps.snapshot_branch && !caps.fault_injection);
    assert_eq!(info.limits.max_parallelism, Some(64));
    assert_eq!(info.limits.max_trial_seconds, None);
}

#[test]
fn listing_hermit_does_not_name_a_binary_it_has_not_read() {
    let info = HermitBackend::new(HermitSettings::default()).info();
    assert_eq!(info.version.binary_version, None);
    assert_eq!(info.version.binary_sha256, None);
    assert!(!info.version.crate_version.is_empty());
}

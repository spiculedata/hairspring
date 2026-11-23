//! The root commands, byte for byte against the argument lists a reference harness (written in Python) builds for one
//! stamp. The fixture was generated once from that harness; its hash is pinned here so it cannot drift unseen. The one
//! deviation from the reference is that `sudo` is named by its absolute path.
use hairspring_core::point::encoding::sha256_hex;
use hairspring_dhyve::commands::{Approved, Commands, Refusal};
use hairspring_dhyve::host::HostConfig;
use hairspring_dhyve::vmname::{Stamp, VmName};
use serde_json::Value;
use std::path::PathBuf;

const FIXTURE: &str = include_str!("fixtures/reference_shapes.json");
const FIXTURE_SHA256: &str = "100631b39d3a15a6c5e18e3e21c001e3ef42b7fc3100a7a6a05b857a267e98d1";

fn list(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

struct Reference {
    fixture: Value,
    commands: Commands,
    stamp: Stamp,
}

fn reference() -> Reference {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let session_dir = fixture["session_dir"].as_str().unwrap().to_string();
    let stamp = Stamp::parse(fixture["stamp"].as_str().unwrap()).unwrap();
    let mut host = HostConfig::new(
        PathBuf::from(format!("{session_dir}/bin/bhyve")),
        PathBuf::from(format!("{session_dir}/bin/bhyvectl")),
        PathBuf::from(format!("{session_dir}/lib")),
        PathBuf::from(&session_dir),
        PathBuf::from(format!("{session_dir}/img/vmlinuz")),
        PathBuf::from(format!("{session_dir}/img/initrd")),
    );
    host.session_stamp = Some(stamp.clone());
    Reference {
        commands: Commands::new(&host, &stamp),
        fixture,
        stamp,
    }
}

#[test]
fn the_fixture_is_the_one_that_was_generated() {
    assert_eq!(sha256_hex(FIXTURE.as_bytes()), FIXTURE_SHA256);
}

#[test]
fn r4_r5_and_r6_equal_the_reference_argument_lists_byte_for_byte() {
    let reference = reference();
    let name = VmName::new(&reference.stamp, 2).unwrap();
    assert_eq!(
        reference
            .commands
            .bhyve(&list(&reference.fixture["start_args"])),
        list(&reference.fixture["r4"])
    );
    assert_eq!(
        reference.commands.destroy(&name),
        list(&reference.fixture["r5"])
    );
    let source = VmName::new(&reference.stamp, 1).unwrap();
    assert_eq!(
        reference.commands.checkpoint(&source),
        list(&reference.fixture["r6"])
    );
}

#[test]
fn the_reference_accepted_its_own_r4_and_r5_and_so_does_the_allow_list() {
    let reference = reference();
    assert_eq!(reference.fixture["reference_accepted"]["R4"], "R4");
    assert_eq!(reference.fixture["reference_accepted"]["R5"], "R5");
    assert_eq!(
        reference.commands.check(&list(&reference.fixture["r4"])),
        Ok(Approved::Bhyve)
    );
    assert_eq!(
        reference.commands.check(&list(&reference.fixture["r5"])),
        Ok(Approved::Destroy)
    );
    assert_eq!(
        reference.commands.check(&list(&reference.fixture["r6"])),
        Ok(Approved::Checkpoint)
    );
}

#[test]
fn what_the_reference_refused_the_allow_list_refuses() {
    let reference = reference();
    assert!(reference.fixture["reference_refused"]["R5_other_stamp"].is_null());
    assert!(reference.fixture["reference_refused"]["R4_core_0"].is_null());
    let other = Stamp::parse(reference.fixture["other_stamp"].as_str().unwrap()).unwrap();
    let foreign = VmName::new(&other, 2).unwrap();
    assert_eq!(
        reference
            .commands
            .check(&reference.commands.destroy(&foreign)),
        Err(Refusal::NotAllowed)
    );
    assert_eq!(
        reference
            .commands
            .check(&reference.commands.checkpoint(&foreign)),
        Err(Refusal::NotAllowed)
    );
    let mut core_zero = list(&reference.fixture["r4"]);
    core_zero[7] = "0".to_string();
    assert_eq!(
        reference.commands.check(&core_zero),
        Err(Refusal::NotAllowed)
    );
    let mut elsewhere = list(&reference.fixture["r6"]);
    elsewhere[6] = "--checkpoint=/var/tmp/elsewhere/boot".to_string();
    assert_eq!(
        reference.commands.check(&elsewhere),
        Err(Refusal::NotAllowed)
    );
}

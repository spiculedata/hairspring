//! The allow-list for a VM start checks the value of every option, not only its name. A start that is given a network
//! backend or a disk (either of which can make a library load a kernel module as root, or hand the guest a host device), a
//! path outside the run's own directory, or any value the builder does not produce, is refused.
use hairspring_dhyve::checkpoint::{CheckpointRecord, RestoreKeys};
use hairspring_dhyve::commands::{Approved, Commands, Refusal};
use hairspring_dhyve::fwcfg::Inputs;
use hairspring_dhyve::host::HostConfig;
use hairspring_dhyve::layout::StateLayout;
use hairspring_dhyve::vmargs::{prepare_args, restore_args};
use hairspring_dhyve::vmname::{Stamp, VmName};
use std::path::PathBuf;

struct Rig {
    host: HostConfig,
    commands: Commands,
    name: VmName,
    prepare: Vec<String>,
    restore: Vec<String>,
}

fn rig(tag: &str) -> Rig {
    let state = PathBuf::from(format!("/tmp/hsal-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state);
    let host = HostConfig::new(
        "/w/bin/bhyve".into(),
        "/w/bin/bhyvectl".into(),
        "/w/lib".into(),
        state,
        "/w/img/kernel".into(),
        "/w/img/initrd".into(),
    );
    let stamp = Stamp::parse("20261201T0540Z").unwrap();
    let name = VmName::new(&stamp, 2).unwrap();
    let run = StateLayout::new(&host.state_dir, false).run_paths(&stamp, &name);
    let files = Inputs { seed: 1007, job: 0 }.write(&run.fwcfg_dir).unwrap();
    let record = CheckpointRecord {
        stamp: stamp.clone(),
        dir: StateLayout::new(&host.state_dir, false).checkpoint_dir(&stamp),
        hashes: ["a".into(), "b".into(), "c".into()],
        keys: RestoreKeys {
            stop_seq: 1,
            branch_count: 395_000,
            vtime_ns: 777,
            probe_id: 0,
            probe_arg0: 0,
            vcpu_id: 0,
        },
        rip: 1,
    };
    Rig {
        commands: Commands::new(&host, &stamp),
        prepare: prepare_args(&host, &run, &files, &name),
        restore: restore_args(&host, &run, &files, &record, &name),
        name,
        host,
    }
}

fn approved(rig: &Rig, args: &[String]) -> Result<Approved, Refusal> {
    rig.commands.check(&rig.commands.bhyve(args))
}

/// `args` with the value that follows `flag` (the first one that starts with `prefix`) replaced by `value`.
fn with_value(args: &[String], flag: &str, prefix: &str, value: &str) -> Vec<String> {
    let mut out = args.to_vec();
    let at = (0..out.len() - 1)
        .find(|&i| out[i] == flag && out[i + 1].starts_with(prefix))
        .unwrap_or_else(|| panic!("no {flag} {prefix}"));
    out[at + 1] = value.to_string();
    out
}

#[test]
fn what_the_builder_produces_is_approved_for_both_shapes() {
    let rig = rig("good");
    assert_eq!(approved(&rig, &rig.prepare), Ok(Approved::Bhyve));
    assert_eq!(approved(&rig, &rig.restore), Ok(Approved::Bhyve));
}

#[test]
fn a_network_backend_or_a_disk_or_a_device_in_a_slot_or_a_console_is_refused() {
    let rig = rig("devices");
    for (flag, prefix, bad) in [
        (
            "-s",
            "31",
            "2,virtio-net,netgraph,path=ngctl,peerhook=ether,hook=vmlink",
        ),
        ("-s", "31", "2,virtio-net,tap0"),
        ("-s", "31", "2,virtio-net,vmnet0"),
        ("-s", "31", "3,virtio-blk,/dev/ada0"),
        ("-s", "31", "3,ahci-hd,/tmp/disk.img"),
        ("-s", "31", "31,lpc,extra"),
        ("-l", "com1", "com1,/dev/ttyu0"),
        ("-l", "com1", "com1,stdio"),
        ("-c", "1", "2"),
        ("-m", "256M", "64G"),
    ] {
        for args in [&rig.prepare, &rig.restore] {
            let hostile = with_value(args, flag, prefix, bad);
            assert_eq!(
                approved(&rig, &hostile),
                Err(Refusal::NotAllowed),
                "{flag} {bad}"
            );
        }
    }
}

#[test]
fn a_path_outside_the_run_or_the_session_is_refused_for_every_option_that_takes_one() {
    let rig = rig("paths");
    let outside = "/dev/ada0";
    for (flag, prefix, key) in [
        ("-o", "dhyve.director.control=", "dhyve.director.control"),
        ("-o", "dhyve.director.uart_log=", "dhyve.director.uart_log"),
        (
            "-o",
            "deterministic.state_manifest=",
            "deterministic.state_manifest",
        ),
        (
            "-o",
            "deterministic.state_regs=",
            "deterministic.state_regs",
        ),
        ("-o", "deterministic.state_mem=", "deterministic.state_mem"),
        (
            "-o",
            "deterministic.state_devices=",
            "deterministic.state_devices",
        ),
        ("-o", "linux.kernel=", "linux.kernel"),
        ("-o", "linux.initrd=", "linux.initrd"),
    ] {
        let hostile = with_value(&rig.prepare, flag, prefix, &format!("{key}={outside}"));
        assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed), "{key}");
    }
    let hostile = with_value(
        &rig.prepare,
        "-f",
        "name=opt/hs/seed",
        "name=opt/hs/seed,file=/etc/passwd",
    );
    assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed));
    let hostile = with_value(
        &rig.prepare,
        "-f",
        "name=opt/hs/seed",
        "name=opt/hs/other,file=/tmp/x",
    );
    assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed));
    let hostile = with_value(&rig.restore, "-r", "/", "/var/tmp/other/boot");
    assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed));
    let not_a_checkpoint = format!(
        "{}/ckpt/20261201T0540Z/boot.kern",
        rig.host.state_dir.display()
    );
    assert_eq!(
        approved(
            &rig,
            &with_value(&rig.restore, "-r", "/", &not_a_checkpoint)
        ),
        Err(Refusal::NotAllowed)
    );
}

#[test]
fn a_changed_value_of_a_fixed_option_is_refused() {
    let rig = rig("fixed");
    for (prefix, bad) in [
        ("dhyve.director.enabled=", "dhyve.director.enabled=false"),
        ("dhyve.log_mask=", "dhyve.log_mask=0xffff"),
        (
            "deterministic.max_branches=",
            "deterministic.max_branches=1",
        ),
        ("lpc.fwcfg=", "lpc.fwcfg=qemu,extra"),
        ("uuid=", "uuid=00000000-0000-0000-0000-000000000000"),
        ("linux.cmdline=", "linux.cmdline=init=/bin/sh"),
        (
            "dhyve.director.heuristic_ready_pattern=",
            "dhyve.director.heuristic_ready_pattern=x",
        ),
        ("deterministic.seed=", "deterministic.seed=-1"),
    ] {
        let hostile = with_value(&rig.prepare, "-o", prefix, bad);
        assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed), "{bad}");
    }
    let hostile = with_value(
        &rig.restore,
        "-o",
        "dhyve.director.restore.stop_seq=",
        "dhyve.director.restore.stop_seq=1;x",
    );
    assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed));
    let hostile = with_value(
        &rig.restore,
        "-o",
        "dhyve.director.restore.kind=",
        "dhyve.director.restore.kind=other",
    );
    assert_eq!(approved(&rig, &hostile), Err(Refusal::NotAllowed));
}

#[test]
fn an_extra_or_a_repeated_option_or_an_unknown_flag_is_refused() {
    let rig = rig("extra");
    let extra = |opt: &str| {
        let mut args = rig.prepare.clone();
        let last = args.pop().unwrap();
        args.extend(["-o".to_string(), opt.to_string(), last]);
        args
    };
    for opt in [
        "evil.option=1",
        "net.backend=netgraph",
        "dhyve.director.enabled=true",
        "uuid=x",
    ] {
        assert_eq!(
            approved(&rig, &extra(opt)),
            Err(Refusal::NotAllowed),
            "{opt}"
        );
    }
    let mut flag = rig.prepare.clone();
    let last = flag.pop().unwrap();
    flag.extend(["-S".to_string(), "x".to_string(), last]);
    assert_eq!(approved(&rig, &flag), Err(Refusal::NotAllowed));
    let mut dangling = rig.prepare.clone();
    let last = dangling.pop().unwrap();
    dangling.extend(["-o".to_string(), last]);
    assert_eq!(approved(&rig, &dangling), Err(Refusal::NotAllowed));
    let _ = &rig.name;
}

//! The one place the VM program's own argument list is formed. The list is never printed, logged or put in an error: the
//! option names are the program's and are not part of anything this crate says to a person.
use crate::checkpoint::CheckpointRecord;
use crate::fwcfg::Files;
use crate::host::HostConfig;
use crate::layout::RunPaths;
use crate::vmname::VmName;
use crate::wire::RESUME_BRANCH_TARGET;

pub(crate) const CPUS: &str = "1";
pub(crate) const MEMORY: &str = "256M";
pub(crate) const READY_PATTERN: &str = "HS_READY";
pub(crate) const LOG_MASK: &str = "0xff7f";
pub(crate) const UUID: &str = "9a1983b1-8f4e-5c2a-9c43-8f7e12d4b6a1";
pub(crate) const CMDLINE: &str =
    "console=ttyS0 nokaslr norandmaps rdinit=/init quiet earlyprintk=serial tsc=reliable \
lpj=8000000 noapictimer hpet=disable nopcid preempt=none nmi_watchdog=0";
/// The seed the VM's own generator is held at on every run.
pub const VM_SEED: u64 = 1;

fn pair(option: &str, value: impl std::fmt::Display) -> [String; 2] {
    ["-o".to_string(), format!("{option}={value}")]
}

fn control_options(run: &RunPaths) -> Vec<String> {
    [
        pair("dhyve.director.enabled", "true"),
        pair("dhyve.director.control", run.control_sock.display()),
        pair("dhyve.director.heuristic_ready_pattern", READY_PATTERN),
        pair("dhyve.director.uart_log", run.uart_log.display()),
    ]
    .concat()
}

fn base(host: &HostConfig, run: &RunPaths, fwcfg: &Files, name: &VmName) -> Vec<String> {
    let mut args: Vec<String> = ["-c", CPUS, "-m", MEMORY, "-s", "31,lpc", "-l", "com1,det"]
        .map(String::from)
        .to_vec();
    args.extend(pair("lpc.fwcfg", "qemu"));
    for item in fwcfg.items() {
        args.extend(["-f".to_string(), item]);
    }
    args.extend(pair("linux.kernel", host.kernel.display()));
    args.extend(pair("linux.initrd", host.initrd.display()));
    args.extend(pair("linux.cmdline", CMDLINE));
    args.extend(pair("deterministic.seed", VM_SEED));
    args.extend(pair("deterministic.max_branches", RESUME_BRANCH_TARGET));
    args.extend(pair("uuid", UUID));
    args.extend(pair(
        "deterministic.state_manifest",
        run.state_manifest.display(),
    ));
    args.extend(pair("deterministic.state_regs", run.state_regs.display()));
    args.extend(pair("deterministic.state_mem", run.state_mem.display()));
    args.extend(pair(
        "deterministic.state_devices",
        run.state_devices.display(),
    ));
    args.extend(pair("dhyve.log_mask", LOG_MASK));
    args.push(name.as_str().to_string());
    args
}

/// A cold boot that runs to the guest's ready stop.
pub fn prepare_args(
    host: &HostConfig,
    run: &RunPaths,
    fwcfg: &Files,
    name: &VmName,
) -> Vec<String> {
    let mut args = control_options(run);
    args.extend(base(host, run, fwcfg, name));
    args
}

/// A restore of `record`, named `name`. The name is the last argument, which is where the module reads it from.
pub fn restore_args(
    host: &HostConfig,
    run: &RunPaths,
    fwcfg: &Files,
    record: &CheckpointRecord,
    name: &VmName,
) -> Vec<String> {
    let mut args = control_options(run);
    args.extend([
        "-r".to_string(),
        record.dir.join("boot").display().to_string(),
    ]);
    for (key, value) in record.keys.options() {
        args.extend(pair(&format!("dhyve.director.restore.{key}"), value));
    }
    args.extend(base(host, run, fwcfg, name));
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::RestoreKeys;
    use crate::fwcfg::Inputs;
    use crate::layout::StateLayout;
    use crate::vmname::Stamp;
    use std::path::PathBuf;

    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    fn host() -> HostConfig {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        HostConfig::new(
            "/w/bin/bhyve".into(),
            "/w/bin/bhyvectl".into(),
            "/w/lib".into(),
            std::env::temp_dir().join(format!("hs-dhyve-args-{}-{n}", std::process::id())),
            "/w/img/kernel".into(),
            "/w/img/initrd".into(),
        )
    }

    fn fixtures() -> (RunPaths, Files, VmName) {
        let h = host();
        let stamp = Stamp::from_counter(1);
        let name = VmName::new(&stamp, 3).unwrap();
        let run = StateLayout::new(&h.state_dir, false).run_paths(&stamp, &name);
        let files = Inputs { seed: 1007, job: 0 }.write(&run.fwcfg_dir).unwrap();
        (run, files, name)
    }

    fn record() -> CheckpointRecord {
        CheckpointRecord {
            stamp: Stamp::from_counter(1),
            dir: PathBuf::from("/w/state/ckpt/x"),
            hashes: ["a".into(), "b".into(), "c".into()],
            keys: RestoreKeys {
                stop_seq: 3,
                branch_count: 1000,
                vtime_ns: 5,
                probe_id: 0,
                probe_arg0: 9,
                vcpu_id: 0,
            },
            rip: 1,
        }
    }

    fn options(args: &[String]) -> Vec<&str> {
        args.windows(2)
            .filter(|w| w[0] == "-o")
            .map(|w| w[1].as_str())
            .collect()
    }

    #[test]
    fn the_name_is_the_last_argument_of_both_shapes() {
        let (run, files, name) = fixtures();
        assert_eq!(
            prepare_args(&host(), &run, &files, &name).last().unwrap(),
            name.as_str()
        );
        let restore = restore_args(&host(), &run, &files, &record(), &name);
        assert_eq!(restore.last().unwrap(), name.as_str());
        assert_eq!(restore.iter().filter(|a| *a == name.as_str()).count(), 1);
    }

    #[test]
    fn a_prepare_has_the_control_options_and_no_restore() {
        let (run, files, name) = fixtures();
        let args = prepare_args(&host(), &run, &files, &name);
        assert!(!args.contains(&"-r".to_string()));
        let opts = options(&args);
        assert!(opts.contains(&"dhyve.director.enabled=true"));
        assert!(opts
            .contains(&format!("dhyve.director.control={}", run.control_sock.display()).as_str()));
        assert!(opts.contains(&"dhyve.director.heuristic_ready_pattern=HS_READY"));
        assert!(
            opts.contains(&format!("dhyve.director.uart_log={}", run.uart_log.display()).as_str())
        );
        assert!(opts.contains(&"dhyve.log_mask=0xff7f"));
        assert!(!opts
            .iter()
            .any(|o| o.starts_with("dhyve.director.restore.")));
    }

    #[test]
    fn a_restore_names_the_checkpoint_and_all_seven_keys() {
        let (run, files, name) = fixtures();
        let args = restore_args(&host(), &run, &files, &record(), &name);
        let at = args.iter().position(|a| a == "-r").unwrap();
        assert_eq!(args[at + 1], "/w/state/ckpt/x/boot");
        let keys: Vec<_> = options(&args)
            .into_iter()
            .filter(|o| o.starts_with("dhyve.director.restore."))
            .collect();
        assert_eq!(keys.len(), 7);
        assert!(keys.contains(&"dhyve.director.restore.stop_seq=3"));
        assert!(keys.contains(&"dhyve.director.restore.kind=ready"));
    }

    #[test]
    fn both_shapes_carry_six_input_files_one_cpu_and_the_fixed_seed_and_limit() {
        let (run, files, name) = fixtures();
        for args in [
            prepare_args(&host(), &run, &files, &name),
            restore_args(&host(), &run, &files, &record(), &name),
        ] {
            assert_eq!(args.iter().filter(|a| *a == "-f").count(), 6);
            assert_eq!(args[args.iter().position(|a| a == "-c").unwrap() + 1], "1");
            let opts = options(&args);
            assert!(opts.contains(&"deterministic.seed=1"));
            assert!(opts.contains(&"deterministic.max_branches=4000000000"));
            assert!(opts.contains(&"linux.kernel=/w/img/kernel"));
        }
    }
}

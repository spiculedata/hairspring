//! What a VM start may be given, option by option and value by value. A start runs as root, so an option that names a
//! network backend, a disk, a console device, or a path outside this run's own directory is refused, whatever its name: a
//! network backend may make a library load a kernel module, and a device or a path hands the guest or the root program a
//! host resource. Every option is allowed once, with the one value the builder gives it, or a plain number where the value
//! is a number, or a path that is exactly one of this run's own files.
use crate::fwcfg::FIELDS;
use crate::host::HostConfig;
use crate::layout::StateLayout;
use crate::vmargs::{CMDLINE, CPUS, LOG_MASK, MEMORY, READY_PATTERN, UUID};
use crate::vmname::{Stamp, VmName};
use crate::wire::RESUME_BRANCH_TARGET;
use std::collections::BTreeSet;
use std::path::Path;

/// The restore options after their common prefix, and whether each takes a plain number.
const RESTORE_KEYS: [(&str, bool); 7] = [
    ("kind", false),
    ("stop_seq", true),
    ("branch_count", true),
    ("vtime_ns", true),
    ("probe_id", true),
    ("probe_arg0", true),
    ("vcpu_id", true),
];
const RESTORE_PREFIX: &str = "dhyve.director.restore.";

fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn number(text: &str) -> bool {
    !text.is_empty() && text.len() <= 20 && text.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `args` (the VM program's arguments, the name last) is a start of this session: every value is one the builder
/// gives for the run named by the last argument.
pub(crate) fn start_args_ok(
    host: &HostConfig,
    layout: &StateLayout,
    stamp: &Stamp,
    checkpoint_dir: &Path,
    args: &[String],
) -> bool {
    let Some((name_text, options)) = args.split_last() else {
        return false;
    };
    let Some(name) = VmName::parse(name_text).filter(|n| n.stamp() == *stamp) else {
        return false;
    };
    let run = layout.run_paths(stamp, &name);
    let ckpt = shown(&checkpoint_dir.join("boot"));
    let mut seen = BTreeSet::new();
    let mut at = 0;
    while at < options.len() {
        let (Some(flag), Some(value)) = (options.get(at), options.get(at + 1)) else {
            return false;
        };
        let fixed = |want: &str| (value == want).then(|| flag.clone());
        let key = match flag.as_str() {
            "-c" => fixed(CPUS),
            "-m" => fixed(MEMORY),
            "-s" => fixed("31,lpc"),
            "-l" => fixed("com1,det"),
            "-r" => fixed(&ckpt),
            "-f" => value
                .strip_prefix("name=opt/hs/")
                .and_then(|rest| rest.split_once(",file="))
                .filter(|(field, file)| {
                    FIELDS.iter().any(|(f, _)| f == field)
                        && *file == shown(&run.fwcfg_dir.join(field))
                })
                .map(|(field, _)| format!("-f {field}")),
            "-o" => value
                .split_once('=')
                .and_then(|(k, v)| option_ok(host, &run, k, v).then(|| format!("-o {k}"))),
            _ => None,
        };
        let Some(key) = key else {
            return false;
        };
        if !seen.insert(key) {
            return false;
        }
        at += 2;
    }
    true
}

fn option_ok(host: &HostConfig, run: &crate::layout::RunPaths, key: &str, value: &str) -> bool {
    if let Some(rest) = key.strip_prefix(RESTORE_PREFIX) {
        return RESTORE_KEYS.iter().any(|(k, numeric)| {
            *k == rest
                && if *numeric {
                    number(value)
                } else {
                    value == "ready"
                }
        });
    }
    let branches = RESUME_BRANCH_TARGET.to_string();
    match key {
        "dhyve.director.enabled" => value == "true",
        "dhyve.director.control" => value == shown(&run.control_sock),
        "dhyve.director.heuristic_ready_pattern" => value == READY_PATTERN,
        "dhyve.director.uart_log" => value == shown(&run.uart_log),
        "dhyve.log_mask" => value == LOG_MASK,
        "lpc.fwcfg" => value == "qemu",
        "linux.kernel" => value == shown(&host.kernel),
        "linux.initrd" => value == shown(&host.initrd),
        "linux.cmdline" => value == CMDLINE,
        "deterministic.seed" => number(value),
        "deterministic.max_branches" => value == branches,
        "deterministic.state_manifest" => value == shown(&run.state_manifest),
        "deterministic.state_regs" => value == shown(&run.state_regs),
        "deterministic.state_mem" => value == shown(&run.state_mem),
        "deterministic.state_devices" => value == shown(&run.state_devices),
        "uuid" => value == UUID,
        _ => false,
    }
}

//! The argument lists of every command this backend runs, and the allow-list that decides whether a list may be run.
//!
//! The backend itself runs unprivileged. The three commands that need root (start a VM, destroy a VM, checkpoint a VM)
//! run as `sudo -n env -i LD_LIBRARY_PATH=<lib> <program> ...`, with the pinned program and its pinned library
//! directory. The only other commands are read-only questions of the machine check and a free-space question. Nothing
//! else is ever run: in particular a program that loads or unloads a kernel module is refused by name, whatever its path.
use crate::host::HostConfig;
use crate::vmname::VmName;
use std::path::Path;

/// The read-only programs of the machine check, by pinned absolute path.
pub const KLDSTAT: &str = "/sbin/kldstat";
pub const SYSCTL: &str = "/sbin/sysctl";
pub const DF: &str = "/bin/df";
/// The two settings the machine check may ask for.
pub const SYSCTL_NAMES: [&str; 2] = ["kern.osrelease", "machdep.panic_on_nmi"];
/// The programs that are refused by name.
const MODULE_PROGRAMS: [&str; 2] = ["kldload", "kldunload"];

/// A command that may be run, by what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approved {
    Bhyve,
    Destroy,
    Checkpoint,
    Kldstat,
    Sysctl,
    FreeSpace,
}

/// Why a command was refused. The text never carries the argument list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The command names a program that loads or unloads a kernel module.
    KernelModule,
    NotAllowed,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::KernelModule => write!(
                f,
                "refused to run a program that loads or unloads kernel modules: an operator does that, never this backend"
            ),
            Refusal::NotAllowed => write!(f, "refused to run a command this backend does not use"),
        }
    }
}

impl std::error::Error for Refusal {}

fn base_name(text: &str) -> &str {
    text.rsplit('/').next().unwrap_or(text)
}

fn names_module_program(arg: &str) -> bool {
    let value = arg.split_once('=').map_or(arg, |(_, v)| v);
    MODULE_PROGRAMS.contains(&base_name(arg)) || MODULE_PROGRAMS.contains(&base_name(value))
}

fn plain(arg: &str) -> bool {
    !arg.contains(['\0', '\n'])
}

fn refuse_module_programs(argv: &[String]) -> Result<(), Refusal> {
    if argv.iter().any(|a| names_module_program(a)) {
        return Err(Refusal::KernelModule);
    }
    if argv.is_empty() || !argv.iter().all(|a| plain(a)) {
        return Err(Refusal::NotAllowed);
    }
    Ok(())
}

/// Whether `argv` is one of the read-only questions the machine check and the free-space check ask. These need no host
/// settings, so the machine check can use this without any.
pub fn check_read_only(argv: &[String]) -> Result<Approved, Refusal> {
    refuse_module_programs(argv)?;
    let as_strs: Vec<&str> = argv.iter().map(String::as_str).collect();
    match as_strs.as_slice() {
        [KLDSTAT, "-q", "-n", "vmm"] => Ok(Approved::Kldstat),
        [SYSCTL, "-n", name] if SYSCTL_NAMES.contains(name) => Ok(Approved::Sysctl),
        [DF, "-Pk", dir] if !dir.starts_with('-') && !dir.is_empty() => Ok(Approved::FreeSpace),
        _ => Err(Refusal::NotAllowed),
    }
}

/// Builds argument lists and checks them.
#[derive(Debug, Clone)]
pub struct Commands {
    host: HostConfig,
}

impl Commands {
    pub fn new(host: &HostConfig) -> Commands {
        Commands { host: host.clone() }
    }

    fn display(path: &Path) -> String {
        path.display().to_string()
    }

    fn root_prefix(&self) -> Vec<String> {
        vec![
            Self::display(&self.host.sudo),
            "-n".to_string(),
            Self::display(&self.host.env),
            "-i".to_string(),
            format!("LD_LIBRARY_PATH={}", self.host.lib_dir.display()),
        ]
    }

    /// Start a VM: the pinned `bhyve` with `args`, whose last element is the VM's name.
    pub fn bhyve(&self, args: &[String]) -> Vec<String> {
        let mut argv = self.root_prefix();
        argv.push(Self::display(&self.host.bhyve));
        argv.extend_from_slice(args);
        argv
    }

    pub fn destroy(&self, name: &VmName) -> Vec<String> {
        let mut argv = self.root_prefix();
        argv.extend([
            Self::display(&self.host.bhyvectl),
            "--destroy".to_string(),
            format!("--vm={name}"),
        ]);
        argv
    }

    /// Checkpoint a running VM into `<dir>/boot` (and its two companion files).
    pub fn checkpoint(&self, dir: &Path, name: &VmName) -> Vec<String> {
        let mut argv = self.root_prefix();
        argv.extend([
            Self::display(&self.host.bhyvectl),
            format!("--checkpoint={}", dir.join("boot").display()),
            format!("--vm={name}"),
        ]);
        argv
    }

    pub fn kldstat() -> Vec<String> {
        [KLDSTAT, "-q", "-n", "vmm"].map(String::from).to_vec()
    }

    pub fn sysctl(name: &str) -> Vec<String> {
        [SYSCTL, "-n", name].map(String::from).to_vec()
    }

    pub fn free_space(dir: &Path) -> Vec<String> {
        vec![DF.to_string(), "-Pk".to_string(), Self::display(dir)]
    }

    /// Whether `argv` is exactly one of the shapes this backend runs.
    pub fn check(&self, argv: &[String]) -> Result<Approved, Refusal> {
        refuse_module_programs(argv)?;
        let prefix = self.root_prefix();
        if let Some(rest) = argv.strip_prefix(prefix.as_slice()) {
            return self.check_root(rest);
        }
        check_read_only(argv)
    }

    fn check_root(&self, rest: &[String]) -> Result<Approved, Refusal> {
        // The caller has already refused module programs and unprintable arguments.
        let (program, args) = rest.split_first().ok_or(Refusal::NotAllowed)?;
        if *program == Self::display(&self.host.bhyve) {
            let named = args.last().is_some_and(|n| VmName::parse(n).is_some());
            return if named {
                Ok(Approved::Bhyve)
            } else {
                Err(Refusal::NotAllowed)
            };
        }
        if *program != Self::display(&self.host.bhyvectl) {
            return Err(Refusal::NotAllowed);
        }
        let named = |arg: &String| {
            arg.strip_prefix("--vm=")
                .is_some_and(|n| VmName::parse(n).is_some())
        };
        match args {
            [flag, vm] if flag == "--destroy" && named(vm) => Ok(Approved::Destroy),
            [target, vm] if named(vm) && is_checkpoint_target(target) => Ok(Approved::Checkpoint),
            _ => Err(Refusal::NotAllowed),
        }
    }
}

fn is_checkpoint_target(arg: &str) -> bool {
    arg.strip_prefix("--checkpoint=")
        .is_some_and(|p| p.starts_with('/') && p.ends_with("/boot") && !p.contains("/../"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vmname::Stamp;
    use std::path::PathBuf;

    fn host() -> HostConfig {
        HostConfig {
            sudo: "/usr/local/bin/sudo".into(),
            env: "/usr/bin/env".into(),
            bhyve: "/w/bin/bhyve".into(),
            bhyvectl: "/w/bin/bhyvectl".into(),
            lib_dir: "/w/lib".into(),
            state_dir: "/w/state".into(),
            kernel: "/w/img/kernel".into(),
            initrd: "/w/img/initrd".into(),
        }
    }

    fn name() -> VmName {
        VmName::new(&Stamp::parse("20261123T1530Z").unwrap(), 4).unwrap()
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn the_root_commands_have_exactly_the_unprivileged_wrapper_shape() {
        let c = Commands::new(&host());
        assert_eq!(
            c.destroy(&name()),
            strings(&[
                "/usr/local/bin/sudo",
                "-n",
                "/usr/bin/env",
                "-i",
                "LD_LIBRARY_PATH=/w/lib",
                "/w/bin/bhyvectl",
                "--destroy",
                "--vm=vmvx_20261123T1530Z_XB_r04",
            ])
        );
        assert_eq!(
            c.checkpoint(&PathBuf::from("/w/state/ck"), &name()),
            strings(&[
                "/usr/local/bin/sudo",
                "-n",
                "/usr/bin/env",
                "-i",
                "LD_LIBRARY_PATH=/w/lib",
                "/w/bin/bhyvectl",
                "--checkpoint=/w/state/ck/boot",
                "--vm=vmvx_20261123T1530Z_XB_r04",
            ])
        );
        let start = c.bhyve(&strings(&["-c", "1", "vmvx_20261123T1530Z_XB_r04"]));
        assert_eq!(
            &start[..6],
            &strings(&[
                "/usr/local/bin/sudo",
                "-n",
                "/usr/bin/env",
                "-i",
                "LD_LIBRARY_PATH=/w/lib",
                "/w/bin/bhyve",
            ])[..]
        );
    }

    #[test]
    fn every_command_it_builds_is_approved_as_what_it_is() {
        let c = Commands::new(&host());
        let n = name();
        assert_eq!(c.check(&c.destroy(&n)), Ok(Approved::Destroy));
        assert_eq!(
            c.check(&c.checkpoint(&PathBuf::from("/w/state/ck"), &n)),
            Ok(Approved::Checkpoint)
        );
        assert_eq!(
            c.check(&c.bhyve(&strings(&["-c", "1", n.as_str()]))),
            Ok(Approved::Bhyve)
        );
        assert_eq!(c.check(&Commands::kldstat()), Ok(Approved::Kldstat));
        assert_eq!(
            c.check(&Commands::sysctl("kern.osrelease")),
            Ok(Approved::Sysctl)
        );
        assert_eq!(
            c.check(&Commands::sysctl("machdep.panic_on_nmi")),
            Ok(Approved::Sysctl)
        );
        assert_eq!(
            c.check(&Commands::free_space(&PathBuf::from("/w/state"))),
            Ok(Approved::FreeSpace)
        );
    }

    #[test]
    fn module_programs_are_refused_by_name_whatever_the_path_or_wrapper() {
        let c = Commands::new(&host());
        for argv in [
            strings(&["/sbin/kldload", "/w/bin/vmm.ko"]),
            strings(&["/sbin/kldunload", "vmm"]),
            strings(&["kldload", "vmm"]),
            strings(&["/tmp/anywhere/kldunload", "vmm"]),
            strings(&[
                "/usr/local/bin/sudo",
                "-n",
                "/sbin/kldload",
                "/w/bin/vmm.ko",
            ]),
            strings(&[
                "/usr/local/bin/sudo",
                "-n",
                "/usr/bin/env",
                "-i",
                "LD_LIBRARY_PATH=/w/lib",
                "/sbin/kldunload",
                "vmm",
            ]),
            strings(&["/bin/sh", "-c", "x", "--prog=/sbin/kldload"]),
        ] {
            assert_eq!(c.check(&argv), Err(Refusal::KernelModule), "{argv:?}");
        }
    }

    #[test]
    fn a_command_that_is_not_one_of_the_shapes_is_refused() {
        let c = Commands::new(&host());
        let n = name();
        let good = c.destroy(&n);
        let without_n = {
            let mut v = good.clone();
            v.remove(1);
            v
        };
        let other_lib = {
            let mut v = good.clone();
            v[4] = "LD_LIBRARY_PATH=/usr/lib".into();
            v
        };
        let extra_env = {
            let mut v = good.clone();
            v.insert(5, "HOME=/root".into());
            v
        };
        let other_flag = {
            let mut v = good.clone();
            v[6] = "--create".into();
            v
        };
        let plain_name = {
            let mut v = good.clone();
            v[7] = "--vm=hs_1".into();
            v
        };
        let unwrapped_vm = strings(&["/w/bin/bhyve", "-c", "1", n.as_str()]);
        let unnamed_start = c.bhyve(&strings(&["-c", "1"]));
        let other_program = {
            let mut v = good.clone();
            v[5] = "/bin/sh".into();
            v
        };
        let relative_checkpoint = {
            let mut v = c.checkpoint(&PathBuf::from("/w/state/ck"), &n);
            v[6] = "--checkpoint=rel/boot".into();
            v
        };
        let traversing_checkpoint = {
            let mut v = c.checkpoint(&PathBuf::from("/w/state/ck"), &n);
            v[6] = "--checkpoint=/w/../etc/boot".into();
            v
        };
        for (what, argv) in [
            ("no -n", without_n),
            ("other lib", other_lib),
            ("extra env", extra_env),
            ("other flag", other_flag),
            ("plain name", plain_name),
            ("no wrapper", unwrapped_vm),
            ("no name", unnamed_start),
            ("other program", other_program),
            ("relative checkpoint", relative_checkpoint),
            ("traversing checkpoint", traversing_checkpoint),
            ("empty", Vec::new()),
            ("sysctl write", strings(&["/sbin/sysctl", "-w", "x=1"])),
            (
                "other sysctl",
                strings(&["/sbin/sysctl", "-n", "kern.ostype"]),
            ),
            ("kldstat other", strings(&["/sbin/kldstat", "-v"])),
            ("df option", strings(&["/bin/df", "-Pk", "-x"])),
            ("newline", strings(&["/bin/df", "-Pk", "a\nb"])),
        ] {
            assert_eq!(c.check(&argv), Err(Refusal::NotAllowed), "{what}");
        }
    }

    #[test]
    fn refusals_say_nothing_about_the_argument_list() {
        let text = Refusal::NotAllowed.to_string();
        assert!(!text.contains("--"));
        assert!(Refusal::KernelModule.to_string().contains("kernel modules"));
    }
}

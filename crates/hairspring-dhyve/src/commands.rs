//! The argument lists of every command this backend runs, and the allow-list that decides whether a list may be run.
//!
//! The backend itself runs unprivileged. The three commands that need root (start a VM, destroy a VM, checkpoint a VM)
//! run as `sudo -n env -i LD_LIBRARY_PATH=<lib> <program> ...` with the pinned programs and library directory; a start
//! also runs under `cpuset -l <core>`. The only other commands are read-only questions: the machine check's, `kldstat`
//! and a free-space question. Nothing else is ever run. A program that loads or unloads a kernel module is refused by
//! name, and a command that names a VM of another stamp than this session's, or a checkpoint anywhere but this session's
//! checkpoint directory, is refused.
use crate::host::{HostConfig, ALLOWED_CORES};
use crate::layout::StateLayout;
use crate::startcheck::start_args_ok;
use crate::vmname::{Stamp, VmName};
use std::path::{Path, PathBuf};

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
    KldstatIdentity,
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

/// Whether `argv` is one of the read-only questions the machine check and the free-space check ask, with `kldstat`
/// taken to be the pinned one. These need no host settings, so the machine check can use this without any.
pub fn check_read_only(argv: &[String]) -> Result<Approved, Refusal> {
    check_read_only_with(KLDSTAT, argv)
}

fn check_read_only_with(kldstat: &str, argv: &[String]) -> Result<Approved, Refusal> {
    refuse_module_programs(argv)?;
    let as_strs: Vec<&str> = argv.iter().map(String::as_str).collect();
    match as_strs.as_slice() {
        [program, "-q", "-n", "vmm"] if *program == kldstat => Ok(Approved::Kldstat),
        [program, "-n", "vmm"] if *program == kldstat => Ok(Approved::KldstatIdentity),
        [SYSCTL, "-n", name] if SYSCTL_NAMES.contains(name) => Ok(Approved::Sysctl),
        [DF, "-Pk", dir] if !dir.starts_with('-') && !dir.is_empty() => Ok(Approved::FreeSpace),
        _ => Err(Refusal::NotAllowed),
    }
}

/// Builds argument lists and checks them, for one session stamp.
#[derive(Debug, Clone)]
pub struct Commands {
    host: HostConfig,
    stamp: Stamp,
    checkpoint_dir: PathBuf,
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

impl Commands {
    pub fn new(host: &HostConfig, stamp: &Stamp) -> Commands {
        let layout = StateLayout::new(&host.state_dir, host.session_stamp.is_some());
        Commands {
            checkpoint_dir: layout.checkpoint_dir(stamp),
            host: host.clone(),
            stamp: stamp.clone(),
        }
    }

    fn root_prefix(&self) -> Vec<String> {
        vec![
            shown(&self.host.sudo),
            "-n".to_string(),
            shown(&self.host.env),
            "-i".to_string(),
            format!("LD_LIBRARY_PATH={}", self.host.lib_dir.display()),
        ]
    }

    /// The same commands for a checkpoint that is kept in `dir`: a restore named under a stamp of its own still restores the
    /// checkpoint of the prepare.
    pub fn with_checkpoint_dir(self, dir: &Path) -> Commands {
        Commands {
            checkpoint_dir: dir.to_path_buf(),
            ..self
        }
    }

    /// Start a VM on the pinned core: the pinned `bhyve` with `args`, whose last element is the VM's name.
    pub fn bhyve(&self, args: &[String]) -> Vec<String> {
        let mut argv = self.root_prefix();
        argv.extend([
            shown(&self.host.cpuset),
            "-l".to_string(),
            self.host.core.to_string(),
            shown(&self.host.bhyve),
        ]);
        argv.extend_from_slice(args);
        argv
    }

    pub fn destroy(&self, name: &VmName) -> Vec<String> {
        let mut argv = self.root_prefix();
        argv.extend([
            shown(&self.host.bhyvectl),
            "--destroy".to_string(),
            format!("--vm={name}"),
        ]);
        argv
    }

    /// Checkpoint a running VM into this session's checkpoint directory.
    pub fn checkpoint(&self, name: &VmName) -> Vec<String> {
        let mut argv = self.root_prefix();
        argv.extend([
            shown(&self.host.bhyvectl),
            format!(
                "--checkpoint={}",
                self.checkpoint_dir.join("boot").display()
            ),
            format!("--vm={name}"),
        ]);
        argv
    }

    /// Whether the `vmm` module is loaded.
    pub fn kldstat(&self) -> Vec<String> {
        vec![
            shown(&self.host.kldstat),
            "-q".into(),
            "-n".into(),
            "vmm".into(),
        ]
    }

    /// What the loaded `vmm` module is: its line of the module list.
    pub fn kldstat_identity(&self) -> Vec<String> {
        vec![shown(&self.host.kldstat), "-n".into(), "vmm".into()]
    }

    pub fn sysctl(name: &str) -> Vec<String> {
        [SYSCTL, "-n", name].map(String::from).to_vec()
    }

    pub fn free_space(dir: &Path) -> Vec<String> {
        vec![DF.to_string(), "-Pk".to_string(), shown(dir)]
    }

    /// Whether `argv` is exactly one of the shapes this backend runs.
    pub fn check(&self, argv: &[String]) -> Result<Approved, Refusal> {
        refuse_module_programs(argv)?;
        let prefix = self.root_prefix();
        if let Some(rest) = argv.strip_prefix(prefix.as_slice()) {
            return self.check_root(rest);
        }
        check_read_only_with(&shown(&self.host.kldstat), argv)
    }

    fn named(&self, text: &str) -> bool {
        VmName::parse(text).is_some_and(|n| n.stamp() == self.stamp)
    }

    fn check_root(&self, rest: &[String]) -> Result<Approved, Refusal> {
        // The caller has already refused module programs and unprintable arguments.
        let cpuset = shown(&self.host.cpuset);
        if rest.first() == Some(&cpuset) {
            return self.check_start(&rest[1..]);
        }
        let (program, args) = rest.split_first().ok_or(Refusal::NotAllowed)?;
        if *program != shown(&self.host.bhyvectl) {
            return Err(Refusal::NotAllowed);
        }
        let vm = |arg: &String| arg.strip_prefix("--vm=").is_some_and(|n| self.named(n));
        let target = format!(
            "--checkpoint={}",
            self.checkpoint_dir.join("boot").display()
        );
        match args {
            [flag, name] if flag == "--destroy" && vm(name) => Ok(Approved::Destroy),
            [t, name] if *t == target && vm(name) => Ok(Approved::Checkpoint),
            _ => Err(Refusal::NotAllowed),
        }
    }

    /// `-l <core> <bhyve> <args...>` after the `cpuset` program.
    fn check_start(&self, rest: &[String]) -> Result<Approved, Refusal> {
        let [flag, core, program, args @ ..] = rest else {
            return Err(Refusal::NotAllowed);
        };
        let core_ok =
            core == &self.host.core.to_string() && ALLOWED_CORES.contains(&self.host.core);
        if flag != "-l" || !core_ok || *program != shown(&self.host.bhyve) {
            return Err(Refusal::NotAllowed);
        }
        let layout = StateLayout::new(&self.host.state_dir, self.host.session_stamp.is_some());
        if !start_args_ok(&self.host, &layout, &self.stamp, &self.checkpoint_dir, args) {
            return Err(Refusal::NotAllowed);
        }
        Ok(Approved::Bhyve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> HostConfig {
        HostConfig::new(
            "/w/bin/bhyve".into(),
            "/w/bin/bhyvectl".into(),
            "/w/lib".into(),
            "/w/state".into(),
            "/w/img/kernel".into(),
            "/w/img/initrd".into(),
        )
    }

    fn stamp() -> Stamp {
        Stamp::parse("20261123T1530Z").unwrap()
    }

    fn commands() -> Commands {
        Commands::new(&host(), &stamp())
    }

    fn name(run: u32) -> VmName {
        VmName::new(&stamp(), run).unwrap()
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    fn start_args(n: &VmName) -> Vec<String> {
        let run = format!(
            "/w/state/runs/{}/r{:02}",
            n.stamp().as_str(),
            n.run_number()
        );
        strings(&[
            "-c",
            "1",
            "-o",
            "dhyve.director.enabled=true",
            "-f",
            &format!("name=opt/hs/seed,file={run}/fwcfg/seed"),
            n.as_str(),
        ])
    }

    #[test]
    fn the_root_commands_have_exactly_the_unprivileged_wrapper_shape() {
        let c = commands();
        let wrapper = [
            "/usr/local/bin/sudo",
            "-n",
            "/usr/bin/env",
            "-i",
            "LD_LIBRARY_PATH=/w/lib",
        ];
        let mut destroy = strings(&wrapper);
        destroy.extend(strings(&[
            "/w/bin/bhyvectl",
            "--destroy",
            "--vm=vmvx_20261123T1530Z_XB_r04",
        ]));
        assert_eq!(c.destroy(&name(4)), destroy);
        let mut checkpoint = strings(&wrapper);
        checkpoint.extend(strings(&[
            "/w/bin/bhyvectl",
            "--checkpoint=/w/state/ckpt/20261123T1530Z/boot",
            "--vm=vmvx_20261123T1530Z_XB_r01",
        ]));
        assert_eq!(c.checkpoint(&name(1)), checkpoint);
        let mut start = strings(&wrapper);
        start.extend(strings(&[
            "/usr/bin/cpuset",
            "-l",
            "1",
            "/w/bin/bhyve",
            "-c",
            "1",
        ]));
        assert_eq!(c.bhyve(&strings(&["-c", "1"])), start);
    }

    #[test]
    fn every_command_it_builds_is_approved_as_what_it_is() {
        let c = commands();
        let n = name(4);
        assert_eq!(c.check(&c.destroy(&n)), Ok(Approved::Destroy));
        assert_eq!(c.check(&c.checkpoint(&name(1))), Ok(Approved::Checkpoint));
        assert_eq!(c.check(&c.bhyve(&start_args(&n))), Ok(Approved::Bhyve));
        assert_eq!(c.check(&c.kldstat()), Ok(Approved::Kldstat));
        assert_eq!(
            c.check(&c.kldstat_identity()),
            Ok(Approved::KldstatIdentity)
        );
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
    fn a_session_with_its_own_stamp_keeps_its_checkpoint_in_the_ckpt_directory() {
        let mut h = host();
        h.session_stamp = Some(stamp());
        let c = Commands::new(&h, &stamp());
        assert!(c
            .checkpoint(&name(1))
            .contains(&"--checkpoint=/w/state/ckpt/boot".to_string()));
        assert_eq!(c.check(&c.checkpoint(&name(1))), Ok(Approved::Checkpoint));
    }

    #[test]
    fn module_programs_are_refused_by_name_whatever_the_path_or_wrapper() {
        let c = commands();
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

    /// No shape that is approved can start a program that loads a module, whatever is put in any place of an argument
    /// list. Every combination of a prefix, a program and a tail is tried; an approved one must name only the pinned
    /// programs, and none may carry a module program anywhere.
    #[test]
    fn no_argument_shape_can_cause_a_module_load() {
        let c = commands();
        let n = name(2);
        let mut seeds: Vec<Vec<String>> = vec![
            c.destroy(&n),
            c.checkpoint(&name(1)),
            c.bhyve(&start_args(&n)),
            c.kldstat(),
            c.kldstat_identity(),
            Commands::sysctl("kern.osrelease"),
            Commands::free_space(Path::new("/w/state")),
        ];
        let intruders = [
            "/sbin/kldload",
            "kldunload",
            "/x/kldload",
            "--vm=/sbin/kldload",
            "-o=kldunload",
        ];
        let mut all = seeds.clone();
        for seed in seeds.drain(..) {
            for at in 0..=seed.len() {
                for intruder in intruders {
                    let mut inserted = seed.clone();
                    inserted.insert(at, intruder.to_string());
                    all.push(inserted);
                    if at < seed.len() {
                        let mut replaced = seed.clone();
                        replaced[at] = intruder.to_string();
                        all.push(replaced);
                    }
                }
            }
        }
        let programs = [
            "/usr/local/bin/sudo",
            "/usr/bin/env",
            "/usr/bin/cpuset",
            "/w/bin/bhyve",
            "/w/bin/bhyvectl",
            "/sbin/kldstat",
            "/sbin/sysctl",
            "/bin/df",
        ];
        for argv in &all {
            let verdict = c.check(argv);
            let module = argv.iter().any(|a| names_module_program(a));
            if module {
                assert_eq!(verdict, Err(Refusal::KernelModule), "{argv:?}");
            }
            if verdict.is_ok() {
                assert!(!module, "{argv:?}");
                assert!(
                    argv.iter().all(|a| programs.contains(&a.as_str())
                        || !a.starts_with('/')
                        || a.starts_with("/w/")
                        || a.starts_with("--")),
                    "{argv:?}"
                );
            }
        }
        assert!(all.len() > 100);
    }

    #[test]
    fn a_command_that_is_not_one_of_the_shapes_is_refused() {
        let c = commands();
        let n = name(4);
        let good = c.destroy(&n);
        let edit = |at: usize, text: &str| {
            let mut v = good.clone();
            v[at] = text.to_string();
            v
        };
        let without_n = {
            let mut v = good.clone();
            v.remove(1);
            v
        };
        let extra_env = {
            let mut v = good.clone();
            v.insert(5, "HOME=/root".into());
            v
        };
        let start = c.bhyve(&start_args(&n));
        let start_without_cpuset = {
            let mut v = start.clone();
            v.drain(5..8);
            v
        };
        let start_other_core = {
            let mut v = start.clone();
            v[7] = "0".into();
            v
        };
        let start_other_option = {
            let mut args = start_args(&n);
            args.insert(0, "-o".into());
            args.insert(1, "evil.option=1".into());
            c.bhyve(&args)
        };
        let start_other_flag = c.bhyve(&strings(&["-x", "1", n.as_str()]));
        let start_other_stamp = c.bhyve(&start_args(
            &VmName::new(&Stamp::parse("20261124T0000Z").unwrap(), 2).unwrap(),
        ));
        let other_stamp = VmName::new(&Stamp::parse("20261124T0000Z").unwrap(), 2).unwrap();
        for (what, argv) in [
            ("no -n", without_n),
            ("other lib", edit(4, "LD_LIBRARY_PATH=/usr/lib")),
            ("extra env", extra_env),
            ("other flag", edit(6, "--create")),
            ("plain name", edit(7, "--vm=hs_1")),
            ("other stamp destroy", c.destroy(&other_stamp)),
            ("other stamp checkpoint", c.checkpoint(&other_stamp)),
            ("other program", edit(5, "/bin/sh")),
            (
                "no wrapper",
                strings(&["/w/bin/bhyve", "-c", "1", n.as_str()]),
            ),
            ("no cpuset", start_without_cpuset),
            ("other core", start_other_core),
            ("other option", start_other_option),
            ("other start flag", start_other_flag),
            ("other stamp start", start_other_stamp),
            ("no name", c.bhyve(&strings(&["-c", "1"]))),
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
        let elsewhere = {
            let mut v = c.checkpoint(&name(1));
            v[6] = "--checkpoint=/w/state/ckpt/other/boot".into();
            v
        };
        let traversing = {
            let mut v = c.checkpoint(&name(1));
            v[6] = "--checkpoint=/w/state/ckpt/20261123T1530Z/../x/boot".into();
            v
        };
        for argv in [elsewhere, traversing] {
            assert_eq!(c.check(&argv), Err(Refusal::NotAllowed));
        }
    }

    #[test]
    fn refusals_say_nothing_about_the_argument_list() {
        let text = Refusal::NotAllowed.to_string();
        assert!(!text.contains("--"));
        assert!(Refusal::KernelModule.to_string().contains("kernel modules"));
    }
}

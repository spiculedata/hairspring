//! The fake `bhyvectl` and the fake `sudo`.
//!
//! `bhyvectl --destroy --vm=NAME` removes the name (exit 1 when it was not there, as the real one does for a VM that is
//! not there). `bhyvectl --checkpoint=<dir>/boot --vm=NAME` exits 2 with no output when `<dir>` is missing, 1 when the VM
//! does not exist, and otherwise writes the three checkpoint files; the `ctl` file can make it fail or write a file whose
//! restored pointer cannot be decoded.
use super::scenario::{environment_ok, Fake};
use hairspring_dhyve::ckptmeta::synthetic;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

/// What the fake `kldstat` prints for the `vmm` module, unless the `ctl` file says another line.
const KLD_LINE: &str = " 7    1 0xffffffff82100000 3a000 vmm.ko";

/// The restored pointer the fake's checkpoints decode to.
pub const FAKE_RIP: u64 = 0xffff_ffff_8012_3456;

/// `kldstat -q -n vmm` (exit status only) and `kldstat -n vmm` (the module's line). `kld=absent` says it is not loaded.
pub fn kldstat(fake: &Fake, args: &[String]) -> i32 {
    fake.log_call(&format!("kldstat {}", args.join(" ")));
    if fake.setting("kld") == Some("absent".into()) {
        return 1;
    }
    if args.first().map(String::as_str) != Some("-q") {
        let line = fake
            .setting("kld_line")
            .unwrap_or_else(|| KLD_LINE.to_string());
        println!("Id Refs Address                Size Name\n{line}");
    }
    0
}

pub fn bhyvectl(fake: &Fake, args: &[String]) -> i32 {
    fake.log_call(&format!("bhyvectl {}", args.join(" ")));
    if let Err(why) = environment_ok(fake) {
        eprintln!("{why}");
        return 70;
    }
    let vm = args.iter().find_map(|a| a.strip_prefix("--vm="));
    let (Some(vm), Some(action)) = (vm, args.first()) else {
        return 64;
    };
    if action == "--destroy" {
        if fake.setting("destroy") == Some("hang".into()) {
            sleep(Duration::from_secs(4));
        }
        return if fake.setting("destroy") == Some("fail".into()) || !fake.unregister(vm) {
            1
        } else {
            0
        };
    }
    let Some(target) = action.strip_prefix("--checkpoint=") else {
        return 64;
    };
    checkpoint(fake, vm, Path::new(target))
}

fn checkpoint(fake: &Fake, vm: &str, target: &Path) -> i32 {
    let dir_missing = target.parent().is_none_or(|d| !d.is_dir());
    if dir_missing {
        return 2;
    }
    if !fake.exists(vm) || fake.setting("checkpoint") == Some("fail".into()) {
        return 1;
    }
    let (meta, kern) = match fake.setting("checkpoint").as_deref() {
        Some("undecodable") => ("{}".to_string(), vec![0u8; 8]),
        Some("old_module") => {
            let (meta, kern) = synthetic(FAKE_RIP);
            (meta.replace("1155", "1139"), kern)
        }
        _ => synthetic(FAKE_RIP),
    };
    let slow = fake.setting("checkpoint") == Some("slow".into());
    let written = std::fs::write(target, b"fake memory")
        .and_then(|()| std::fs::write(format!("{}.kern", target.display()), kern))
        .and_then(|()| {
            if slow {
                sleep(Duration::from_millis(150));
            }
            std::fs::write(format!("{}.meta", target.display()), meta)
        });
    i32::from(written.is_err())
}

/// `sudo -n <program> <args...>`: runs the program, with this process's environment, in place of itself.
pub fn sudo(fake: &Fake, args: &[String]) -> i32 {
    fake.log_call(&format!("sudo {}", args.get(1).map_or("", String::as_str)));
    let rest = match args.split_first() {
        Some((flag, rest)) if flag == "-n" => rest,
        _ => return 64,
    };
    let Some((program, program_args)) = rest.split_first() else {
        return 64;
    };
    if fake.sudo_mode().as_deref() == Some("ignore_term") {
        return sudo_that_ignores_term(program, program_args);
    }
    let error = Command::new(program).args(program_args).exec();
    eprintln!("could not run the program: {}", error.kind());
    127
}

extern "C" {
    fn signal(sig: i32, handler: usize) -> usize;
}

/// A `sudo` that stays up when asked to end, and runs the program as a child in a group of its own, as a root `sudo` does
/// to an unprivileged caller. Only SIGKILL ends it, and the program it started goes on.
fn sudo_that_ignores_term(program: &str, args: &[String]) -> i32 {
    const SIGTERM: i32 = 15;
    const SIG_IGN: usize = 1;
    // SAFETY: `signal` is given a signal number and the constant that means "ignore"; no memory is shared.
    unsafe {
        signal(SIGTERM, SIG_IGN);
    }
    let child = Command::new(program).args(args).process_group(0).spawn();
    match child.and_then(|mut c| c.wait()) {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 127,
    }
}

/// `cpuset -l <core> <program> <args...>`: runs the program in place of itself, as the real one does after pinning it.
pub fn cpuset(fake: &Fake, args: &[String]) -> i32 {
    let core = args.get(1).map_or("", String::as_str);
    fake.log_call(&format!("cpuset -l {core}"));
    match args {
        [flag, core, program, rest @ ..] if flag == "-l" && core.parse::<u32>().is_ok() => {
            let error = Command::new(program).args(rest).exec();
            eprintln!("could not run the program: {}", error.kind());
            127
        }
        _ => 64,
    }
}

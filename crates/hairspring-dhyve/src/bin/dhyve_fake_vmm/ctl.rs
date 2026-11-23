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

/// The restored pointer the fake's checkpoints decode to.
pub const FAKE_RIP: u64 = 0xffff_ffff_8012_3456;

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
    let (meta, kern) = if fake.setting("checkpoint") == Some("undecodable".into()) {
        ("{}".to_string(), vec![0u8; 8])
    } else {
        synthetic(FAKE_RIP)
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
    let error = Command::new(program).args(program_args).exec();
    eprintln!("could not run the program: {}", error.kind());
    127
}

//! Where the files of a prepare and of each trial live under the state directory.
//!
//! ```text
//! <state>/host.lock, stamp-counter      the host lock and the stamp counter (see hostlock)
//! <state>/ckpt/<stamp>/boot*            the checkpoint of one prepare
//! <state>/runs/<stamp>/<rNN>/...        one VM's files: control socket, logs, fw_cfg files, state files
//! ```
use crate::vmname::{Stamp, VmName};
use std::path::{Path, PathBuf};

/// The longest control socket path accepted: a Unix socket path must fit the platform's address, with room to spare.
pub const MAX_SOCKET_PATH_BYTES: usize = 100;
const CHECKPOINT_FILES: [&str; 3] = ["boot", "boot.kern", "boot.meta"];

/// The state directory.
#[derive(Debug, Clone)]
pub struct StateLayout {
    root: PathBuf,
}

/// The files of one VM run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPaths {
    pub dir: PathBuf,
    pub control_sock: PathBuf,
    pub uart_log: PathBuf,
    pub run_log: PathBuf,
    pub fwcfg_dir: PathBuf,
    pub state_manifest: PathBuf,
    pub state_regs: PathBuf,
    pub state_mem: PathBuf,
    pub state_devices: PathBuf,
}

impl StateLayout {
    pub fn new(root: &Path) -> StateLayout {
        StateLayout {
            root: root.to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn checkpoint_dir(&self, stamp: &Stamp) -> PathBuf {
        self.root.join("ckpt").join(stamp.as_str())
    }

    /// The three files a checkpoint is.
    pub fn checkpoint_files(&self, stamp: &Stamp) -> [PathBuf; 3] {
        let dir = self.checkpoint_dir(stamp);
        CHECKPOINT_FILES.map(|f| dir.join(f))
    }

    pub fn run_paths(&self, stamp: &Stamp, name: &VmName) -> RunPaths {
        let short = &name.as_str()[name.as_str().len() - 3..];
        let dir = self.root.join("runs").join(stamp.as_str()).join(short);
        RunPaths {
            control_sock: dir.join("ctl.sock"),
            uart_log: dir.join("uart.log"),
            run_log: dir.join("run.log"),
            fwcfg_dir: dir.join("fwcfg"),
            state_manifest: dir.join("state_manifest.json"),
            state_regs: dir.join("state_regs.bin"),
            state_mem: dir.join("state_mem.bin"),
            state_devices: dir.join("state_devices.bin"),
            dir,
        }
    }
}

impl RunPaths {
    /// Whether the control socket's path fits an address.
    pub fn socket_path_fits(&self) -> bool {
        self.control_sock.as_os_str().len() <= MAX_SOCKET_PATH_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_under_the_state_directory_by_stamp_and_run() {
        let layout = StateLayout::new(Path::new("/s"));
        let stamp = Stamp::parse("20261123T1530Z").unwrap();
        let name = VmName::new(&stamp, 4).unwrap();
        let run = layout.run_paths(&stamp, &name);
        assert_eq!(run.dir, Path::new("/s/runs/20261123T1530Z/r04"));
        assert_eq!(
            run.control_sock,
            Path::new("/s/runs/20261123T1530Z/r04/ctl.sock")
        );
        assert_eq!(run.uart_log, run.dir.join("uart.log"));
        assert_eq!(
            layout.checkpoint_files(&stamp)[2],
            Path::new("/s/ckpt/20261123T1530Z/boot.meta")
        );
        assert_eq!(layout.root(), Path::new("/s"));
    }

    #[test]
    fn a_socket_path_that_is_too_long_does_not_fit() {
        let stamp = Stamp::from_counter(1);
        let name = VmName::new(&stamp, 0).unwrap();
        let short = StateLayout::new(Path::new("/s")).run_paths(&stamp, &name);
        assert!(short.socket_path_fits());
        let long_root = format!("/{}", "d".repeat(120));
        let long = StateLayout::new(Path::new(&long_root)).run_paths(&stamp, &name);
        assert!(!long.socket_path_fits());
    }
}

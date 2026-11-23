//! The record of a checkpoint: the three files' hashes, the keys a restore must be given, and the instruction pointer the
//! restored CPU resumes at. The files themselves stay in the state directory.
use crate::ckptmeta;
use crate::vmname::Stamp;
use crate::wire::Message;
use hairspring_core::environment::sha256_file;
use std::path::{Path, PathBuf};

/// The values of the stop the checkpoint was taken at, which a restore must be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreKeys {
    pub stop_seq: u64,
    pub branch_count: u64,
    pub vtime_ns: u64,
    pub probe_id: u64,
    pub probe_arg0: u64,
    pub vcpu_id: u32,
}

impl RestoreKeys {
    pub fn from_event(event: &Message) -> RestoreKeys {
        RestoreKeys {
            stop_seq: event.stop_seq,
            branch_count: event.branch_count,
            vtime_ns: event.vtime_ns,
            probe_id: event.probe_id,
            probe_arg0: event.arg0,
            vcpu_id: event.vcpu_id,
        }
    }

    /// The `(option, value)` pairs a restore is started with, in order.
    pub fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("kind", "ready".to_string()),
            ("stop_seq", self.stop_seq.to_string()),
            ("branch_count", self.branch_count.to_string()),
            ("vtime_ns", self.vtime_ns.to_string()),
            ("probe_id", self.probe_id.to_string()),
            ("probe_arg0", self.probe_arg0.to_string()),
            ("vcpu_id", self.vcpu_id.to_string()),
        ]
    }
}

/// Why a checkpoint could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    Io(String),
    Rip(ckptmeta::MetaError),
    /// The checkpoint was not made by the expected module.
    Module(ckptmeta::MetaError),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::Io(m) => write!(f, "a checkpoint file could not be read: {m}"),
            RecordError::Rip(e) | RecordError::Module(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RecordError {}

/// What was checkpointed and what a restore needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointRecord {
    pub stamp: Stamp,
    pub dir: PathBuf,
    /// SHA-256 of `boot`, `boot.kern` and `boot.meta`, in that order.
    pub hashes: [String; 3],
    pub keys: RestoreKeys,
    pub rip: u64,
}

/// The largest `.kern` or `.meta` file read whole.
const MAX_SMALL_FILE_BYTES: u64 = 64 * 1024 * 1024;

fn read_small(path: &Path) -> Result<Vec<u8>, RecordError> {
    let io = |e: std::io::Error| RecordError::Io(e.kind().to_string());
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > MAX_SMALL_FILE_BYTES {
        return Err(RecordError::Io("too large".to_string()));
    }
    std::fs::read(path).map_err(io)
}

/// The SHA-256 of the three files of the checkpoint in `dir`.
pub fn hash_files(dir: &Path) -> Result<[String; 3], RecordError> {
    let one = |name: &str| {
        sha256_file(&dir.join(name)).map_err(|e| RecordError::Io(e.kind().to_string()))
    };
    Ok([one("boot")?, one("boot.kern")?, one("boot.meta")?])
}

impl CheckpointRecord {
    /// Reads the checkpoint in `dir`: hashes its files and decodes the restored instruction pointer.
    pub fn read(
        stamp: &Stamp,
        dir: &Path,
        keys: RestoreKeys,
    ) -> Result<CheckpointRecord, RecordError> {
        let kern = read_small(&dir.join("boot.kern"))?;
        let meta = read_small(&dir.join("boot.meta"))?;
        let meta = String::from_utf8_lossy(&meta);
        let rip = ckptmeta::restored_rip(&meta, &kern).map_err(RecordError::Rip)?;
        ckptmeta::check_module(&meta).map_err(RecordError::Module)?;
        Ok(CheckpointRecord {
            stamp: stamp.clone(),
            dir: dir.to_path_buf(),
            hashes: hash_files(dir)?,
            keys,
            rip,
        })
    }

    /// Whether the files on disk are still the ones recorded.
    pub fn unchanged(&self) -> bool {
        hash_files(&self.dir).is_ok_and(|now| now == self.hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hs-dhyve-ck-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn keys() -> RestoreKeys {
        RestoreKeys::from_event(&Message {
            kind: 1,
            stop_seq: 3,
            branch_count: 1000,
            vtime_ns: 5,
            probe_id: 0,
            arg0: 9,
            vcpu_id: 0,
            ..Message::default()
        })
    }

    fn write_checkpoint(dir: &Path, rip: u64) {
        let (meta, kern) = ckptmeta::synthetic(rip);
        std::fs::write(dir.join("boot"), b"memory").unwrap();
        std::fs::write(dir.join("boot.kern"), kern).unwrap();
        std::fs::write(dir.join("boot.meta"), meta).unwrap();
    }

    #[test]
    fn the_keys_come_from_the_event_in_the_order_a_restore_is_given_them() {
        let names: Vec<_> = keys().options().into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            names,
            [
                "kind",
                "stop_seq",
                "branch_count",
                "vtime_ns",
                "probe_id",
                "probe_arg0",
                "vcpu_id"
            ]
        );
        assert_eq!(keys().options()[1].1, "3");
        assert_eq!(keys().options()[5].1, "9");
    }

    #[test]
    fn a_record_hashes_the_files_and_decodes_the_pointer() {
        let dir = scratch("read");
        write_checkpoint(&dir, 0xffff_ffff_8012_3456);
        let stamp = Stamp::from_counter(1);
        let record = CheckpointRecord::read(&stamp, &dir, keys()).unwrap();
        assert_eq!(record.rip, 0xffff_ffff_8012_3456);
        assert_eq!(record.hashes[0].len(), 64);
        assert!(record.unchanged());
        std::fs::write(dir.join("boot"), b"changed").unwrap();
        assert!(!record.unchanged());
    }

    #[test]
    fn a_missing_file_or_an_undecodable_pointer_is_an_error() {
        let stamp = Stamp::from_counter(1);
        let dir = scratch("missing");
        let err = CheckpointRecord::read(&stamp, &dir, keys()).unwrap_err();
        assert!(matches!(err, RecordError::Io(_)));
        assert!(err.to_string().contains("could not be read"));
        write_checkpoint(&dir, 1);
        std::fs::write(dir.join("boot.meta"), b"{}").unwrap();
        let err = CheckpointRecord::read(&stamp, &dir, keys()).unwrap_err();
        assert!(matches!(err, RecordError::Rip(_)));
        assert!(err.to_string().contains("checkpoint"));
        std::fs::remove_file(dir.join("boot")).unwrap();
        assert!(hash_files(&dir).is_err());
    }
}

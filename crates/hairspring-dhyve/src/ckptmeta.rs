//! Reading a checkpoint's `.meta` file (JSON) and `.kern` file (the kernel-side device state) for the one value a restore
//! needs: the instruction pointer the checkpointed CPU resumes at. The `.meta` file lists entries (`device`, `size`,
//! `file_offset`) somewhere in its document; the `vm` entry of one CPU is 344 bytes, and bytes 200 to 207 of it are the
//! next instruction pointer, little endian.
use serde_json::Value;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

/// The size of the `vm` entry of a one-CPU checkpoint.
pub const VM_ENTRY_SIZE: u64 = 344;
/// Where the next instruction pointer is inside that entry.
const NEXTRIP_OFFSET: usize = 200;
const NEXTRIP_BYTES: usize = 8;
const VM_ENTRY: &str = "vm";
/// How deep the document is searched for entries. The JSON reader itself refuses a document nested deeper than 128, so this
/// is never the limit that matters; the reference decoder has none.
const MAX_DEPTH: usize = 128;
/// The size of the `vmcx` entry the expected module writes. Another size is another module.
pub const VMCX_ENTRY_SIZE: u64 = 1155;

/// Why the pointer could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaError {
    NotJson,
    NoEntries,
    /// No `vm` entry, or more than one.
    VmEntryMissing,
    VmEntryDuplicated,
    VmEntrySize(u64),
    OutsideKern,
    /// An entry whose device is not text or whose size or offset is not a whole number.
    BadEntry,
    /// The `vmcx` entry is missing or not the size of the expected module's.
    ModuleEntry(Option<u64>),
}

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetaError::NotJson => write!(f, "the checkpoint's description is not readable"),
            MetaError::NoEntries => write!(f, "the checkpoint's description lists no entries"),
            MetaError::VmEntryMissing => write!(f, "the checkpoint has no CPU entry"),
            MetaError::VmEntryDuplicated => write!(f, "the checkpoint has more than one CPU entry"),
            MetaError::VmEntrySize(n) => write!(
                f,
                "the checkpoint's CPU entry is {n} bytes, not {VM_ENTRY_SIZE}"
            ),
            MetaError::OutsideKern => {
                write!(f, "the checkpoint's CPU entry lies outside its state file")
            }
            MetaError::BadEntry => write!(
                f,
                "the checkpoint's description has an entry that is not well formed"
            ),
            MetaError::ModuleEntry(_) => {
                write!(f, "the checkpoint was not made by the expected module")
            }
        }
    }
}

impl std::error::Error for MetaError {}

fn number(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse().ok()
        }
        _ => None,
    }
}

/// An object that holds all three keys is an entry, and then its values must be well formed: the reference decoder refuses the
/// whole file for an entry with a device that is not text or a size or offset that is not a whole number.
fn collect(node: &Value, depth: usize, out: &mut Vec<(String, u64, u64)>) -> Result<(), MetaError> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    match node {
        Value::Object(map) => {
            if let (Some(device), Some(size), Some(offset)) =
                (map.get("device"), map.get("size"), map.get("file_offset"))
            {
                let (Value::String(device), Some(size), Some(offset)) =
                    (device, number(size), number(offset))
                else {
                    return Err(MetaError::BadEntry);
                };
                out.push((device.clone(), size, offset));
            }
            map.values().try_for_each(|v| collect(v, depth + 1, out))
        }
        Value::Array(items) => items.iter().try_for_each(|v| collect(v, depth + 1, out)),
        _ => Ok(()),
    }
}

/// The entries by device name as `(size, file_offset)`, and the names that occur more than once.
pub type Entries = (BTreeMap<String, (u64, u64)>, Vec<String>);

/// `device -> (size, file_offset)` for the entries of a `.meta` document, and the names that occur more than once.
pub fn entries(meta: &str) -> Result<Entries, MetaError> {
    let doc: Value = serde_json::from_str(meta).map_err(|_| MetaError::NotJson)?;
    let mut found = Vec::new();
    collect(&doc, 0, &mut found)?;
    if found.is_empty() {
        return Err(MetaError::NoEntries);
    }
    let mut map = BTreeMap::new();
    let mut duplicated = Vec::new();
    for (name, size, offset) in found {
        match map.entry(name.clone()) {
            Entry::Vacant(slot) => {
                slot.insert((size, offset));
            }
            Entry::Occupied(_) if !duplicated.contains(&name) => duplicated.push(name),
            Entry::Occupied(_) => {}
        }
    }
    Ok((map, duplicated))
}

/// The restored instruction pointer, from the `.meta` text and the `.kern` bytes.
pub fn restored_rip(meta: &str, kern: &[u8]) -> Result<u64, MetaError> {
    let (map, duplicated) = entries(meta)?;
    if duplicated.iter().any(|n| n == VM_ENTRY) {
        return Err(MetaError::VmEntryDuplicated);
    }
    let &(size, offset) = map.get(VM_ENTRY).ok_or(MetaError::VmEntryMissing)?;
    if size != VM_ENTRY_SIZE {
        return Err(MetaError::VmEntrySize(size));
    }
    let start = usize::try_from(offset)
        .ok()
        .and_then(|o| o.checked_add(NEXTRIP_OFFSET))
        .ok_or(MetaError::OutsideKern)?;
    let end = start
        .checked_add(NEXTRIP_BYTES)
        .ok_or(MetaError::OutsideKern)?;
    let entry_end =
        usize::try_from(offset.saturating_add(size)).map_err(|_| MetaError::OutsideKern)?;
    if entry_end > kern.len() {
        return Err(MetaError::OutsideKern);
    }
    let raw: [u8; NEXTRIP_BYTES] = kern
        .get(start..end)
        .and_then(|b| b.try_into().ok())
        .ok_or(MetaError::OutsideKern)?;
    Ok(u64::from_le_bytes(raw))
}

/// Whether the checkpoint was made by the expected module: exactly one `vmcx` entry, of the expected size.
pub fn check_module(meta: &str) -> Result<(), MetaError> {
    let (map, duplicated) = entries(meta)?;
    if duplicated.iter().any(|n| n == "vmcx") {
        return Err(MetaError::ModuleEntry(None));
    }
    match map.get("vmcx") {
        Some(&(VMCX_ENTRY_SIZE, _)) => Ok(()),
        Some(&(size, _)) => Err(MetaError::ModuleEntry(Some(size))),
        None => Err(MetaError::ModuleEntry(None)),
    }
}

/// A `.meta` text and `.kern` bytes that decode to `rip`, for tests and the fake.
pub fn synthetic(rip: u64) -> (String, Vec<u8>) {
    let lead = 16usize;
    let mut kern = vec![0u8; lead + VM_ENTRY_SIZE as usize + 8];
    kern[lead + NEXTRIP_OFFSET..lead + NEXTRIP_OFFSET + NEXTRIP_BYTES]
        .copy_from_slice(&rip.to_le_bytes());
    let meta = serde_json::json!({
        "basic metadata": {"memsize": "268435456"},
        "snapshot": [
            {"device": "vmcx", "size": 1155, "file_offset": 0},
            {"device": "vm", "size": VM_ENTRY_SIZE, "file_offset": lead},
        ]
    });
    (meta.to_string(), kern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_is_read_from_bytes_200_to_207_of_the_vm_entry() {
        let (meta, kern) = synthetic(0xffff_ffff_8012_3456);
        assert_eq!(restored_rip(&meta, &kern), Ok(0xffff_ffff_8012_3456));
    }

    #[test]
    fn entries_are_found_anywhere_in_the_document_and_numbers_may_be_strings() {
        let meta = r#"{"a":{"b":[{"device":"vm","size":"344","file_offset":"4"}]}}"#;
        let mut kern = vec![0u8; 4 + 344];
        kern[204..212].copy_from_slice(&7u64.to_le_bytes());
        assert_eq!(restored_rip(meta, &kern), Ok(7));
    }

    #[test]
    fn a_document_that_is_not_json_or_lists_nothing_is_refused() {
        assert_eq!(restored_rip("nope", &[]), Err(MetaError::NotJson));
        assert_eq!(restored_rip("{}", &[]), Err(MetaError::NoEntries));
        assert_eq!(
            restored_rip(r#"{"x":[1,2]}"#, &[]),
            Err(MetaError::NoEntries)
        );
    }

    #[test]
    fn a_missing_duplicated_or_wrong_sized_vm_entry_is_refused() {
        let missing = r#"{"e":[{"device":"vmcx","size":1,"file_offset":0}]}"#;
        assert_eq!(restored_rip(missing, &[]), Err(MetaError::VmEntryMissing));
        let twice = r#"{"e":[{"device":"vm","size":344,"file_offset":0},{"device":"vm","size":344,"file_offset":0}]}"#;
        assert_eq!(
            restored_rip(twice, &[0; 400]),
            Err(MetaError::VmEntryDuplicated)
        );
        let size = r#"{"e":[{"device":"vm","size":200,"file_offset":0}]}"#;
        assert_eq!(
            restored_rip(size, &[0; 400]),
            Err(MetaError::VmEntrySize(200))
        );
    }

    #[test]
    fn an_entry_outside_the_state_file_is_refused() {
        let (meta, kern) = synthetic(1);
        assert_eq!(
            restored_rip(&meta, &kern[..100]),
            Err(MetaError::OutsideKern)
        );
        let huge = r#"{"e":[{"device":"vm","size":344,"file_offset":18446744073709551615}]}"#;
        assert_eq!(restored_rip(huge, &[0; 400]), Err(MetaError::OutsideKern));
    }

    #[test]
    fn a_nested_entry_is_found_as_the_reference_finds_it_up_to_the_readers_own_limit() {
        let mut text = String::from(r#"{"device":"vm","size":344,"file_offset":0}"#);
        for _ in 0..40 {
            text = format!(r#"{{"x":{text}}}"#);
        }
        assert_eq!(restored_rip(&text, &[0; 400]), Ok(0));
        for _ in 0..200 {
            text = format!("[{text}]");
        }
        assert_eq!(restored_rip(&text, &[0; 400]), Err(MetaError::NotJson));
    }

    #[test]
    fn an_entry_that_is_not_well_formed_refuses_the_whole_file() {
        for bad in [
            r#"{"e":[{"device":5,"size":344,"file_offset":0}]}"#,
            r#"{"e":[{"device":"vm","size":-1,"file_offset":0}]}"#,
            r#"{"e":[{"device":"vm","size":344.5,"file_offset":0}]}"#,
            r#"{"e":[{"device":"vm","size":"+344","file_offset":0}]}"#,
            r#"{"e":[{"device":"vm","size":true,"file_offset":0}]}"#,
        ] {
            assert_eq!(
                restored_rip(bad, &[0; 400]),
                Err(MetaError::BadEntry),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_module_check_wants_one_vmcx_entry_of_the_expected_size() {
        let (meta, _) = synthetic(1);
        assert_eq!(check_module(&meta), Ok(()));
        let old = r#"{"e":[{"device":"vmcx","size":1139,"file_offset":0}]}"#;
        assert_eq!(check_module(old), Err(MetaError::ModuleEntry(Some(1139))));
        let none = r#"{"e":[{"device":"vm","size":344,"file_offset":0}]}"#;
        assert_eq!(check_module(none), Err(MetaError::ModuleEntry(None)));
        let twice = r#"{"e":[{"device":"vmcx","size":1155,"file_offset":0},{"device":"vmcx","size":1155,"file_offset":0}]}"#;
        assert_eq!(check_module(twice), Err(MetaError::ModuleEntry(None)));
    }

    #[test]
    fn errors_say_what_is_wrong() {
        for e in [
            MetaError::NotJson,
            MetaError::NoEntries,
            MetaError::VmEntryMissing,
            MetaError::VmEntryDuplicated,
            MetaError::VmEntrySize(3),
            MetaError::OutsideKern,
            MetaError::BadEntry,
            MetaError::ModuleEntry(None),
        ] {
            assert!(e.to_string().starts_with("the checkpoint"));
        }
    }
}

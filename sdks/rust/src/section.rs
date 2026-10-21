//! The call-site records, collected by the linker.
//!
//! Every call site places its record in one named section; the linker concatenates them, and the SDK reads the
//! whole section back as a slice.
//!
//! * ELF (Linux): the section is named `hs_rust_catalog_l1`, a valid C identifier, so GNU ld, lld and gold define
//!   `__start_hs_rust_catalog_l1` and `__stop_hs_rust_catalog_l1` for it.
//! * Mach-O (macOS): the section is `__DATA,__hs_rs_cat_l1`, and ld64 synthesises `section$start$...` and
//!   `section$end$...` for any section that exists.
//!
//! Both need the section to exist, so the SDK always plants one sentinel record (kind `None`) in it, and the reader
//! refers to the sentinel so the object that holds it is linked. The names differ from the C SDK's on purpose: the
//! record layouts differ, and a C and a Rust SDK linked into one executable must not read each other's records.

use crate::site::{Kind, Site};

/// The version of the `Site` layout. It is part of every section name (`..._l1`), so two copies of the crate with
/// different layouts never read each other's records. Bump it, and the names here and in `macros.rs`, together.
#[cfg(test)]
pub const LAYOUT: u32 = 1;

// The linker-defined boundaries are declared as bytes and cast: the records are not FFI types.
#[cfg(target_vendor = "apple")]
extern "C" {
    #[link_name = "\x01section$start$__DATA$__hs_rs_cat_l1"]
    static SECTION_START: u8;
    #[link_name = "\x01section$end$__DATA$__hs_rs_cat_l1"]
    static SECTION_STOP: u8;
}

#[cfg(not(target_vendor = "apple"))]
extern "C" {
    #[link_name = "__start_hs_rust_catalog_l1"]
    static SECTION_START: u8;
    #[link_name = "__stop_hs_rust_catalog_l1"]
    static SECTION_STOP: u8;
}

#[used]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__hs_rs_cat_l1")]
#[cfg_attr(not(target_vendor = "apple"), link_section = "hs_rust_catalog_l1")]
static SENTINEL: Site = Site::new("", "", "", 0, Kind::None);

/// The offline twin section must exist too, for tools that read a binary without running it.
#[used]
#[cfg_attr(target_vendor = "apple", link_section = "__DATA,__hs_rs_cstr_l1")]
#[cfg_attr(not(target_vendor = "apple"), link_section = "hs_rust_catstr_l1")]
static TWIN_SENTINEL: [u8; 1] = [0];

/// Every record in the section, the sentinel and any other non-live record included.
pub fn sites() -> &'static [Site] {
    std::hint::black_box(&SENTINEL);
    let start = std::ptr::addr_of!(SECTION_START).cast::<Site>();
    let stop = std::ptr::addr_of!(SECTION_STOP).cast::<Site>();
    let bytes = (stop as usize).saturating_sub(start as usize);
    // SAFETY: the linker placed `Site` records, each a multiple of its 64-byte alignment, back to back between the
    // two symbols; nothing else uses these sections.
    unsafe { std::slice::from_raw_parts(start, bytes / std::mem::size_of::<Site>()) }
}

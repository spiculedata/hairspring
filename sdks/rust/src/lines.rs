//! One function per protocol line kind. Each fills a buffer with the prefix, one JSON object and a newline; counters
//! sit in fixed-width fields so a line's length never depends on a counter's value.

use crate::fmt::{Buf, FILE_MAX, ID_MAX, MSG_MAX, THREAD_KEY_MAX};
use crate::site::Kind;

pub const SDK_NAME: &str = "rust";
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The protocol's per-process limit on ids; `end.sites` never claims more than a parser can accept.
pub const MAX_IDS: u64 = 4096;
/// Widest witness object: key 64 bytes, ord 20 digits, four 20-character values.
pub const WITNESS_WIDTH: usize = 197;

/// A line did not fit the buffer; the caller counts it as dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflow;

pub type Built = Result<(), Overflow>;

/// What the lines say about the process.
#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub tag: String,
    pub tag_stable: bool,
    pub signal_handlers: bool,
    pub catalog_hash: u64,
    pub n_ids: u64,
    pub dropped: u64,
}

pub struct CatalogRow<'a> {
    pub id: &'a str,
    pub kind: Kind,
    pub msg: &'a str,
    pub file: &'a str,
    pub line: u32,
}

/// A witness ready to print: the thread's key (NUL-terminated or not), its ordinal, four values.
pub struct WitnessOut<'a> {
    pub key: &'a [u8],
    pub ord: u64,
    pub vals: [i64; 4],
}

pub struct AssertRow<'a> {
    pub id: &'a str,
    pub kind: Kind,
    pub trues: u64,
    pub falses: u64,
    pub first_true: Option<WitnessOut<'a>>,
    pub first_false: Option<WitnessOut<'a>>,
}

pub struct ThreadRow<'a> {
    pub key: &'a [u8],
    pub evals: u64,
    pub tape_consumed: u64,
    pub is_stable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Exit,
    Signal,
    Explicit,
}

fn finish(b: &mut Buf) -> Built {
    b.put_str("}\n");
    if b.is_over() {
        Err(Overflow)
    } else {
        Ok(())
    }
}

fn open_line(b: &mut Buf, kind: &str) {
    b.put_str("HAIRSPRING_EVENT {\"v\":1,\"kind\":\"");
    b.put_str(kind);
    b.put_str("\"");
}

fn put_proc(b: &mut Buf, p: &ProcInfo) {
    b.put_str(",\"proc\":");
    b.put_ident(p.tag.as_bytes(), crate::fmt::TAG_MAX);
}

pub fn hello(b: &mut Buf, p: &ProcInfo) -> Built {
    open_line(b, "hello");
    b.put_str(",\"sdk\":\"");
    b.put_str(SDK_NAME);
    b.put_str("\",\"sdk_version\":\"");
    b.put_str(SDK_VERSION);
    b.put_str("\"");
    put_proc(b, p);
    b.put_str(",\"catalog_hash\":\"");
    b.put_hex64(p.catalog_hash);
    b.put_str(if p.signal_handlers {
        "\",\"signal_handlers\":true"
    } else {
        "\",\"signal_handlers\":false"
    });
    if !p.tag_stable {
        b.put_str(",\"tag_stable\":false"); // additive: a tag derived from a pid
    }
    finish(b)
}

/// The source path may be longer than the protocol allows; keep its tail, which names the file.
fn file_tail(file: &str) -> &[u8] {
    let bytes = file.as_bytes();
    &bytes[bytes.len().saturating_sub(FILE_MAX)..]
}

fn catalog_body(b: &mut Buf, p: &ProcInfo, row: &CatalogRow, with_text: bool) {
    open_line(b, "catalog");
    put_proc(b, p);
    b.put_str(",\"id\":");
    b.put_ident(row.id.as_bytes(), ID_MAX);
    b.put_str(",\"type\":\"");
    b.put_str(row.kind.name());
    b.put_str(if row.kind.must_hit() {
        "\",\"must_hit\":true"
    } else {
        "\",\"must_hit\":false"
    });
    if !with_text {
        return;
    }
    b.put_str(",\"msg\":");
    b.put_json_text(row.msg.as_bytes(), MSG_MAX);
    b.put_str(",\"loc\":{\"file\":");
    b.put_json_text(file_tail(row.file), FILE_MAX);
    b.put_str(",\"line\":");
    b.put_u64(u64::from(row.line));
    b.put_str("}");
}

pub fn catalog(b: &mut Buf, p: &ProcInfo, row: &CatalogRow) -> Built {
    catalog_body(b, p, row, true);
    if !b.is_over() {
        return finish(b);
    }
    // An oversized message or path must not cost the whole line: retry with the text left out.
    b.clear();
    catalog_body(b, p, row, false);
    finish(b)
}

/// A witness object padded to `WITNESS_WIDTH`, or `null` padded the same, so presence does not change length.
fn put_witness(b: &mut Buf, w: Option<&WitnessOut>) {
    let start = b.len();
    match w {
        Some(w) => {
            b.put_str("{\"thread\":");
            b.put_thread_key(w.key, THREAD_KEY_MAX);
            b.put_str(",\"ord\":");
            b.put_u64_w(w.ord);
            b.put_str(",\"vals\":[");
            for (i, v) in w.vals.iter().enumerate() {
                if i > 0 {
                    b.put_str(",");
                }
                b.put_i64(*v);
            }
            b.put_str("]}");
        }
        None => b.put_str("null"),
    }
    let used = b.len() - start;
    b.pad(WITNESS_WIDTH.saturating_sub(used));
}

pub fn assert_line(b: &mut Buf, p: &ProcInfo, row: &AssertRow) -> Built {
    open_line(b, "assert");
    put_proc(b, p);
    b.put_str(",\"id\":");
    b.put_ident(row.id.as_bytes(), ID_MAX);
    let falses = if row.kind.is_point() { 0 } else { row.falses };
    b.put_str(",\"hits\":");
    b.put_u64_w(row.trues.wrapping_add(falses));
    b.put_str(",\"trues\":");
    b.put_u64_w(row.trues);
    b.put_str(",\"falses\":");
    b.put_u64_w(falses);
    if !row.kind.is_point() {
        b.put_str(",\"first_false\":");
        put_witness(b, row.first_false.as_ref());
    }
    b.put_str(",\"first_true\":");
    put_witness(b, row.first_true.as_ref());
    finish(b)
}

pub fn work(b: &mut Buf, p: &ProcInfo, ops: u64) -> Built {
    open_line(b, "work");
    put_proc(b, p);
    b.put_str(",\"ops\":");
    b.put_u64_w(ops);
    finish(b)
}

pub fn thread(b: &mut Buf, p: &ProcInfo, row: &ThreadRow) -> Built {
    open_line(b, "thread");
    put_proc(b, p);
    b.put_str(",\"thread\":");
    b.put_thread_key_padded(row.key);
    b.put_str(",\"evals\":");
    b.put_u64_w(row.evals);
    b.put_str(",\"tape_consumed\":");
    b.put_u64_w(row.tape_consumed);
    b.put_str(if row.is_stable {
        ",\"key_stable\":true "
    } else {
        ",\"key_stable\":false"
    });
    finish(b)
}

pub fn end(
    b: &mut Buf,
    p: &ProcInfo,
    reason: EndReason,
    status: i64,
    evals: u64,
    threads: u64,
) -> Built {
    open_line(b, "end");
    put_proc(b, p);
    b.put_str(",\"reason\":\"");
    b.put_str(match reason {
        EndReason::Exit => "exit",
        EndReason::Signal => "signal",
        EndReason::Explicit => "explicit",
    });
    b.put_str("\",\"status\":");
    b.put_i64_w(status);
    b.put_str(",\"sites\":");
    b.put_u64(p.n_ids.min(MAX_IDS)); // what a parser can accept
    b.put_str(",\"evals\":");
    b.put_u64_w(evals);
    b.put_str(",\"dropped\":");
    b.put_u64_w(p.dropped);
    b.put_str(",\"threads\":");
    b.put_u64(threads);
    finish(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc_info() -> ProcInfo {
        ProcInfo {
            tag: "p1".to_string(),
            tag_stable: true,
            signal_handlers: false,
            catalog_hash: 0xfb6c_f21a_75aa_d6ef,
            n_ids: 7,
            dropped: 0,
        }
    }

    fn text(b: &Buf) -> String {
        String::from_utf8(b.as_bytes().to_vec()).unwrap()
    }

    fn w(n: u64) -> String {
        format!("{n:>20}")
    }

    fn wit(s: &str) -> String {
        format!("{s:<WITNESS_WIDTH$}")
    }

    #[test]
    fn hello_names_the_sdk_and_the_catalog_hash() {
        let mut b = Buf::new();
        hello(&mut b, &proc_info()).unwrap();
        assert_eq!(
            text(&b),
            "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\",\"sdk\":\"rust\",\"sdk_version\":\"0.1.0\",\"proc\":\"p1\",\"catalog_hash\":\"fb6cf21a75aad6ef\",\"signal_handlers\":false}\n"
        );
    }

    #[test]
    fn hello_flags_an_unstable_tag_and_installed_handlers() {
        let mut p = proc_info();
        p.tag_stable = false;
        p.signal_handlers = true;
        let mut b = Buf::new();
        hello(&mut b, &p).unwrap();
        assert!(text(&b).ends_with("\"signal_handlers\":true,\"tag_stable\":false}\n"));
    }

    #[test]
    fn catalog_carries_text_and_location() {
        let row = CatalogRow {
            id: "basic.always.ok",
            kind: Kind::Always,
            msg: "always holds",
            file: "tests/fixtures/hs_sample.c",
            line: 31,
        };
        let mut b = Buf::new();
        catalog(&mut b, &proc_info(), &row).unwrap();
        assert_eq!(
            text(&b),
            "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"basic.always.ok\",\"type\":\"always\",\"must_hit\":true,\"msg\":\"always holds\",\"loc\":{\"file\":\"tests/fixtures/hs_sample.c\",\"line\":31}}\n"
        );
    }

    #[test]
    fn catalog_must_hit_is_false_for_unreachable() {
        let row = CatalogRow {
            id: "x",
            kind: Kind::Unreachable,
            msg: "",
            file: "f.rs",
            line: 1,
        };
        let mut b = Buf::new();
        catalog(&mut b, &proc_info(), &row).unwrap();
        assert!(text(&b).contains("\"type\":\"unreachable\",\"must_hit\":false"));
    }

    #[test]
    fn catalog_keeps_the_tail_of_a_long_path() {
        let long = format!("{}/tail.rs", "d".repeat(400));
        let row = CatalogRow {
            id: "x",
            kind: Kind::Always,
            msg: "",
            file: &long,
            line: 1,
        };
        let mut b = Buf::new();
        catalog(&mut b, &proc_info(), &row).unwrap();
        let s = text(&b);
        assert!(s.contains("/tail.rs\""));
        assert!(!s.contains(&"d".repeat(300)));
    }

    #[test]
    fn assert_for_a_condition_pads_both_witnesses() {
        let row = AssertRow {
            id: "basic.sometimes",
            kind: Kind::Sometimes,
            trues: 2,
            falses: 2,
            first_true: Some(WitnessOut {
                key: b"t#1",
                ord: 7,
                vals: [0, 0, 0, 0],
            }),
            first_false: Some(WitnessOut {
                key: b"t#1",
                ord: 8,
                vals: [1, 0, 0, 0],
            }),
        };
        let mut b = Buf::new();
        assert_line(&mut b, &proc_info(), &row).unwrap();
        let expected = format!(
            "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"basic.sometimes\",\"hits\":{},\"trues\":{},\"falses\":{},\"first_false\":{},\"first_true\":{}}}\n",
            w(4),
            w(2),
            w(2),
            wit(&format!("{{\"thread\":\"t#1\",\"ord\":{},\"vals\":[1,0,0,0]}}", w(8))),
            wit(&format!("{{\"thread\":\"t#1\",\"ord\":{},\"vals\":[0,0,0,0]}}", w(7))),
        );
        assert_eq!(text(&b), expected);
    }

    #[test]
    fn a_missing_witness_is_null_padded_to_the_same_width() {
        let row = AssertRow {
            id: "a",
            kind: Kind::Always,
            trues: 5,
            falses: 0,
            first_true: Some(WitnessOut {
                key: b"t#1",
                ord: 1,
                vals: [0, 2, 0, 0],
            }),
            first_false: None,
        };
        let mut b = Buf::new();
        assert_line(&mut b, &proc_info(), &row).unwrap();
        let s = text(&b);
        assert!(s.contains(&format!("\"first_false\":{},\"first_true\":", wit("null"))));
        // The length does not depend on whether a failure happened.
        let both = AssertRow {
            first_false: Some(WitnessOut {
                key: b"t#1",
                ord: 99,
                vals: [-1, -2, -3, -4],
            }),
            falses: 1,
            ..row
        };
        let mut b2 = Buf::new();
        assert_line(&mut b2, &proc_info(), &both).unwrap();
        assert_eq!(b.len(), b2.len());
    }

    #[test]
    fn a_point_kind_sends_only_the_true_side() {
        let row = AssertRow {
            id: "basic.reach.hit",
            kind: Kind::Reachable,
            trues: 1,
            falses: 9,
            first_true: Some(WitnessOut {
                key: b"t#1",
                ord: 11,
                vals: [0, 0, 0, 0],
            }),
            first_false: None,
        };
        let mut b = Buf::new();
        assert_line(&mut b, &proc_info(), &row).unwrap();
        let s = text(&b);
        assert!(!s.contains("first_false"));
        assert!(s.contains(&format!(
            "\"hits\":{},\"trues\":{},\"falses\":{}",
            w(1),
            w(1),
            w(0)
        )));
    }

    #[test]
    fn work_and_end_use_fixed_fields() {
        let mut b = Buf::new();
        work(&mut b, &proc_info(), 300).unwrap();
        assert_eq!(
            text(&b),
            format!(
                "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"work\",\"proc\":\"p1\",\"ops\":{}}}\n",
                w(300)
            )
        );
        b.clear();
        end(&mut b, &proc_info(), EndReason::Exit, 0, 11, 1).unwrap();
        assert_eq!(
            text(&b),
            format!(
                "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\",\"status\":{},\"sites\":7,\"evals\":{},\"dropped\":{},\"threads\":1}}\n",
                w(0),
                w(11),
                w(0)
            )
        );
    }

    #[test]
    fn end_caps_sites_at_what_a_parser_accepts_and_names_the_reason() {
        let mut p = proc_info();
        p.n_ids = 9000;
        let mut b = Buf::new();
        end(&mut b, &p, EndReason::Signal, 11, 0, 0).unwrap();
        let s = text(&b);
        assert!(s.contains("\"reason\":\"signal\""));
        assert!(s.contains("\"sites\":4096,"));
        b.clear();
        end(&mut b, &p, EndReason::Explicit, -3, 0, 0).unwrap();
        assert!(text(&b).contains(&format!("\"reason\":\"explicit\",\"status\":{:>20}", -3)));
    }

    #[test]
    fn thread_pads_the_key_and_the_stability_flag() {
        let mut b = Buf::new();
        thread(
            &mut b,
            &proc_info(),
            &ThreadRow {
                key: b"t#1",
                evals: 11,
                tape_consumed: 0,
                is_stable: false,
            },
        )
        .unwrap();
        assert_eq!(
            text(&b),
            format!(
                "HAIRSPRING_EVENT {{\"v\":1,\"kind\":\"thread\",\"proc\":\"p1\",\"thread\":\"t#1\"{},\"evals\":{},\"tape_consumed\":{},\"key_stable\":false}}\n",
                " ".repeat(THREAD_KEY_MAX - 3),
                w(11),
                w(0)
            )
        );
        b.clear();
        thread(
            &mut b,
            &proc_info(),
            &ThreadRow {
                key: b"w-1",
                evals: 1,
                tape_consumed: 0,
                is_stable: true,
            },
        )
        .unwrap();
        assert!(text(&b).ends_with(",\"key_stable\":true }\n"));
    }

    #[test]
    fn a_line_that_does_not_fit_is_reported_not_truncated() {
        let long_id = "i".repeat(ID_MAX);
        let msg = "m".repeat(MSG_MAX);
        let file = "f".repeat(FILE_MAX);
        let row = CatalogRow {
            id: &long_id,
            kind: Kind::Always,
            msg: &msg,
            file: &file,
            line: 1,
        };
        let mut b = Buf::new();
        assert!(catalog(&mut b, &proc_info(), &row).is_ok());
        let mut tiny = proc_info();
        tiny.tag = "t".repeat(64);
        let mut b2 = Buf::new();
        b2.pad(crate::fmt::LINE_MAX - 10);
        assert_eq!(hello(&mut b2, &tiny), Err(Overflow));
    }
}

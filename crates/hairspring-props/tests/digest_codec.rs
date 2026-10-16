//! The digest wire format: round trips, canonical form, and one hostile input per rule.
use hairspring_props::limits::*;
use hairspring_props::*;

fn entry(idx: u32) -> DigestEntry {
    DigestEntry {
        idx,
        kind: AssertKind::Always,
        catalogued: true,
        conflicted: false,
        hits: 3,
        trues: 2,
        falses: 1,
        witness: Some(Witness {
            proc: "p1".into(),
            thread: "w-0".into(),
            ord: 9,
            vals: [1, -2, 3, i64::MIN],
        }),
        decl: None,
    }
}

/// A sometimes entry that passed: it may have a declaration but no witness.
fn passing(idx: u32) -> DigestEntry {
    DigestEntry {
        kind: AssertKind::Sometimes,
        trues: 3,
        falses: 0,
        witness: None,
        ..entry(idx)
    }
}

fn digest(entries: Vec<DigestEntry>) -> TrialDigest {
    TrialDigest {
        flags: DigestFlags {
            inconclusive: false,
            degraded: true,
            perturbing: false,
            report_missing: true,
            has_conflicts: false,
        },
        ids_dropped: 2,
        entries,
    }
}

fn rejected(d: &TrialDigest) -> bool {
    TrialDigest::decode(&d.encode()).is_err()
}

#[test]
fn a_digest_round_trips_and_its_encoding_is_canonical() {
    let mut with_decl = passing(7);
    with_decl.decl = Some(Decl {
        msg: "m".into(),
        loc: Some(Loc {
            file: "f.c".into(),
            line: 4,
            func: Some("fn".into()),
        }),
    });
    let conflict_only = DigestEntry {
        idx: 9,
        kind: AssertKind::Reachable,
        catalogued: false,
        conflicted: true,
        hits: 0,
        trues: 0,
        falses: 0,
        witness: None,
        decl: None,
    };
    let d = digest(vec![entry(300), with_decl, conflict_only]);
    let bytes = d.encode();
    let back = TrialDigest::decode(&bytes).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.encode(), bytes);
}

#[test]
fn entries_keep_the_order_they_were_given() {
    let d = digest(vec![entry(9), entry(2), entry(5)]);
    let back = TrialDigest::decode(&d.encode()).unwrap();
    let order: Vec<_> = back.entries.iter().map(|e| e.idx).collect();
    assert_eq!(order, vec![9, 2, 5]);
}

#[test]
fn an_empty_digest_is_a_few_bytes() {
    let d = TrialDigest::default();
    assert!(d.encode().len() <= 4);
    assert_eq!(TrialDigest::decode(&d.encode()).unwrap(), d);
}

#[test]
fn an_unknown_version_is_refused() {
    let mut bytes = digest(vec![entry(1)]).encode();
    bytes[0] = 2;
    assert!(TrialDigest::decode(&bytes).is_err());
    bytes[0] = 0;
    assert!(TrialDigest::decode(&bytes).is_err());
}

#[test]
fn trailing_bytes_and_every_truncation_are_refused_without_panicking() {
    let bytes = digest(vec![entry(1), entry(2)]).encode();
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(TrialDigest::decode(&longer).is_err());
    for cut in 0..bytes.len() {
        assert!(TrialDigest::decode(&bytes[..cut]).is_err(), "cut {cut}");
    }
}

#[test]
fn reserved_flag_bits_and_unknown_kinds_are_refused() {
    let mut bytes = digest(vec![entry(1)]).encode();
    bytes[1] |= 0x80;
    assert!(TrialDigest::decode(&bytes).is_err());
    let mut bytes = digest(vec![entry(1)]).encode();
    // version, flags, ids_dropped (1 byte), count (1 byte), idx (1 byte), then the kind byte.
    bytes[5] = (bytes[5] & !0b111) | 6;
    assert!(TrialDigest::decode(&bytes).is_err());
}

#[test]
fn counts_that_do_not_add_up_are_refused() {
    let mut e = entry(1);
    e.hits = 4;
    assert!(rejected(&digest(vec![e])));
    let mut e = entry(1);
    e.hits = u64::MAX;
    e.trues = u64::MAX;
    e.falses = 1;
    assert!(rejected(&digest(vec![e])));
}

#[test]
fn a_duplicate_index_is_refused() {
    assert!(rejected(&digest(vec![entry(5), entry(5)])));
}

#[test]
fn more_witnesses_than_the_limit_are_refused() {
    let many: Vec<_> = (0..=MAX_DIGEST_WITNESSES as u32).map(entry).collect();
    assert_eq!(many.len(), MAX_DIGEST_WITNESSES + 1);
    assert!(rejected(&digest(many)));
    let ok: Vec<_> = (0..MAX_DIGEST_WITNESSES as u32).map(entry).collect();
    assert!(!rejected(&digest(ok)));
}

#[test]
fn more_declarations_than_the_limit_are_refused() {
    let with_decl = |i| DigestEntry {
        decl: Some(Decl {
            msg: String::new(),
            loc: None,
        }),
        ..passing(i)
    };
    let many: Vec<_> = (0..=MAX_DIGEST_DECLS as u32).map(with_decl).collect();
    assert!(rejected(&digest(many)));
}

#[test]
fn a_witness_on_an_entry_that_cannot_have_one_is_refused() {
    let mut not_failing = passing(1);
    not_failing.witness = entry(1).witness;
    assert!(
        rejected(&digest(vec![not_failing])),
        "a passing id has no witness"
    );
    let mut conflict_only = entry(2);
    conflict_only.catalogued = false;
    conflict_only.conflicted = true;
    assert!(
        rejected(&digest(vec![conflict_only])),
        "nothing was counted"
    );
}

#[test]
fn an_entry_that_says_nothing_is_refused() {
    let useless = DigestEntry {
        catalogued: false,
        conflicted: false,
        hits: 0,
        trues: 0,
        falses: 0,
        witness: None,
        decl: None,
        ..entry(3)
    };
    assert!(rejected(&digest(vec![useless])));
}

#[test]
fn hostile_witness_strings_are_refused() {
    let with_witness = |w: Witness| DigestEntry {
        witness: Some(w),
        ..entry(1)
    };
    let good = Witness {
        proc: "p1".into(),
        thread: "w-0".into(),
        ord: 1,
        vals: [0; 4],
    };
    let long_thread = "t".repeat(MAX_THREAD_KEY_BYTES + 1);
    for t in ["", "a b", "w\u{202e}x", long_thread.as_str(), "x\n"] {
        let w = Witness {
            thread: t.to_string(),
            ..good.clone()
        };
        assert!(rejected(&digest(vec![with_witness(w)])), "thread {t:?}");
    }
    let long_proc = "p".repeat(MAX_PROC_BYTES + 1);
    for p in ["", "p 1", long_proc.as_str()] {
        let w = Witness {
            proc: p.to_string(),
            ..good.clone()
        };
        assert!(rejected(&digest(vec![with_witness(w)])), "proc {p:?}");
    }
}

#[test]
fn hostile_declaration_text_is_refused() {
    let decl = |msg: &str, file: &str| DigestEntry {
        decl: Some(Decl {
            msg: msg.to_string(),
            loc: Some(Loc {
                file: file.to_string(),
                line: 1,
                func: None,
            }),
        }),
        ..passing(1)
    };
    let long_msg = "m".repeat(MAX_MSG_BYTES + 1);
    let long_file = "f".repeat(MAX_LOC_FILE_BYTES + 1);
    for (msg, file) in [
        ("bidi \u{202e} text", "f.c"),
        ("line\nbreak", "f.c"),
        ("ansi \u{1b}[31m", "f.c"),
        (long_msg.as_str(), "f.c"),
        ("ok", long_file.as_str()),
        ("ok", "tab\tfile"),
    ] {
        assert!(rejected(&digest(vec![decl(msg, file)])), "{msg:?} {file:?}");
    }
    assert!(!rejected(&digest(vec![decl("fine message", "src/f.c")])));
}

#[test]
fn more_entries_than_a_run_may_track_are_refused() {
    let entries: Vec<_> = (0..=MAX_IDS_PER_RUN as u32).map(passing).collect();
    assert!(rejected(&digest(entries)));
}

#[test]
fn invalid_utf8_and_overlong_varints_are_refused() {
    let mut bytes = digest(vec![entry(1)]).encode();
    // Replace the single-byte ids_dropped (value 2) by the overlong two-byte form of 2.
    assert_eq!(bytes[2], 2);
    bytes.splice(2..3, [0x82, 0x00]);
    assert!(TrialDigest::decode(&bytes).is_err());
    let mut bytes = digest(vec![entry(1)]).encode();
    let pos = bytes.windows(3).position(|w| w == b"w-0").unwrap();
    bytes[pos] = 0xff;
    assert!(TrialDigest::decode(&bytes).is_err());
}

#[test]
fn a_digest_over_the_size_cap_is_refused_before_it_is_read() {
    let big = vec![0u8; MAX_DIGEST_BYTES + 1];
    assert!(TrialDigest::decode(&big).is_err());
}

#[test]
fn a_fifty_id_catalog_costs_a_few_hundred_bytes() {
    let entries: Vec<_> = (0..50).map(passing).collect();
    let len = digest(entries).encode().len();
    assert!(len < 400, "{len} bytes");
}

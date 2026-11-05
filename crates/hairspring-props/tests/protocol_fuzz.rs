//! Generated-input tests for everything that reads untrusted guest bytes: the line parser, the trial fold, the digest
//! decoder, the run catalog and the run summary. Inputs come from a seeded generator that leans on awkward values
//! (limit boundaries, 64-bit extremes, `1e999`, control and bidirectional characters, duplicate keys, deep nesting,
//! invalid UTF-8, lines with no newline, writes that interleave). Every case is reproducible from its seed.
//!
//! The default case counts keep the file to a few seconds. Set `HAIRSPRING_FUZZ_CASES` to run that many cases per
//! property instead (see docs/testing.md); a failing assertion prints the seed and case number.

use hairspring_props::limits::*;
use hairspring_props::valid::{is_ident, is_plain_text, is_thread_key};
use hairspring_props::*;
use std::collections::HashSet;

mod common;

use common::fuzz_rng::{base_seed, cases, Rng};

// ---------------------------------------------------------------------------------------------------------------
// Value pools

const AWKWARD_NUMBERS: &[&str] = &[
    "0",
    "1",
    "-1",
    "255",
    "256",
    "4294967295",
    "4294967296",
    "9223372036854775807",
    "9223372036854775808",
    "-9223372036854775808",
    "-9223372036854775809",
    "18446744073709551615",
    "18446744073709551616",
    "1e999",
    "-1e999",
    "1e0",
    "1.0",
    "1.5",
    "-0",
    "-0.0",
    "0.1e1",
    "00",
    "+1",
    "1.",
    ".5",
    "0x10",
    "NaN",
    "Infinity",
    "-Infinity",
    "null",
    "true",
    "false",
    "\"1\"",
    "[]",
    "[1]",
    "{}",
    "1e",
    "99999999999999999999999999999999999999",
];

const AWKWARD_STRINGS: &[&str] = &[
    "",
    " ",
    "a",
    "p1",
    "a.b/c:d_e-f",
    "has space",
    "tab\\there",
    "new\\nline",
    "nul\\u0000byte",
    "esc\\u001b[31mred",
    "del\\u007f",
    "c1\\u0085next",
    "bidi\\u202eevil\\u202c",
    "isolate\\u2066x\\u2069",
    "zwsp\\u200bhidden",
    "bom\\ufeff",
    "linesep\\u2028x",
    "para\\u2029x",
    "tag\\udb40\\udc41",
    "lone\\ud800surrogate",
    "lowlone\\udc00",
    "caf\\u00e9",
    "\\ud83d\\ude00",
    "quote\\\"inside",
    "back\\\\slash",
    "t#1",
    "w-0",
    "_eval",
    "HAIRSPRING_EVENT {}",
];

fn long_ident(len: usize) -> String {
    "x".repeat(len)
}

/// A string value (already JSON-quoted) near the boundaries of the limits.
fn string_value(rng: &mut Rng, limit: usize) -> String {
    match rng.below(8) {
        0 => format!("\"{}\"", long_ident(limit)),
        1 => format!("\"{}\"", long_ident(limit + 1)),
        2 => format!("\"{}\"", long_ident(limit.saturating_sub(1))),
        3 => format!("\"{}\"", long_ident(rng.below(limit * 4 + 1))),
        _ => format!("\"{}\"", rng.pick(AWKWARD_STRINGS)),
    }
}

fn number_value(rng: &mut Rng) -> String {
    if rng.chance(3) {
        rng.next().to_string()
    } else {
        (*rng.pick(AWKWARD_NUMBERS)).to_string()
    }
}

fn kind_name(rng: &mut Rng, hostile: bool) -> String {
    if !hostile {
        return format!("\"{}\"", rng.pick(&AssertKind::ALL).as_str());
    }
    match rng.below(8) {
        0 => "\"nonsense\"".into(),
        1 => "\"ALWAYS\"".into(),
        2 => "5".into(),
        _ => format!("\"{}\"", rng.pick(&AssertKind::ALL).as_str()),
    }
}

/// The `must_hit` a well-formed catalog line carries for the quoted kind name.
fn must_hit_for(quoted_kind: &str) -> String {
    let name = quoted_kind.trim_matches('"');
    AssertKind::from_wire(name)
        .map_or("false", |k| if k.must_hit() { "true" } else { "false" })
        .into()
}

fn witness_value(rng: &mut Rng) -> String {
    match rng.below(6) {
        0 => "null".into(),
        1 => "[]".into(),
        2 => format!(
            r#"{{"thread":{},"ord":{},"vals":[{},{},{},{}]}}"#,
            string_value(rng, MAX_THREAD_KEY_BYTES),
            number_value(rng),
            number_value(rng),
            number_value(rng),
            number_value(rng),
            number_value(rng)
        ),
        3 => format!(
            r#"{{"thread":"w-0","ord":1,"vals":[{}]}}"#,
            (0..rng.below(7))
                .map(|_| number_value(rng))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => format!(
            r#"{{"thread":"w-0","ord":{},"vals":[1,-2,3,{}]}}"#,
            rng.below(1000),
            rng.next() as i64
        ),
    }
}

fn loc_value(rng: &mut Rng) -> String {
    match rng.below(5) {
        0 => "null".into(),
        1 => "7".into(),
        2 => format!(
            r#"{{"file":{},"line":{},"func":{}}}"#,
            string_value(rng, MAX_LOC_FILE_BYTES),
            number_value(rng),
            string_value(rng, MAX_LOC_FUNC_BYTES)
        ),
        3 => format!(r#"{{"file":"f.c","line":{}}}"#, number_value(rng)),
        _ => r#"{"file":"f.c","line":7,"func":"fn"}"#.into(),
    }
}

/// The fields of one event of a random kind, each either well formed or awkward.
fn event_fields(rng: &mut Rng) -> Vec<(String, String)> {
    let mut f: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| f.push((k.to_string(), v));
    let hostile = rng.chance(3);
    let awkward = move |rng: &mut Rng| hostile && rng.chance(3);
    let version = if awkward(rng) {
        number_value(rng)
    } else {
        "1".into()
    };
    put("v", version);
    let proc_value = |rng: &mut Rng| {
        if awkward(rng) {
            string_value(rng, MAX_PROC_BYTES)
        } else {
            "\"p1\"".into()
        }
    };
    let id_value = |rng: &mut Rng| {
        if awkward(rng) {
            string_value(rng, MAX_ID_BYTES)
        } else {
            format!("\"id{}\"", rng.below(6))
        }
    };
    let kind = if hostile { rng.below(8) } else { rng.below(7) };
    let kind_text = [
        "hello", "catalog", "assert", "work", "thread", "end", "eval", "mystery",
    ][kind];
    put("kind", format!("\"{kind_text}\""));
    put("proc", proc_value(rng));
    match kind {
        0 => {
            let text = |rng: &mut Rng, limit| {
                if hostile {
                    string_value(rng, limit)
                } else {
                    "\"c\"".into()
                }
            };
            put("sdk", text(rng, MAX_SDK_STR_BYTES));
            put("sdk_version", text(rng, MAX_SDK_STR_BYTES));
            put("catalog_hash", text(rng, MAX_HASH_BYTES));
            put(
                "signal_handlers",
                if awkward(rng) {
                    number_value(rng)
                } else {
                    "false".into()
                },
            );
        }
        1 => {
            put("id", id_value(rng));
            let type_text = kind_name(rng, hostile);
            put("type", type_text.clone());
            put(
                "must_hit",
                if hostile {
                    ["true", "false", "1", "null"][rng.below(4)].into()
                } else {
                    must_hit_for(&type_text)
                },
            );
            put(
                "msg",
                if hostile {
                    string_value(rng, MAX_MSG_BYTES)
                } else {
                    "\"m\"".into()
                },
            );
            put(
                "loc",
                if hostile {
                    loc_value(rng)
                } else {
                    "null".into()
                },
            );
        }
        2 => {
            put("id", id_value(rng));
            let (t, fl) = (rng.below(5) as u64, rng.below(5) as u64);
            let (hits, trues, falses) = if awkward(rng) {
                (number_value(rng), number_value(rng), number_value(rng))
            } else {
                ((t + fl).to_string(), t.to_string(), fl.to_string())
            };
            put("hits", hits);
            put("trues", trues);
            put("falses", falses);
            let (ff, ft) = if hostile {
                (witness_value(rng), witness_value(rng))
            } else {
                ("null".into(), "null".into())
            };
            put("first_false", ff);
            put("first_true", ft);
        }
        3 => put("ops", number_value(rng)),
        4 => {
            put("thread", string_value(rng, MAX_THREAD_KEY_BYTES));
            put("evals", number_value(rng));
            put("tape_consumed", number_value(rng));
            put("key_stable", ["true", "false", "0"][rng.below(3)].into());
        }
        5 => {
            put(
                "reason",
                ["\"exit\"", "\"signal\"", "\"explicit\"", "\"other\"", "3"][rng.below(5)].into(),
            );
            put("status", number_value(rng));
            put("sites", number_value(rng));
            put("evals", number_value(rng));
            put("dropped", number_value(rng));
            put("threads", number_value(rng));
        }
        6 => {
            put("id", id_value(rng));
            put("type", kind_name(rng, hostile));
            put(
                "ok",
                if hostile {
                    ["true", "false", "1", "\"yes\""][rng.below(4)].into()
                } else {
                    ["true", "false"][rng.below(2)].into()
                },
            );
        }
        _ => put("payload", number_value(rng)),
    }
    f
}

/// A nested value of the given depth, for unknown fields.
fn nested(depth: usize, object: bool) -> String {
    let (open, close) = if object { ("{\"k\":", "}") } else { ("[", "]") };
    let mut s = String::new();
    for _ in 0..depth {
        s.push_str(open);
    }
    s.push('0');
    for _ in 0..depth {
        s.push_str(close);
    }
    s
}

/// One event body (the JSON after the prefix), possibly corrupted.
fn event_body(rng: &mut Rng) -> String {
    let mut fields = event_fields(rng);
    if rng.chance(12) {
        let (k, v) = rng.pick(&fields).clone();
        let at = rng.below(fields.len() + 1);
        fields.insert(at, (k, v)); // duplicate key, possibly with a different position
    }
    if rng.chance(6) {
        fields.push(("extra".into(), nested(rng.below(8), rng.chance(2))));
    }
    if rng.chance(8) {
        fields.push(("extra".into(), number_value(rng)));
    }
    if rng.chance(10) {
        let at = rng.below(fields.len());
        fields.remove(at);
    }
    let mut body = String::from("{");
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            body.push(',');
        }
        body.push_str(&format!("\"{k}\":{v}"));
    }
    body.push('}');
    body
}

fn event_line(rng: &mut Rng) -> Vec<u8> {
    let mut line = match rng.below(40) {
        0 => b"HAIRSPRING_EVENT".to_vec(),
        1 => b"HAIRSPRING_EVENT\t".to_vec(),
        2 => b"HAIRSPRING_EVENT{".to_vec(),
        3 => b"HAIRSPRING_EVENT  ".to_vec(),
        _ => EVENT_PREFIX.as_bytes().to_vec(),
    };
    line.extend_from_slice(event_body(rng).as_bytes());
    if rng.chance(4) {
        corrupt(rng, &mut line);
    }
    line
}

/// Byte-level damage: flips, cuts, inserted invalid UTF-8, doubled tails, a stray carriage return.
fn corrupt(rng: &mut Rng, line: &mut Vec<u8>) {
    while rng.chance(4) && !line.is_empty() {
        let at = rng.below(line.len());
        match rng.below(8) {
            0 => line[at] = rng.next() as u8,
            1 => {
                line.remove(at);
            }
            2 => line.insert(at, rng.next() as u8),
            3 => line.truncate(at),
            4 => {
                let bad: &[&[u8]] = &[
                    b"\xff",
                    b"\xc0\xaf",
                    b"\xed\xa0\x80",
                    b"\xf4\x90\x80\x80",
                    b"\xe2\x82",
                ];
                let piece = rng.pick(bad);
                for (i, b) in piece.iter().enumerate() {
                    line.insert(at + i, *b);
                }
            }
            5 => {
                let tail = line[at..].to_vec();
                line.extend(tail);
            }
            6 => line.insert(at, b'\r'),
            _ => line.extend(std::iter::repeat_n(b' ', rng.below(5000))),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Invariants every accepted event must satisfy

fn assert_loc(loc: &Loc) {
    assert!(is_plain_text(&loc.file, MAX_LOC_FILE_BYTES));
    if let Some(func) = &loc.func {
        assert!(is_plain_text(func, MAX_LOC_FUNC_BYTES));
    }
}

fn assert_witness(w: &WireWitness) {
    assert!(is_thread_key(&w.thread, MAX_THREAD_KEY_BYTES));
}

fn assert_event_obeys_limits(event: &Event) {
    match event {
        Event::Hello(h) => {
            assert!(is_ident(&h.proc, MAX_PROC_BYTES));
            assert!(h.sdk.len() <= MAX_SDK_STR_BYTES && h.sdk_version.len() <= MAX_SDK_STR_BYTES);
            assert!(h.catalog_hash.len() <= MAX_HASH_BYTES);
        }
        Event::Catalog(c) => {
            assert!(is_ident(&c.proc, MAX_PROC_BYTES) && is_ident(&c.id, MAX_ID_BYTES));
            assert!(is_plain_text(&c.msg, MAX_MSG_BYTES));
            assert_eq!(c.must_hit, c.kind.must_hit());
            c.loc.iter().for_each(assert_loc);
        }
        Event::Assert(a) => {
            assert!(is_ident(&a.proc, MAX_PROC_BYTES) && is_ident(&a.id, MAX_ID_BYTES));
            assert_eq!(a.trues.checked_add(a.falses), Some(a.hits));
            a.first_false
                .iter()
                .chain(&a.first_true)
                .for_each(assert_witness);
        }
        Event::Work(w) => assert!(is_ident(&w.proc, MAX_PROC_BYTES)),
        Event::Thread(t) => {
            assert!(is_ident(&t.proc, MAX_PROC_BYTES));
            assert!(is_thread_key(&t.thread.thread, MAX_THREAD_KEY_BYTES));
        }
        Event::End(e) => {
            assert!(is_ident(&e.proc, MAX_PROC_BYTES));
            assert!(e.threads <= MAX_THREADS_PER_PROC as u64);
        }
        Event::Eval(e) => {
            assert!(is_ident(&e.proc, MAX_PROC_BYTES) && is_ident(&e.id, MAX_ID_BYTES));
        }
    }
}

/// What the line parser must say about a line, judged from the bytes alone.
fn assert_outcome_matches_bytes(line: &[u8], outcome: &LineOutcome) {
    let trimmed = line.strip_suffix(b"\r").unwrap_or(line);
    let has_prefix = trimmed.starts_with(EVENT_PREFIX.as_bytes());
    if has_prefix && trimmed.len() > MAX_LINE_BYTES {
        assert_eq!(*outcome, LineOutcome::Rejected(RejectReason::TooLong));
    }
    if has_prefix
        && trimmed.len() <= MAX_LINE_BYTES
        && std::str::from_utf8(&trimmed[EVENT_PREFIX.len()..]).is_err()
    {
        assert_eq!(*outcome, LineOutcome::Rejected(RejectReason::InvalidUtf8));
    }
    if !trimmed.starts_with(EVENT_PREFIX_BARE.as_bytes()) {
        assert_eq!(*outcome, LineOutcome::NotEvent);
    }
    if let LineOutcome::Event { .. } = outcome {
        assert!(has_prefix && trimmed.len() <= MAX_LINE_BYTES);
    }
}

#[test]
fn generated_lines_never_panic_and_accepted_events_obey_every_limit() {
    let n = cases(20_000);
    let mut accepted = 0usize;
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + case as u64 + 1);
        let line = event_line(&mut rng);
        let outcome = parse_line(&line);
        assert_outcome_matches_bytes(&line, &outcome);
        if let LineOutcome::Event { event, .. } = &outcome {
            accepted += 1;
            assert_event_obeys_limits(event);
        }
    }
    // The generator must reach the accepting path often enough for the limit checks to mean something.
    assert!(
        accepted * 8 >= n,
        "only {accepted} of {n} lines were accepted"
    );
}

#[test]
fn arbitrary_bytes_with_and_without_the_prefix_never_panic() {
    let n = cases(6_000);
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + 0xA000 + case as u64);
        let len = match rng.below(4) {
            0 => rng.below(40),
            1 => rng.below(400),
            2 => MAX_LINE_BYTES - 20 + rng.below(40),
            _ => rng.below(9000),
        };
        let mut line: Vec<u8> = if rng.chance(2) {
            EVENT_PREFIX.as_bytes().to_vec()
        } else {
            Vec::new()
        };
        line.extend((0..len).map(|_| rng.next() as u8));
        assert_outcome_matches_bytes(&line, &parse_line(&line));
    }
}

#[test]
fn nesting_beyond_the_limit_is_too_deep_and_never_overflows_the_stack() {
    // Root object is depth 1; an unknown field may add up to MAX_JSON_DEPTH - 1 more levels.
    for object in [true, false] {
        for depth in 0..=600usize {
            let body = format!(
                r#"{{"v":1,"kind":"work","proc":"p","ops":1,"x":{}}}"#,
                nested(depth, object)
            );
            let line = [EVENT_PREFIX.as_bytes(), body.as_bytes()].concat();
            let outcome = parse_line(&line);
            // Object nesting counts `{"k":` per level, array nesting counts `[`; both add `depth` levels.
            let levels = 1 + depth;
            if line.len() > MAX_LINE_BYTES {
                assert_eq!(outcome, LineOutcome::Rejected(RejectReason::TooLong));
            } else if levels > MAX_JSON_DEPTH {
                assert_eq!(
                    outcome,
                    LineOutcome::Rejected(RejectReason::TooDeep),
                    "depth {depth} object={object}"
                );
            } else {
                assert!(
                    matches!(outcome, LineOutcome::Event { .. }),
                    "depth {depth} object={object}: {outcome:?}"
                );
            }
        }
    }
}

#[test]
fn a_repeated_key_is_a_duplicate_wherever_it_appears() {
    for position in 0..6 {
        let mut keys = vec![
            r#""v":1"#.to_string(),
            r#""kind":"work""#.to_string(),
            r#""proc":"p""#.to_string(),
            r#""ops":1"#.to_string(),
        ];
        keys.insert(position.min(keys.len()), r#""ops":2"#.to_string());
        let line = format!("HAIRSPRING_EVENT {{{}}}", keys.join(","));
        assert_eq!(
            parse_line(line.as_bytes()),
            LineOutcome::Rejected(RejectReason::DuplicateKey),
            "{line}"
        );
    }
    // The same key in different objects is not a duplicate.
    let ok = r#"HAIRSPRING_EVENT {"v":1,"kind":"work","proc":"p","ops":1,"a":{"k":1},"b":{"k":2}}"#;
    assert!(matches!(
        parse_line(ok.as_bytes()),
        LineOutcome::Event { .. }
    ));
}

#[test]
fn numeric_extremes_are_accepted_exactly_or_rejected_never_rounded() {
    let max = u64::MAX.to_string();
    let line = format!(r#"HAIRSPRING_EVENT {{"v":1,"kind":"work","proc":"p","ops":{max}}}"#);
    match parse_line(line.as_bytes()) {
        LineOutcome::Event {
            event: Event::Work(w),
            ..
        } => assert_eq!(w.ops, u64::MAX),
        other => panic!("{other:?}"),
    }
    for bad in [
        "18446744073709551616",
        "-1",
        "1e999",
        "1e0",
        "1.0",
        "1.5",
        "NaN",
        "Infinity",
        "\"1\"",
        "null",
        "99999999999999999999999999999999999999",
    ] {
        let line = format!(r#"HAIRSPRING_EVENT {{"v":1,"kind":"work","proc":"p","ops":{bad}}}"#);
        assert!(
            matches!(parse_line(line.as_bytes()), LineOutcome::Rejected(_)),
            "ops={bad} must be rejected"
        );
    }
    // Witness values are signed: both ends of i64 are exact, one past either end is refused.
    let witness = |v: &str| {
        let wit = format!(r#"{{"thread":"w","ord":1,"vals":[{v},0,0,0]}}"#);
        format!(
            r#"HAIRSPRING_EVENT {{"v":1,"kind":"assert","proc":"p","id":"a","hits":1,"trues":0,"falses":1,"first_false":{wit},"first_true":null}}"#
        )
    };
    for (v, ok) in [
        ("9223372036854775807", true),
        ("-9223372036854775808", true),
        ("9223372036854775808", false),
        ("-9223372036854775809", false),
    ] {
        let outcome = parse_line(witness(v).as_bytes());
        match (&outcome, ok) {
            (
                LineOutcome::Event {
                    event: Event::Assert(a),
                    ..
                },
                true,
            ) => {
                assert_eq!(
                    a.first_false.as_ref().map(|w| w.vals[0]),
                    v.parse::<i64>().ok()
                );
            }
            (LineOutcome::Rejected(_), false) => {}
            _ => panic!("vals[0]={v}: {outcome:?}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The fold: whole streams, damaged and interleaved

/// A stream of generated lines, ordinary output, interleaved partial writes and an optional unterminated tail.
fn stream(rng: &mut Rng) -> (Vec<u8>, usize) {
    let mut out: Vec<u8> = Vec::new();
    let mut candidates = 0usize;
    let lines = rng.below(50);
    let mut pending: Option<Vec<u8>> = None;
    for _ in 0..lines {
        let mut line = match rng.below(10) {
            0 => b"ordinary program output".to_vec(),
            1 => b"HAIRSPRING_EVENTS_FILE=/tmp/x".to_vec(),
            2 => Vec::new(),
            _ => event_line(rng),
        };
        if rng.chance(12) && line.len() > 10 {
            // Two writers: the second line lands in the middle of the first.
            let cut = 1 + rng.below(line.len() - 1);
            let rest = line.split_off(cut);
            pending = Some(rest);
        }
        out.extend_from_slice(&line);
        let ending: &[u8] = if rng.chance(8) { b"\r\n" } else { b"\n" };
        if let Some(rest) = pending.take_if(|_| rng.chance(2)) {
            out.extend_from_slice(&rest);
        }
        out.extend_from_slice(ending);
    }
    if let Some(rest) = pending {
        out.extend_from_slice(&rest);
    }
    if rng.chance(3) {
        out.pop(); // no newline at end of file
    }
    for line in out.split(|b| *b == b'\n') {
        let l = line.strip_suffix(b"\r").unwrap_or(line);
        let bare = EVENT_PREFIX_BARE.as_bytes();
        let is_candidate = l.starts_with(EVENT_PREFIX.as_bytes())
            || (l.starts_with(bare)
                && matches!(l.get(bare.len()), None | Some(b'{') | Some(b'\t')));
        candidates += usize::from(is_candidate);
    }
    (out, candidates)
}

fn counted(h: &EventHealth) -> u64 {
    h.accepted
        + h.too_long
        + h.invalid_utf8
        + h.malformed
        + h.unknown_kind
        + h.unsupported_version
        + h.over_limit
}

fn assert_trial_is_bounded_and_honest(t: &TrialProps, candidates: usize, budget: usize) {
    assert!(t.procs.len() <= MAX_PROCS_PER_TRIAL);
    assert!(t.health.event_bytes as usize <= budget);
    assert!(t.properties.len() <= MAX_PROCS_PER_TRIAL * MAX_IDS_PER_PROC);
    for p in &t.procs {
        assert!(p.threads.len() <= MAX_THREADS_PER_PROC);
    }
    let mut seen = HashSet::new();
    for p in &t.properties {
        assert!(seen.insert(&p.id), "an id appears twice");
        assert!(p.hits >= p.trues && p.hits >= p.falses, "{p:?}");
        assert!(is_ident(&p.id, MAX_ID_BYTES));
        assert!(is_plain_text(&p.msg, MAX_MSG_BYTES));
    }
    // Every line that claimed to be an event is accounted for somewhere; nothing vanishes silently.
    assert!(
        counted(&t.health) >= candidates as u64,
        "{candidates} candidate lines but only {} counted: {:?}",
        counted(&t.health),
        t.health
    );
    // Lines that were refused or lost always degrade the trial, so no caller reads the counts as complete.
    let refused =
        t.health.too_long + t.health.invalid_utf8 + t.health.malformed + t.health.over_limit;
    if refused > 0 {
        assert!(t.is_degraded(), "{:?}", t.health);
    }
    // A failure the guest reported is never hidden by degradation.
    for p in &t.properties {
        if p.failure().is_some() {
            assert!(t.verdict().iter().any(|f| f.id == p.id));
        }
    }
}

#[test]
fn folding_generated_streams_stays_bounded_and_accounts_for_every_candidate_line() {
    let n = cases(4_000);
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + 0xB000 + case as u64);
        let (out, candidates) = stream(&mut rng);
        let budget = match rng.below(4) {
            0 => rng.below(2000),
            1 => LEGACY_MAX_EVENT_BYTES_PER_TRIAL,
            _ => DEFAULT_MAX_EVENT_BYTES_PER_TRIAL,
        };
        let t = TrialEvents::from_output(&out).fold_with_budget(budget);
        assert_trial_is_bounded_and_honest(&t, candidates, budget);
        // The same bytes folded line by line give the same answer.
        let lines = TrialEvents::from_lines(out.split(|b| *b == b'\n')).fold_with_budget(budget);
        assert_eq!(t, lines, "case {case}");
        let _ = t.verdict();
        let _ = (t.report_missing(), t.thread_reports(), t.total_work());
    }
}

#[test]
fn interleaved_partial_writes_are_judged_by_the_same_rules_as_any_line() {
    // Two valid lines cut at every offset and spliced together: whatever is accepted must be one of the originals.
    let a = br#"HAIRSPRING_EVENT {"v":1,"kind":"work","proc":"a","ops":7}"#.to_vec();
    let b = br#"HAIRSPRING_EVENT {"v":1,"kind":"work","proc":"b","ops":9}"#.to_vec();
    for cut_a in 0..=a.len() {
        for cut_b in 0..=b.len() {
            let mut spliced = a[..cut_a].to_vec();
            spliced.extend_from_slice(&b[cut_b..]);
            // The protocol has no checksum, so a splice may well be a different valid line ("ab", 79); what it
            // must never be is an event outside the limits, or a panic.
            if let LineOutcome::Event { event, .. } = parse_line(&spliced) {
                assert_event_obeys_limits(&event);
            }
        }
    }
}

#[test]
fn run_summaries_never_claim_more_than_the_trials_added() {
    let n = cases(300);
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + 0xC000 + case as u64);
        let cap = [3usize, 8, MAX_IDS_PER_RUN][rng.below(3)];
        let mut run = RunProps::with_id_cap(cap);
        let trials = 1 + rng.below(8);
        let mut degraded = 0u64;
        let mut inconclusive = 0u64;
        for seed in 0..trials {
            let (out, _) = stream(&mut rng);
            let t = TrialEvents::from_output(&out).fold();
            if rng.chance(5) {
                inconclusive += 1;
                run = run.add_inconclusive(seed as u64, &t);
            } else {
                degraded += u64::from(t.is_degraded());
                run = run.add(seed as u64, &t);
            }
        }
        let s = run.summary();
        assert_eq!(s.trials, trials as u64, "case {case}");
        assert_eq!(s.inconclusive_trials, inconclusive);
        assert!(s.degraded_trials >= degraded.min(s.trials));
        assert!(s.degraded_trials <= s.trials);
        assert!(s.report_missing_trials <= s.trials);
        assert!(s.conflict_trials <= s.trials);
        if s.ids_dropped > 0 {
            assert!(
                s.degraded_trials > 0,
                "ids were dropped but no trial is degraded"
            );
        }
        let stats = run.finish();
        assert!(stats.len() <= cap);
        for p in &stats {
            let bound = trials as u64;
            assert!(p.trials <= bound);
            assert!(p.trials_rated <= p.trials);
            assert!(p.trials_claimable <= p.trials_rated);
            assert!(
                p.trials_hit <= p.trials && p.trials_false <= p.trials && p.trials_true <= p.trials
            );
            assert!(p.trials_rated + p.trials_unreported + p.trials_degraded >= p.trials_rated);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The digest decoder and the run catalog

fn text(rng: &mut Rng, max: usize, charset: &[u8]) -> String {
    let len = 1 + rng.below(max.min(24));
    (0..len).map(|_| *rng.pick(charset) as char).collect()
}

const IDENT: &[u8] = b"abcXYZ019._:/-";

fn random_digest(rng: &mut Rng) -> TrialDigest {
    let mut entries = Vec::new();
    let mut witnesses = 0;
    let mut decls = 0;
    let mut idx = HashSet::new();
    for _ in 0..rng.below(12) {
        let i = match rng.below(4) {
            0 => u32::MAX,
            _ => rng.below(100) as u32,
        };
        if !idx.insert(i) {
            continue;
        }
        let kind = *rng.pick(&AssertKind::ALL);
        let conflicted = rng.chance(4);
        let catalogued = !conflicted || rng.chance(2);
        let (mut hits, mut trues, mut falses) = (0u64, 0u64, 0u64);
        if catalogued {
            trues = if rng.chance(6) {
                u64::MAX / 2
            } else {
                rng.below(1000) as u64
            };
            falses = if rng.chance(6) {
                u64::MAX / 2
            } else {
                rng.below(1000) as u64
            };
            hits = trues + falses;
        }
        let failing = catalogued && trace_failure(kind, hits, falses);
        let witness = (failing && witnesses < MAX_DIGEST_WITNESSES && rng.chance(2)).then(|| {
            witnesses += 1;
            Witness {
                proc: text(rng, MAX_PROC_BYTES, IDENT),
                thread: text(rng, MAX_THREAD_KEY_BYTES, b"w0#-_"),
                ord: rng.next(),
                vals: [rng.next() as i64, i64::MIN, i64::MAX, -1],
            }
        });
        let decl = (catalogued && decls < MAX_DIGEST_DECLS && rng.chance(3)).then(|| {
            decls += 1;
            Decl {
                msg: text(rng, MAX_MSG_BYTES, b"hello world, 123"),
                loc: rng.chance(2).then(|| Loc {
                    file: text(rng, MAX_LOC_FILE_BYTES, IDENT),
                    line: rng.next(),
                    func: rng.chance(2).then(|| text(rng, MAX_LOC_FUNC_BYTES, IDENT)),
                }),
            }
        });
        entries.push(DigestEntry {
            idx: i,
            kind,
            catalogued,
            conflicted,
            hits,
            trues,
            falses,
            witness,
            decl,
        });
    }
    TrialDigest {
        flags: DigestFlags {
            inconclusive: rng.chance(2),
            degraded: rng.chance(2),
            perturbing: rng.chance(2),
            report_missing: rng.chance(2),
            has_conflicts: rng.chance(2),
            excluded: rng.chance(2),
        },
        ids_dropped: if rng.chance(4) {
            u64::MAX
        } else {
            rng.below(50) as u64
        },
        entries,
    }
}

fn trace_failure(kind: AssertKind, hits: u64, falses: u64) -> bool {
    failure_basis(kind, hits, falses).is_some()
}

#[test]
fn digests_round_trip_and_the_decoder_only_accepts_what_the_encoder_writes() {
    let n = cases(10_000);
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + 0xD000 + case as u64);
        let digest = random_digest(&mut rng);
        let bytes = digest.encode();
        assert_eq!(
            TrialDigest::decode(&bytes),
            Ok(digest.clone()),
            "case {case}"
        );

        // Damaged copies: never a panic; an accepted copy must be canonical, so it re-encodes to the same bytes.
        let mut damaged = bytes.clone();
        corrupt(&mut rng, &mut damaged);
        if let Ok(d) = TrialDigest::decode(&damaged) {
            assert_eq!(
                d.encode(),
                damaged,
                "case {case}: a non-canonical digest was accepted"
            );
            assert!(d.entries.len() <= MAX_IDS_PER_RUN);
            assert!(
                d.entries.iter().filter(|e| e.witness.is_some()).count() <= MAX_DIGEST_WITNESSES
            );
            for e in &d.entries {
                if e.catalogued {
                    assert_eq!(e.trues.checked_add(e.falses), Some(e.hits));
                }
                if let Some(decl) = &e.decl {
                    assert!(is_plain_text(&decl.msg, MAX_MSG_BYTES));
                }
            }
        }
        // Every truncation is refused rather than read as a shorter digest.
        if !bytes.is_empty() {
            let cut = rng.below(bytes.len());
            assert!(
                TrialDigest::decode(&bytes[..cut]).is_err(),
                "case {case}: cut at {cut}"
            );
        }
    }
}

#[test]
fn arbitrary_bytes_never_panic_the_digest_decoder_and_big_claims_do_not_allocate() {
    let n = cases(6_000);
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + 0xE000 + case as u64);
        let len = rng.below(200);
        let mut bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if rng.chance(2) && bytes.len() >= 2 {
            bytes[0] = 1; // a plausible version, so the decoder gets further in
            bytes[1] &= 0b11_1111;
        }
        if let Ok(d) = TrialDigest::decode(&bytes) {
            assert_eq!(d.encode(), bytes);
        }
    }
    // A header that promises the most entries with no body, and counts and lengths at the edge of 64 bits.
    let mut huge = vec![1u8, 0, 0];
    huge.extend([0xFF; 9]);
    huge.push(0x01);
    assert!(TrialDigest::decode(&huge).is_err());
    let mut over = vec![1u8, 0, 0];
    over.extend([0xFF; 9]);
    over.push(0x02);
    assert!(TrialDigest::decode(&over).is_err());
    let at_cap = {
        let mut v = vec![1u8, 0, 0];
        let mut count = MAX_IDS_PER_RUN as u64;
        while count >= 0x80 {
            v.push((count as u8 & 0x7f) | 0x80);
            count >>= 7;
        }
        v.push(count as u8);
        v
    };
    assert!(TrialDigest::decode(&at_cap).is_err());
    assert!(TrialDigest::decode(&vec![1u8; MAX_DIGEST_BYTES + 1]).is_err());
}

fn random_row(rng: &mut Rng) -> CatalogRow {
    let id = match rng.below(6) {
        0 => String::new(),
        1 => "x".repeat(MAX_ID_BYTES + 1),
        2 => rng
            .pick(&["a b", "a\u{202e}b", "a\0b", "ok.id"])
            .to_string(),
        _ => format!("id{}", rng.below(20)),
    };
    let msg = match rng.below(5) {
        0 => "x".repeat(MAX_MSG_BYTES + 1),
        1 => "bidi \u{202e} text".into(),
        2 => "line\nbreak".into(),
        _ => "plain message".into(),
    };
    CatalogRow {
        id,
        kind: *rng.pick(&AssertKind::ALL),
        msg,
        loc: rng.chance(2).then(|| Loc {
            file: ["f.c", "", "tab\tfile"][rng.below(3)].into(),
            line: rng.next(),
            func: None,
        }),
        decided: rng.chance(2),
    }
}

#[test]
fn catalog_updates_from_an_untrusted_sender_keep_every_row_valid_and_the_cap() {
    let n = cases(4_000);
    for case in 0..n {
        let mut rng = Rng::new(base_seed() + 0xF000 + case as u64);
        let cap = 1 + rng.below(10);
        let mut cat = RunCatalog::new();
        for _ in 0..rng.below(40) {
            let idx = if rng.chance(4) {
                rng.next() as u32
            } else {
                rng.below(cat.len() + 2) as u32
            };
            let update = CatalogUpdate {
                idx,
                row: random_row(&mut rng),
            };
            if let Ok(next) = cat.clone().apply(&update, cap) {
                cat = next;
            }
            assert!(cat.len() <= cap, "case {case}");
            let ids: HashSet<&str> = cat.rows().iter().map(|r| r.id.as_str()).collect();
            assert_eq!(ids.len(), cat.len(), "ids must stay unique");
            for (i, row) in cat.rows().iter().enumerate() {
                assert!(row.validate().is_ok());
                assert_eq!(cat.index_of(&row.id), Some(i as u32));
            }
        }
    }
}

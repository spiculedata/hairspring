use hairspring_props::limits::*;
use hairspring_props::*;

fn line(json: &str) -> Vec<u8> {
    format!("{EVENT_PREFIX}{json}").into_bytes()
}

fn parse(json: &str) -> LineOutcome {
    parse_line(&line(json))
}

fn event_of(json: &str) -> (Event, u32) {
    match parse(json) {
        LineOutcome::Event {
            event,
            unknown_fields,
        } => (event, unknown_fields),
        other => panic!("expected an event for {json}, got {other:?}"),
    }
}

fn rejected(json: &str) -> RejectReason {
    match parse(json) {
        LineOutcome::Rejected(r) => r,
        other => panic!("expected rejection for {json}, got {other:?}"),
    }
}

const HELLO: &str = r#"{"v":1,"kind":"hello","sdk":"c","sdk_version":"0.1.0","proc":"p1","catalog_hash":"9f2c","signal_handlers":false}"#;
const CATALOG: &str = r#"{"v":1,"kind":"catalog","proc":"p1","id":"wal.ckpt.nbackfill_le_mxframe","type":"always","must_hit":true,"msg":"nBackfill <= live mxFrame","loc":{"file":"sqlite3.c","line":66123,"func":"walCheckpoint"}}"#;
const ASSERT: &str = r#"{"v":1,"kind":"assert","proc":"p1","id":"a.b","hits":                 412,"trues":                 411,"falses":                   1,"first_false":{"thread":"writer-0","ord":                 675,"vals":[71,8,0,0]},"first_true":{"thread":"ckpt","ord":                   3,"vals":[0,5,0,0]}}"#;
const WORK: &str = r#"{"v":1,"kind":"work","proc":"p1","ops":                 300}"#;
const END: &str = r#"{"v":1,"kind":"end","proc":"p1","reason":"exit","status":0,"sites":5,"evals":                9123,"dropped":0,"threads":[{"thread":"writer-0","evals":4100,"tape_consumed":312},{"thread":"ckpt","evals":980,"tape_consumed":57,"key_stable":false}]}"#;

#[test]
fn ordinary_output_is_not_an_event() {
    assert_eq!(parse_line(b"hello world"), LineOutcome::NotEvent);
    assert_eq!(parse_line(b""), LineOutcome::NotEvent);
    assert_eq!(parse_line(b"  HAIRSPRING_EVENT {}"), LineOutcome::NotEvent);
    assert_eq!(
        parse_line(b"HAIRSPRING_EVENTS_FILE=/x"),
        LineOutcome::NotEvent
    );
    assert_eq!(
        parse_line(b"HAIRSPRING_VIOLATION: broke"),
        LineOutcome::NotEvent
    );
}

#[test]
fn hello_golden() {
    let (ev, unknown) = event_of(HELLO);
    assert_eq!(unknown, 0);
    assert_eq!(
        ev,
        Event::Hello(Hello {
            proc: "p1".into(),
            sdk: "c".into(),
            sdk_version: "0.1.0".into(),
            catalog_hash: "9f2c".into(),
            signal_handlers: false,
        })
    );
}

#[test]
fn catalog_golden() {
    let (ev, _) = event_of(CATALOG);
    assert_eq!(
        ev,
        Event::Catalog(CatalogEntry {
            proc: "p1".into(),
            id: "wal.ckpt.nbackfill_le_mxframe".into(),
            kind: AssertKind::Always,
            must_hit: true,
            msg: "nBackfill <= live mxFrame".into(),
            loc: Some(Loc {
                file: "sqlite3.c".into(),
                line: 66123,
                func: Some("walCheckpoint".into())
            }),
        })
    );
}

#[test]
fn catalog_without_loc_or_msg_is_accepted() {
    let (ev, _) = event_of(
        r#"{"v":1,"kind":"catalog","proc":"p1","id":"x","type":"reachable","must_hit":true}"#,
    );
    match ev {
        Event::Catalog(c) => {
            assert_eq!(c.msg, "");
            assert_eq!(c.loc, None);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn assert_golden_accepts_fixed_width_numbers() {
    let (ev, _) = event_of(ASSERT);
    assert_eq!(
        ev,
        Event::Assert(AssertReport {
            proc: "p1".into(),
            id: "a.b".into(),
            hits: 412,
            trues: 411,
            falses: 1,
            first_false: Some(WireWitness {
                thread: "writer-0".into(),
                ord: 675,
                vals: [71, 8, 0, 0]
            }),
            first_true: Some(WireWitness {
                thread: "ckpt".into(),
                ord: 3,
                vals: [0, 5, 0, 0]
            }),
        })
    );
}

#[test]
fn assert_accepts_plain_numbers_null_witnesses_and_negative_vals() {
    let (ev, _) = event_of(
        r#"{"v":1,"kind":"assert","proc":"p1","id":"a","hits":2,"trues":2,"falses":0,"first_false":null,"first_true":{"thread":"t","ord":1,"vals":[-1,0,0,9223372036854775807]}}"#,
    );
    match ev {
        Event::Assert(a) => {
            assert_eq!(a.first_false, None);
            assert_eq!(a.first_true.unwrap().vals, [-1, 0, 0, i64::MAX]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn assert_accepts_u64_max_counters() {
    let (ev, _) = event_of(
        r#"{"v":1,"kind":"assert","proc":"p","id":"a","hits":18446744073709551615,"trues":18446744073709551615,"falses":0}"#,
    );
    match ev {
        Event::Assert(a) => assert_eq!(a.hits, u64::MAX),
        other => panic!("{other:?}"),
    }
}

#[test]
fn work_and_end_goldens() {
    let (ev, _) = event_of(WORK);
    assert_eq!(
        ev,
        Event::Work(Work {
            proc: "p1".into(),
            ops: 300
        })
    );
    let (ev, _) = event_of(END);
    assert_eq!(
        ev,
        Event::End(End {
            proc: "p1".into(),
            reason: EndReason::Exit,
            status: 0,
            sites: 5,
            evals: 9123,
            dropped: 0,
            threads: vec![
                ThreadReport {
                    thread: "writer-0".into(),
                    evals: 4100,
                    tape_consumed: 312,
                    key_stable: true
                },
                ThreadReport {
                    thread: "ckpt".into(),
                    evals: 980,
                    tape_consumed: 57,
                    key_stable: false
                },
            ],
        })
    );
}

#[test]
fn eval_defaults_the_proc_tag() {
    let (ev, _) = event_of(r#"{"v":1,"kind":"eval","id":"x.y","type":"sometimes","ok":true}"#);
    assert_eq!(
        ev,
        Event::Eval(Eval {
            proc: DEFAULT_EVAL_PROC.into(),
            id: "x.y".into(),
            kind: AssertKind::Sometimes,
            ok: true
        })
    );
}

#[test]
fn carriage_return_is_tolerated() {
    let mut l = line(WORK);
    l.push(b'\r');
    assert!(matches!(parse_line(&l), LineOutcome::Event { .. }));
}

#[test]
fn unknown_fields_are_counted_and_ignored() {
    let (_, unknown) =
        event_of(r#"{"v":1,"kind":"work","proc":"p1","ops":3,"future":1,"other":{"deep":[1]}}"#);
    assert_eq!(unknown, 2);
    let (_, unknown) = event_of(
        r#"{"v":1,"kind":"catalog","proc":"p","id":"x","type":"always","must_hit":true,"loc":{"file":"f","line":1,"col":3}}"#,
    );
    assert_eq!(unknown, 1);
}

#[test]
fn unknown_kind_is_ignored() {
    assert_eq!(
        parse(r#"{"v":1,"kind":"telemetry","x":1}"#),
        LineOutcome::Ignored(IgnoreReason::UnknownKind)
    );
}

#[test]
fn newer_major_version_is_ignored_even_with_unknown_shape() {
    assert_eq!(
        parse(r#"{"v":2,"kind":"work","ops":"many"}"#),
        LineOutcome::Ignored(IgnoreReason::UnsupportedVersion)
    );
    assert_eq!(
        parse(r#"{"v":18446744073709551615,"kind":"zzz"}"#),
        LineOutcome::Ignored(IgnoreReason::UnsupportedVersion)
    );
}

#[test]
fn bad_version_values_are_rejected() {
    assert_eq!(
        rejected(r#"{"kind":"work"}"#),
        RejectReason::MissingField("v")
    );
    assert_eq!(
        rejected(r#"{"v":0,"kind":"work","proc":"p","ops":1}"#),
        RejectReason::InvalidField("v")
    );
    assert_eq!(
        rejected(r#"{"v":"1","kind":"work","proc":"p","ops":1}"#),
        RejectReason::InvalidField("v")
    );
    assert_eq!(
        rejected(r#"{"v":1.5,"kind":"work","proc":"p","ops":1}"#),
        RejectReason::InvalidField("v")
    );
    assert_eq!(
        rejected(r#"{"v":-1,"kind":"work","proc":"p","ops":1}"#),
        RejectReason::InvalidField("v")
    );
    assert_eq!(rejected(r#"{"v":1}"#), RejectReason::MissingField("kind"));
    assert_eq!(
        rejected(r#"{"v":1,"kind":7}"#),
        RejectReason::InvalidField("kind")
    );
}

#[test]
fn missing_and_mistyped_fields_are_rejected() {
    assert_eq!(
        rejected(r#"{"v":1,"kind":"work","proc":"p"}"#),
        RejectReason::MissingField("ops")
    );
    assert_eq!(
        rejected(r#"{"v":1,"kind":"work","proc":"p","ops":-1}"#),
        RejectReason::InvalidField("ops")
    );
    assert_eq!(
        rejected(r#"{"v":1,"kind":"work","proc":"p","ops":1.5}"#),
        RejectReason::InvalidField("ops")
    );
    assert_eq!(
        rejected(r#"{"v":1,"kind":"work","ops":1}"#),
        RejectReason::MissingField("proc")
    );
    assert_eq!(
        rejected(r#"{"v":1,"kind":"catalog","proc":"p","id":"x","type":"nope","must_hit":true}"#),
        RejectReason::InvalidField("type")
    );
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"end","proc":"p","reason":"boom","status":0,"sites":0,"evals":0,"dropped":0,"threads":[]}"#
        ),
        RejectReason::InvalidField("reason")
    );
}

#[test]
fn must_hit_must_agree_with_the_kind() {
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"catalog","proc":"p","id":"x","type":"unreachable","must_hit":true}"#
        ),
        RejectReason::Inconsistent("must_hit")
    );
}

#[test]
fn assert_counters_must_be_consistent() {
    assert_eq!(
        rejected(r#"{"v":1,"kind":"assert","proc":"p","id":"a","hits":3,"trues":1,"falses":1}"#),
        RejectReason::Inconsistent("hits")
    );
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"assert","proc":"p","id":"a","hits":18446744073709551615,"trues":18446744073709551615,"falses":1}"#
        ),
        RejectReason::Inconsistent("hits")
    );
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"assert","proc":"p","id":"a","hits":1,"trues":1,"falses":0,"first_false":{"thread":"t","ord":1,"vals":[0,0,0,0]}}"#
        ),
        RejectReason::Inconsistent("first_false")
    );
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"assert","proc":"p","id":"a","hits":1,"trues":0,"falses":1,"first_true":{"thread":"t","ord":1,"vals":[0,0,0,0]}}"#
        ),
        RejectReason::Inconsistent("first_true")
    );
}

#[test]
fn vals_must_be_exactly_four_integers() {
    for vals in [
        "[]",
        "[1,2,3]",
        "[1,2,3,4,5]",
        "[1,2,3,1.5]",
        r#"[1,2,3,"4"]"#,
        "7",
    ] {
        let json = format!(
            r#"{{"v":1,"kind":"assert","proc":"p","id":"a","hits":1,"trues":0,"falses":1,"first_false":{{"thread":"t","ord":1,"vals":{vals}}}}}"#
        );
        assert_eq!(
            rejected(&json),
            RejectReason::InvalidField("vals"),
            "{vals}"
        );
    }
}

#[test]
fn id_charset_and_length_are_enforced() {
    let mk = |id: &str| {
        format!(
            r#"{{"v":1,"kind":"catalog","proc":"p","id":"{id}","type":"reachable","must_hit":true}}"#
        )
    };
    assert!(matches!(
        parse(&mk("ok.id:1/x-y_z")),
        LineOutcome::Event { .. }
    ));
    for bad in ["", "has space", "caf\u{e9}", "a\\u0000b", "a;b", "a\\nb"] {
        assert_eq!(
            rejected(&mk(bad)),
            RejectReason::InvalidField("id"),
            "{bad:?}"
        );
    }
    assert!(matches!(
        parse(&mk(&"a".repeat(MAX_ID_BYTES))),
        LineOutcome::Event { .. }
    ));
    assert_eq!(
        rejected(&mk(&"a".repeat(MAX_ID_BYTES + 1))),
        RejectReason::OverLimit("id")
    );
}

#[test]
fn thread_and_proc_keys_are_limited() {
    let with_thread = |t: &str| {
        format!(
            r#"{{"v":1,"kind":"assert","proc":"p","id":"a","hits":1,"trues":0,"falses":1,"first_false":{{"thread":"{t}","ord":1,"vals":[0,0,0,0]}}}}"#
        )
    };
    assert!(matches!(
        parse(&with_thread(&"t".repeat(MAX_THREAD_KEY_BYTES))),
        LineOutcome::Event { .. }
    ));
    assert_eq!(
        rejected(&with_thread(&"t".repeat(MAX_THREAD_KEY_BYTES + 1))),
        RejectReason::OverLimit("thread")
    );
    assert_eq!(
        rejected(&with_thread("bad key")),
        RejectReason::InvalidField("thread")
    );
    let proc = "p".repeat(MAX_PROC_BYTES + 1);
    assert_eq!(
        rejected(&format!(
            r#"{{"v":1,"kind":"work","proc":"{proc}","ops":1}}"#
        )),
        RejectReason::OverLimit("proc")
    );
}

#[test]
fn message_length_and_control_characters_are_enforced() {
    let mk = |msg: &str| {
        format!(
            r#"{{"v":1,"kind":"catalog","proc":"p","id":"x","type":"reachable","must_hit":true,"msg":"{msg}"}}"#
        )
    };
    assert!(matches!(
        parse(&mk(&"m".repeat(MAX_MSG_BYTES))),
        LineOutcome::Event { .. }
    ));
    assert_eq!(
        rejected(&mk(&"m".repeat(MAX_MSG_BYTES + 1))),
        RejectReason::OverLimit("msg")
    );
    assert_eq!(rejected(&mk("a\\nb")), RejectReason::InvalidField("msg"));
    assert_eq!(
        rejected(&mk("a\\u001bb")),
        RejectReason::InvalidField("msg")
    );
    assert!(matches!(
        parse(&mk("caf\u{e9} \u{2713}")),
        LineOutcome::Event { .. }
    ));
}

#[test]
fn line_length_boundaries() {
    // Pad `msg` so the whole line (prefix included) is exactly the wanted length.
    let make = |total: usize| {
        let base = format!(
            r#"{EVENT_PREFIX}{{"v":1,"kind":"catalog","proc":"p","id":"x","type":"reachable","must_hit":true,"pad":""}}"#
        );
        let extra = total - base.len();
        format!(
            r#"{EVENT_PREFIX}{{"v":1,"kind":"catalog","proc":"p","id":"x","type":"reachable","must_hit":true,"pad":"{}"}}"#,
            "z".repeat(extra)
        )
        .into_bytes()
    };
    assert!(matches!(
        parse_line(&make(MAX_LINE_BYTES - 1)),
        LineOutcome::Event { .. }
    ));
    assert!(matches!(
        parse_line(&make(MAX_LINE_BYTES)),
        LineOutcome::Event { .. }
    ));
    assert_eq!(
        parse_line(&make(MAX_LINE_BYTES + 1)),
        LineOutcome::Rejected(RejectReason::TooLong)
    );
}

#[test]
fn non_event_lines_may_be_any_length() {
    assert_eq!(parse_line(&vec![b'x'; 1 << 20]), LineOutcome::NotEvent);
}

#[test]
fn invalid_utf8_is_rejected() {
    let mut l = line(r#"{"v":1,"kind":"work","proc":"p","ops":1,"pad":"AB"}"#);
    let at = l.len() - 5;
    l[at] = 0xff;
    assert_eq!(
        parse_line(&l),
        LineOutcome::Rejected(RejectReason::InvalidUtf8)
    );
}

#[test]
fn nul_bytes_are_rejected() {
    let mut l = line(WORK);
    l.insert(EVENT_PREFIX.len() + 3, 0);
    assert!(matches!(parse_line(&l), LineOutcome::Rejected(_)));
    let l = b"HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"p\",\"ops\":1,\"x\":\"a\0b\"}";
    assert!(matches!(parse_line(l), LineOutcome::Rejected(_)));
}

#[test]
fn missing_separator_is_rejected() {
    assert_eq!(
        parse_line(b"HAIRSPRING_EVENT{\"v\":1}"),
        LineOutcome::Rejected(RejectReason::MissingSeparator)
    );
    assert_eq!(
        parse_line(b"HAIRSPRING_EVENT"),
        LineOutcome::Rejected(RejectReason::MissingSeparator)
    );
}

#[test]
fn malformed_json_is_rejected() {
    for bad in [
        "",
        "{",
        "}",
        "nope",
        "[1,2]",
        "{\"v\":1,}",
        "{\"v\":1} trailing",
        "{'v':1}",
    ] {
        assert!(
            matches!(parse(bad), LineOutcome::Rejected(_)),
            "{bad:?} gave {:?}",
            parse(bad)
        );
    }
    assert_eq!(rejected("[1,2]"), RejectReason::NotAnObject);
    assert_eq!(rejected("{\"v\":1} trailing"), RejectReason::MalformedJson);
}

#[test]
fn duplicate_keys_are_rejected_at_any_depth() {
    assert_eq!(
        rejected(r#"{"v":1,"kind":"work","proc":"p","ops":1,"ops":2}"#),
        RejectReason::DuplicateKey
    );
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"catalog","proc":"p","id":"x","type":"always","must_hit":true,"loc":{"file":"a","file":"b","line":1}}"#
        ),
        RejectReason::DuplicateKey
    );
}

#[test]
fn deep_nesting_is_rejected_without_overflowing_the_stack() {
    let deep = format!(
        r#"{{"v":1,"kind":"work","proc":"p","ops":1,"x":{}1{}}}"#,
        "[".repeat(4),
        "]".repeat(4)
    );
    assert_eq!(rejected(&deep), RejectReason::TooDeep);
    let huge = format!(
        r#"{{"v":1,"kind":"work","proc":"p","ops":1,"x":{}{}}}"#,
        "[".repeat(1500),
        "]".repeat(1500)
    );
    assert!(matches!(parse(&huge), LineOutcome::Rejected(_)));
}

fn end_with_threads(n: usize) -> String {
    let entries = (0..n)
        .map(|i| format!(r#"{{"thread":"t{i}","evals":1,"tape_consumed":0}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"v":1,"kind":"end","proc":"p","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":[{entries}]}}"#
    )
}

#[test]
fn end_line_with_many_threads_fits_until_the_line_limit() {
    match parse(&end_with_threads(60)) {
        LineOutcome::Event {
            event: Event::End(e),
            ..
        } => assert_eq!(e.threads.len(), 60),
        other => panic!("{other:?}"),
    }
    // MAX_THREADS_PER_PROC + 1 entries cannot fit in one line, so the line-length limit drops it first.
    assert_eq!(
        parse(&end_with_threads(MAX_THREADS_PER_PROC + 1)),
        LineOutcome::Rejected(RejectReason::TooLong)
    );
}

#[test]
fn duplicate_thread_keys_in_end_are_rejected() {
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"end","proc":"p","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":[{"thread":"t","evals":1,"tape_consumed":0},{"thread":"t","evals":1,"tape_consumed":0}]}"#
        ),
        RejectReason::Inconsistent("threads")
    );
}

#[test]
fn end_with_bad_thread_entry_is_rejected() {
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"end","proc":"p","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":[{"thread":"t","evals":1}]}"#
        ),
        RejectReason::MissingField("tape_consumed")
    );
    assert_eq!(
        rejected(
            r#"{"v":1,"kind":"end","proc":"p","reason":"exit","status":0,"sites":0,"evals":0,"dropped":0,"threads":"x"}"#
        ),
        RejectReason::InvalidField("threads")
    );
}

#[test]
fn signed_end_status_is_accepted() {
    let (ev, _) = event_of(
        r#"{"v":1,"kind":"end","proc":"p","reason":"signal","status":-11,"sites":0,"evals":0,"dropped":0,"threads":[]}"#,
    );
    match ev {
        Event::End(e) => {
            assert_eq!(e.status, -11);
            assert_eq!(e.reason, EndReason::Signal);
        }
        other => panic!("{other:?}"),
    }
}

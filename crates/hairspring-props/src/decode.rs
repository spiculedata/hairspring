//! Validation and typing of one parsed JSON object into a protocol v1 [`Event`].

use crate::event::*;
use crate::json::{Json, Num};
use crate::limits::*;
use crate::line::{IgnoreReason, RejectReason};
use crate::valid::{is_ident_byte, is_invisible, is_thread_byte};

pub(crate) enum Decoded {
    Event {
        event: Box<Event>,
        unknown_fields: u32,
    },
    Ignored(IgnoreReason),
}

type Obj = [(String, Json)];
type Res<T> = Result<T, RejectReason>;

pub(crate) fn decode_event(root: &Json) -> Res<Decoded> {
    let Json::Obj(obj) = root else {
        return Err(RejectReason::NotAnObject);
    };
    match field(obj, "v")? {
        Json::Num(Num::Unsigned(0)) => return Err(RejectReason::InvalidField("v")),
        Json::Num(Num::Unsigned(v)) if *v > PROTOCOL_VERSION => {
            return Ok(Decoded::Ignored(IgnoreReason::UnsupportedVersion))
        }
        Json::Num(Num::Unsigned(_)) => {}
        _ => return Err(RejectReason::InvalidField("v")),
    }
    let Json::Str(kind) = field(obj, "kind")? else {
        return Err(RejectReason::InvalidField("kind"));
    };
    let (event, known): (Event, &[&str]) = match kind.as_str() {
        "hello" => (decode_hello(obj)?, &HELLO_KEYS),
        "catalog" => (decode_catalog(obj)?, &CATALOG_KEYS),
        "assert" => (decode_assert(obj)?, &ASSERT_KEYS),
        "work" => (decode_work(obj)?, &WORK_KEYS),
        "thread" => (decode_thread(obj)?, &THREAD_KEYS),
        "end" => (decode_end(obj)?, &END_KEYS),
        "eval" => (decode_eval(obj)?, &EVAL_KEYS),
        _ => return Ok(Decoded::Ignored(IgnoreReason::UnknownKind)),
    };
    let unknown_fields = unknown_count(obj, known) + nested_unknown(obj);
    Ok(Decoded::Event {
        event: Box::new(event),
        unknown_fields,
    })
}

const HELLO_KEYS: [&str; 7] = [
    "v",
    "kind",
    "sdk",
    "sdk_version",
    "proc",
    "catalog_hash",
    "signal_handlers",
];
const CATALOG_KEYS: [&str; 8] = ["v", "kind", "proc", "id", "type", "must_hit", "msg", "loc"];
const ASSERT_KEYS: [&str; 9] = [
    "v",
    "kind",
    "proc",
    "id",
    "hits",
    "trues",
    "falses",
    "first_false",
    "first_true",
];
const WORK_KEYS: [&str; 4] = ["v", "kind", "proc", "ops"];
const END_KEYS: [&str; 9] = [
    "v", "kind", "proc", "reason", "status", "sites", "evals", "dropped", "threads",
];
const EVAL_KEYS: [&str; 6] = ["v", "kind", "proc", "id", "type", "ok"];
const LOC_KEYS: [&str; 3] = ["file", "line", "func"];
const WITNESS_KEYS: [&str; 3] = ["thread", "ord", "vals"];
const THREAD_KEYS: [&str; 7] = [
    "v",
    "kind",
    "proc",
    "thread",
    "evals",
    "tape_consumed",
    "key_stable",
];

fn unknown_count(obj: &Obj, known: &[&str]) -> u32 {
    let n = obj
        .iter()
        .filter(|(k, _)| !known.contains(&k.as_str()))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Unknown fields inside `loc` and witnesses (already validated as well-formed objects).
fn nested_unknown(obj: &Obj) -> u32 {
    let mut total = 0u32;
    let mut add = |n: u32| total = total.saturating_add(n);
    if let Some(Json::Obj(loc)) = lookup(obj, "loc") {
        add(unknown_count(loc, &LOC_KEYS));
    }
    for key in ["first_false", "first_true"] {
        if let Some(Json::Obj(w)) = lookup(obj, key) {
            add(unknown_count(w, &WITNESS_KEYS));
        }
    }
    total
}

fn lookup<'a>(obj: &'a Obj, key: &str) -> Option<&'a Json> {
    obj.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn field<'a>(obj: &'a Obj, key: &'static str) -> Res<&'a Json> {
    lookup(obj, key).ok_or(RejectReason::MissingField(key))
}

/// A present, non-null optional field.
fn optional<'a>(obj: &'a Obj, key: &str) -> Option<&'a Json> {
    lookup(obj, key).filter(|v| !matches!(v, Json::Null))
}

fn ident(v: &Json, name: &'static str, max: usize) -> Res<String> {
    keyed(v, name, max, is_ident_byte)
}

fn thread_key(v: &Json) -> Res<String> {
    keyed(v, "thread", MAX_THREAD_KEY_BYTES, is_thread_byte)
}

fn keyed(v: &Json, name: &'static str, max: usize, allowed: fn(u8) -> bool) -> Res<String> {
    let Json::Str(s) = v else {
        return Err(RejectReason::InvalidField(name));
    };
    if s.len() > max {
        return Err(RejectReason::OverLimit(name));
    }
    if s.is_empty() || !s.bytes().all(allowed) {
        return Err(RejectReason::InvalidField(name));
    }
    Ok(s.clone())
}

fn text(v: &Json, name: &'static str, max: usize) -> Res<String> {
    let Json::Str(s) = v else {
        return Err(RejectReason::InvalidField(name));
    };
    if s.len() > max {
        return Err(RejectReason::OverLimit(name));
    }
    if s.chars().any(|c| c.is_control() || is_invisible(c)) {
        return Err(RejectReason::InvalidField(name));
    }
    Ok(s.clone())
}

fn uint(v: &Json, name: &'static str) -> Res<u64> {
    match v {
        Json::Num(Num::Unsigned(n)) => Ok(*n),
        _ => Err(RejectReason::InvalidField(name)),
    }
}

/// Ordinals start at 1.
fn ordinal(v: &Json) -> Res<u64> {
    match uint(v, "ord")? {
        0 => Err(RejectReason::InvalidField("ord")),
        n => Ok(n),
    }
}

fn int(v: &Json, name: &'static str) -> Res<i64> {
    match v {
        Json::Num(Num::Signed(n)) => Ok(*n),
        Json::Num(Num::Unsigned(n)) => {
            i64::try_from(*n).map_err(|_| RejectReason::InvalidField(name))
        }
        _ => Err(RejectReason::InvalidField(name)),
    }
}

fn boolean(v: &Json, name: &'static str) -> Res<bool> {
    match v {
        Json::Bool(b) => Ok(*b),
        _ => Err(RejectReason::InvalidField(name)),
    }
}

fn kind_of(v: &Json) -> Res<AssertKind> {
    match v {
        Json::Str(s) => AssertKind::from_wire(s).ok_or(RejectReason::InvalidField("type")),
        _ => Err(RejectReason::InvalidField("type")),
    }
}

/// The process tag of a real process. The eval tag is reserved so scripts cannot merge into one.
fn proc_of(obj: &Obj) -> Res<String> {
    let tag = ident(field(obj, "proc")?, "proc", MAX_PROC_BYTES)?;
    if tag == DEFAULT_EVAL_PROC {
        return Err(RejectReason::InvalidField("proc"));
    }
    Ok(tag)
}

fn decode_hello(obj: &Obj) -> Res<Event> {
    Ok(Event::Hello(Hello {
        proc: proc_of(obj)?,
        sdk: text(field(obj, "sdk")?, "sdk", MAX_SDK_STR_BYTES)?,
        sdk_version: text(field(obj, "sdk_version")?, "sdk_version", MAX_SDK_STR_BYTES)?,
        catalog_hash: text(field(obj, "catalog_hash")?, "catalog_hash", MAX_HASH_BYTES)?,
        signal_handlers: boolean(field(obj, "signal_handlers")?, "signal_handlers")?,
    }))
}

fn decode_loc(v: &Json) -> Res<Loc> {
    let Json::Obj(o) = v else {
        return Err(RejectReason::InvalidField("loc"));
    };
    Ok(Loc {
        file: text(field(o, "file")?, "file", MAX_LOC_FILE_BYTES)?,
        line: uint(field(o, "line")?, "line")?,
        func: optional(o, "func")
            .map(|f| text(f, "func", MAX_LOC_FUNC_BYTES))
            .transpose()?,
    })
}

fn decode_catalog(obj: &Obj) -> Res<Event> {
    let kind = kind_of(field(obj, "type")?)?;
    let must_hit = boolean(field(obj, "must_hit")?, "must_hit")?;
    if must_hit != kind.must_hit() {
        return Err(RejectReason::Inconsistent("must_hit"));
    }
    Ok(Event::Catalog(CatalogEntry {
        proc: proc_of(obj)?,
        id: ident(field(obj, "id")?, "id", MAX_ID_BYTES)?,
        kind,
        must_hit,
        msg: optional(obj, "msg")
            .map(|m| text(m, "msg", MAX_MSG_BYTES))
            .transpose()?
            .unwrap_or_default(),
        loc: optional(obj, "loc").map(decode_loc).transpose()?,
    }))
}

fn decode_vals(v: &Json) -> Res<[i64; VALS_LEN]> {
    let Json::Arr(items) = v else {
        return Err(RejectReason::InvalidField("vals"));
    };
    if items.len() != VALS_LEN {
        return Err(RejectReason::InvalidField("vals"));
    }
    let mut vals = [0i64; VALS_LEN];
    for (slot, item) in vals.iter_mut().zip(items) {
        *slot = int(item, "vals")?;
    }
    Ok(vals)
}

fn decode_witness(v: &Json) -> Res<WireWitness> {
    let Json::Obj(o) = v else {
        return Err(RejectReason::InvalidField("witness"));
    };
    Ok(WireWitness {
        thread: thread_key(field(o, "thread")?)?,
        ord: ordinal(field(o, "ord")?)?,
        vals: decode_vals(field(o, "vals")?)?,
    })
}

fn decode_assert(obj: &Obj) -> Res<Event> {
    let hits = uint(field(obj, "hits")?, "hits")?;
    let trues = uint(field(obj, "trues")?, "trues")?;
    let falses = uint(field(obj, "falses")?, "falses")?;
    if trues.checked_add(falses) != Some(hits) {
        return Err(RejectReason::Inconsistent("hits"));
    }
    let first_false = optional(obj, "first_false")
        .map(decode_witness)
        .transpose()?;
    let first_true = optional(obj, "first_true")
        .map(decode_witness)
        .transpose()?;
    if first_false.is_some() && falses == 0 {
        return Err(RejectReason::Inconsistent("first_false"));
    }
    if first_true.is_some() && trues == 0 {
        return Err(RejectReason::Inconsistent("first_true"));
    }
    Ok(Event::Assert(AssertReport {
        proc: proc_of(obj)?,
        id: ident(field(obj, "id")?, "id", MAX_ID_BYTES)?,
        hits,
        trues,
        falses,
        first_false,
        first_true,
    }))
}

fn decode_work(obj: &Obj) -> Res<Event> {
    Ok(Event::Work(Work {
        proc: proc_of(obj)?,
        ops: uint(field(obj, "ops")?, "ops")?,
    }))
}

fn decode_thread(obj: &Obj) -> Res<Event> {
    Ok(Event::Thread(ThreadLine {
        proc: proc_of(obj)?,
        thread: ThreadReport {
            thread: thread_key(field(obj, "thread")?)?,
            evals: uint(field(obj, "evals")?, "evals")?,
            tape_consumed: uint(field(obj, "tape_consumed")?, "tape_consumed")?,
            key_stable: optional(obj, "key_stable")
                .map(|b| boolean(b, "key_stable"))
                .transpose()?
                .unwrap_or(true),
        },
    }))
}

fn thread_count(v: &Json) -> Res<u64> {
    let n = uint(v, "threads")?;
    if n > MAX_THREADS_PER_PROC as u64 {
        return Err(RejectReason::OverLimit("threads"));
    }
    Ok(n)
}

fn decode_end(obj: &Obj) -> Res<Event> {
    let Json::Str(reason) = field(obj, "reason")? else {
        return Err(RejectReason::InvalidField("reason"));
    };
    Ok(Event::End(End {
        proc: proc_of(obj)?,
        reason: EndReason::from_wire(reason).ok_or(RejectReason::InvalidField("reason"))?,
        status: int(field(obj, "status")?, "status")?,
        sites: uint(field(obj, "sites")?, "sites")?,
        evals: uint(field(obj, "evals")?, "evals")?,
        dropped: uint(field(obj, "dropped")?, "dropped")?,
        threads: thread_count(field(obj, "threads")?)?,
    }))
}

/// Eval lines always belong to the reserved tag, so they can never merge into an SDK process.
fn eval_proc(obj: &Obj) -> Res<String> {
    match optional(obj, "proc") {
        None => Ok(DEFAULT_EVAL_PROC.to_owned()),
        Some(v) => match ident(v, "proc", MAX_PROC_BYTES)? {
            tag if tag == DEFAULT_EVAL_PROC => Ok(tag),
            _ => Err(RejectReason::InvalidField("proc")),
        },
    }
}

fn decode_eval(obj: &Obj) -> Res<Event> {
    Ok(Event::Eval(Eval {
        proc: eval_proc(obj)?,
        id: ident(field(obj, "id")?, "id", MAX_ID_BYTES)?,
        kind: kind_of(field(obj, "type")?)?,
        ok: boolean(field(obj, "ok")?, "ok")?,
    }))
}

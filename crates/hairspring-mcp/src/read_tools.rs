//! Read tools for a run's properties and trials.
//!
//! These answers go straight into a model's context, so two rules apply. Every string from the program under test sits
//! under a `guest` object (the service puts it there), and the server's instructions say that object is untrusted. And
//! every answer is bounded: a page that would be too big is re-asked with a smaller limit, so its cursor stays correct,
//! and the answer says it was cut.
use crate::tools::Failed;
use crate::validate::*;
use crate::McpServer;
use hairspring_service::{PropertyQuery, SeedQuery, TrialQuery};
use serde_json::{json, Value};

/// Most bytes one answer may take in the reply: 64 KiB less room for the envelope. The reply carries the payload
/// twice (as structured content and as escaped text), and [`reply_cost`] counts both.
const MAX_REPLY_BYTES: usize = 62 * 1024;
const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;
const MAX_SEED_LIMIT: u64 = 500;

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn limit_schema(max: u64) -> Value {
    json!({ "type": "integer", "minimum": 1, "maximum": max, "description": "Rows per page; a page that would be too large is cut and says so" })
}

pub(crate) fn properties_schema() -> Value {
    obj(
        json!({
            "run_id": { "type": "string" },
            "status": { "type": "string", "enum": ["violation", "unmet", "unrated", "pass"] },
            "kind": { "type": "string", "enum": ["always", "always_or_unreachable", "sometimes", "reachable", "unreachable"] },
            "q": { "type": "string", "description": "Prefix of the property id (at most 128 bytes)" },
            "sort": { "type": "string", "enum": ["status", "id", "rate", "evals"] },
            "cursor": { "type": "string", "description": "next_cursor from the previous page" },
            "limit": limit_schema(MAX_LIMIT)
        }),
        &["run_id"],
    )
}

pub(crate) fn property_schema() -> Value {
    obj(
        json!({
            "run_id": { "type": "string" },
            "idx": { "type": "integer", "minimum": 0, "description": "Property index from get_properties" },
            "seeds": { "type": "string", "enum": ["failing", "hit", "never"], "description": "Which seeds to list (default failing)" },
            "cursor": { "type": "string" },
            "limit": limit_schema(MAX_SEED_LIMIT)
        }),
        &["run_id", "idx"],
    )
}

pub(crate) fn trials_schema() -> Value {
    obj(
        json!({
            "run_id": { "type": "string" },
            "verdict": { "type": "string", "enum": ["pass", "violation", "inconclusive", "unusable", "infra"] },
            "fails": { "type": "integer", "minimum": 0, "description": "Only trials in which this property index failed" },
            "hits": { "type": "integer", "minimum": 0, "description": "Only trials that reached this property index" },
            "misses": { "type": "integer", "minimum": 0, "description": "Only usable trials that did not reach this property index" },
            "degraded": { "type": "boolean" },
            "perturbing": { "type": "boolean" },
            "fp": { "type": "string", "description": "Schedule fingerprint" },
            "cursor": { "type": "string" },
            "limit": limit_schema(MAX_LIMIT)
        }),
        &["run_id"],
    )
}

pub(crate) fn trial_schema() -> Value {
    obj(
        json!({ "run_id": { "type": "string" }, "seq": { "type": "integer", "minimum": 0 } }),
        &["run_id", "seq"],
    )
}

/// What `v` costs in a reply: once as JSON and once as an escaped string.
fn reply_cost(v: &Value) -> usize {
    let text = v.to_string();
    text.len() + Value::String(text).to_string().len()
}

fn run_arg(args: &Value) -> Result<&str, Failed> {
    let id = req_str(args, "run_id")?;
    if run_id(id) {
        Ok(id)
    } else {
        Err(Failed::Params("`run_id` has an invalid format".to_string()))
    }
}

fn page_limit(args: &Value, max: u64) -> Result<u32, Failed> {
    Ok(opt_u64(args, "limit")?
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, max) as u32)
}

fn opt_number(args: &Value, key: &str) -> Result<Option<String>, Failed> {
    Ok(opt_u64(args, key)?.map(|n| n.to_string()))
}

fn opt_flag(args: &Value, key: &str) -> Result<Option<String>, Failed> {
    Ok(opt_bool(args, key)?.map(|b| b.to_string()))
}

fn text(args: &Value, key: &str) -> Result<Option<String>, Failed> {
    Ok(opt_str(args, key)?.map(str::to_owned))
}

fn to_value<T: serde::Serialize>(v: Option<T>, what: &str) -> Result<Value, Failed> {
    let v = v.ok_or_else(|| Failed::Tool(format!("no such {what}")))?;
    Ok(serde_json::to_value(v).unwrap_or_default())
}

/// Asks for a page with `limit` rows, and with fewer if the answer would be too large. The smaller page is asked for
/// again rather than trimmed, so its cursor is exactly where the rows stop.
fn fit(requested: u32, ask: impl Fn(u32) -> Result<Value, Failed>) -> Result<Value, Failed> {
    let mut limit = requested;
    loop {
        let mut v = ask(limit)?;
        if reply_cost(&v) <= MAX_REPLY_BYTES || limit <= 1 {
            if limit < requested {
                v["truncated"] = json!(true);
                v["truncated_note"] = json!(format!(
                    "limit reduced from {requested} to {limit} to keep the whole response under 64 KiB; continue with next_cursor"
                ));
            }
            return Ok(v);
        }
        limit = (limit / 2).max(1);
    }
}

fn properties(server: &McpServer, args: &Value) -> Result<Value, Failed> {
    let id = run_arg(args)?;
    let query = |limit: u32| PropertyQuery {
        status: text(args, "status").ok().flatten(),
        kind: text(args, "kind").ok().flatten(),
        q: text(args, "q").ok().flatten(),
        sort: text(args, "sort").ok().flatten(),
        cursor: text(args, "cursor").ok().flatten(),
        limit: Some(limit),
    };
    // Check the argument types once, so the closure above can read them without a second error path.
    for key in ["status", "kind", "q", "sort", "cursor"] {
        opt_str(args, key)?;
    }
    let limit = page_limit(args, MAX_LIMIT)?;
    fit(limit, |l| {
        to_value(server.service().property_page(id, &query(l), None)?, "run")
    })
}

fn property(server: &McpServer, args: &Value) -> Result<Value, Failed> {
    let id = run_arg(args)?;
    let idx = opt_u64(args, "idx")?.ok_or_else(|| {
        Failed::Params("`idx` is required and must be a non-negative integer".into())
    })?;
    let idx = u32::try_from(idx).map_err(|_| Failed::Params("`idx` is out of range".into()))?;
    for key in ["seeds", "cursor"] {
        opt_str(args, key)?;
    }
    let limit = page_limit(args, MAX_SEED_LIMIT)?;
    fit(limit, |l| {
        let q = SeedQuery {
            seeds: text(args, "seeds").ok().flatten(),
            cursor: text(args, "cursor").ok().flatten(),
            limit: Some(l),
        };
        to_value(server.service().property_detail(id, idx, &q)?, "property")
    })
}

fn trials(server: &McpServer, args: &Value) -> Result<Value, Failed> {
    let id = run_arg(args)?;
    let strings = ["verdict", "fp", "cursor"].map(|k| text(args, k));
    let numbers = ["fails", "hits", "misses"].map(|k| opt_number(args, k));
    let flags = ["degraded", "perturbing"].map(|k| opt_flag(args, k));
    let [verdict, fp, cursor] = strings;
    let [fails, hits, misses] = numbers;
    let [degraded, perturbing] = flags;
    let (verdict, fp, cursor) = (verdict?, fp?, cursor?);
    let (fails, hits, misses) = (fails?, hits?, misses?);
    let (degraded, perturbing) = (degraded?, perturbing?);
    let limit = page_limit(args, MAX_LIMIT)?;
    fit(limit, |l| {
        let q = TrialQuery {
            verdict: verdict.clone(),
            fails: fails.clone(),
            hits: hits.clone(),
            misses: misses.clone(),
            degraded: degraded.clone(),
            perturbing: perturbing.clone(),
            fp: fp.clone(),
            cursor: cursor.clone(),
            limit: Some(l),
        };
        to_value(server.service().trial_page(id, &q)?, "run")
    })
}

fn trial(server: &McpServer, args: &Value) -> Result<Value, Failed> {
    let id = run_arg(args)?;
    let seq = opt_u64(args, "seq")?.ok_or_else(|| {
        Failed::Params("`seq` is required and must be a non-negative integer".into())
    })?;
    let mut v = to_value(server.service().trial_detail(id, seq)?, "trial")?;
    if reply_cost(&v) > MAX_REPLY_BYTES {
        v["guest"]["detail"] = Value::Null;
        v["detail_omitted"] = json!(true);
        v["detail_omitted_note"] = json!(
            "the kept detail is too large for one tool answer; download it with the REST endpoint for this trial"
        );
    }
    Ok(v)
}

/// Runs one of the read tools. `None` when `name` is not one of them.
pub(crate) fn call(server: &McpServer, name: &str, args: &Value) -> Option<Result<Value, Failed>> {
    Some(match name {
        "get_properties" => properties(server, args),
        "get_property" => property(server, args),
        "list_trials" => trials(server, args),
        "get_trial" => trial(server, args),
        _ => return None,
    })
}

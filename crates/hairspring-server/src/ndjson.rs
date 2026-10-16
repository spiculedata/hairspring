//! Content negotiation for list endpoints: `Accept: application/x-ndjson` asks for one JSON object per line.
//!
//! The header is the only switch. Anything else (no header, another type, a wildcard, a tie with JSON, a malformed
//! value) gets the JSON form, so existing clients are unaffected. The NDJSON body is built from the same value the
//! JSON form would send, after the same role, validation and rate-limit checks, so it carries the same guest fencing.
//! Errors never reach this module: they are `ApiError`s and stay JSON.
use crate::error::ApiError;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hairspring_service::ndjson::Listing;
use serde_json::Value;

const NDJSON_TYPE: &str = "application/x-ndjson";
const NDJSON_CONTENT_TYPE: &str = "application/x-ndjson; charset=utf-8";
const JSON_TYPE: &str = "application/json";

/// The weight an entry's parameters give its type: `1.0` without a `q`, and `0.0` for a `q` that is not in 0..=1.
fn weight(params: &str) -> f32 {
    match params.split(';').find_map(|p| p.trim().strip_prefix("q=")) {
        None => 1.0,
        Some(q) => q
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|w| (0.0..=1.0).contains(w))
            .unwrap_or(0.0),
    }
}

/// The highest weight any `Accept` entry gives to exactly this type.
fn best_weight(headers: &HeaderMap, wanted: &str) -> f32 {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|entry| {
            let (media, params) = entry.split_once(';').unwrap_or((entry, ""));
            media
                .trim()
                .eq_ignore_ascii_case(wanted)
                .then(|| weight(params))
        })
        .fold(0.0, f32::max)
}

/// True when the caller asked for NDJSON more strongly than for JSON. A tie is JSON.
pub fn wants_ndjson(headers: &HeaderMap) -> bool {
    let ndjson = best_weight(headers, NDJSON_TYPE);
    ndjson > 0.0 && ndjson > best_weight(headers, JSON_TYPE)
}

/// Answers a list in the form the caller asked for. `items_key` names the array of items in the JSON form.
pub fn list_response(
    headers: &HeaderMap,
    value: Value,
    items_key: &str,
) -> Result<Response, ApiError> {
    let mut response = if wants_ndjson(headers) {
        let listing = Listing::from_json(value, items_key)
            .ok_or_else(|| ApiError::internal(format!("`{items_key}` is not a list")))?;
        (
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static(NDJSON_CONTENT_TYPE),
            )],
            listing.encode(),
        )
            .into_response()
    } else {
        Json(value).into_response()
    };
    // Either form depends on the request's Accept header, so a cache must key on it.
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Accept"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(header::ACCEPT, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn only_a_stronger_ndjson_entry_selects_it() {
        for (values, want) in [
            (vec![], false),
            (vec!["application/x-ndjson"], true),
            (vec!["APPLICATION/X-NDJSON; charset=utf-8"], true),
            (vec!["*/*"], false),
            (vec!["application/*"], false),
            (vec!["application/json"], false),
            (vec!["application/json, application/x-ndjson"], false),
            (vec!["application/json;q=0.5, application/x-ndjson"], true),
            (vec!["application/x-ndjson;q=0"], false),
            (vec!["application/x-ndjson;q=oops"], false),
            (vec!["application/x-ndjson;q=2"], false),
            (vec!["text/html, application/x-ndjson;q=0.1"], true),
            (vec!["text/html", "application/x-ndjson"], true),
            (vec!["application/x-ndjsonx"], false),
            (vec![";;;,,,"], false),
        ] {
            assert_eq!(wants_ndjson(&accept(&values)), want, "{values:?}");
        }
    }
}

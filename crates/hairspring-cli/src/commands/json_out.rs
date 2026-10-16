//! `--json` output for `results` and `show`: NDJSON in the same form the REST list endpoints answer with.
//!
//! Every stdout line is one JSON object with keys in sorted order, and the last line is the control line
//! `{"_next":null}` (the CLI always prints a complete list, so there is never a next page). Nothing else is printed to
//! stdout; failures go to stderr through the usual error path. Item lines are the objects the REST API serves, so
//! guest text is under `guest` exactly as it is there.
use crate::ctx::CmdResult;
use hairspring_service::ndjson::Listing;
use hairspring_service::{PropertyQuery, RunDetail, Service, MAX_PAGE};
use serde_json::{json, Map, Value};
use std::io::Write;

/// Marks the lines of `show --json`, which are not all the same kind. Sorts before every other key.
const KIND_KEY: &str = "_kind";
/// A stuck cursor must not loop forever; a run's catalog is far smaller than this many pages.
const MAX_PROPERTY_PAGES: usize = 1000;

/// One line per run, newest first: the same objects as `GET /api/runs`.
pub fn results(svc: &Service, limit: u32, out: &mut dyn Write) -> CmdResult<()> {
    let runs = svc.list_runs(limit)?;
    let listing = Listing::from_json(json!({ "runs": runs }), "runs")
        .ok_or("cannot list the runs as NDJSON")?;
    write!(out, "{}", listing.encode())?;
    Ok(())
}

/// The run (`_kind: "run"`: the run, its failures and its property summary, plus the property table's page-level
/// fields under `page`), then one `_kind: "property"` line per property.
pub fn show(svc: &Service, detail: &RunDetail, out: &mut dyn Write) -> CmdResult<()> {
    let (page, rows) = property_rows(svc, &detail.run.id)?;
    let head = run_line(detail, page)?;
    let items: Vec<Value> = std::iter::once(head)
        .chain(rows.into_iter().map(|row| tagged(row, "property")))
        .collect();
    let listing = Listing {
        items,
        next: None,
        page: Map::new(),
    };
    write!(out, "{}", listing.encode())?;
    Ok(())
}

fn run_line(detail: &RunDetail, page: Map<String, Value>) -> CmdResult<Value> {
    let mut fields = match serde_json::to_value(detail)? {
        Value::Object(fields) => fields,
        _ => return Err("a run did not serialise as an object".into()),
    };
    if !page.is_empty() {
        fields.insert("page".to_owned(), Value::Object(page));
    }
    Ok(tagged(Value::Object(fields), "run"))
}

fn tagged(value: Value, kind: &str) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            std::iter::once((KIND_KEY.to_owned(), Value::from(kind)))
                .chain(fields)
                .collect(),
        ),
        other => other,
    }
}

/// Every property row, page by page, and the first page's page-level fields (claim, notes, totals).
fn property_rows(svc: &Service, run_id: &str) -> CmdResult<(Map<String, Value>, Vec<Value>)> {
    let mut rows = Vec::new();
    let mut page_fields = Map::new();
    let mut cursor: Option<String> = None;
    for index in 0..MAX_PROPERTY_PAGES {
        let query = PropertyQuery {
            cursor: cursor.take(),
            limit: Some(MAX_PAGE),
            ..PropertyQuery::default()
        };
        let Some(page) = svc.property_page(run_id, &query, None)? else {
            break;
        };
        let listing = Listing::from_json(serde_json::to_value(page)?, "properties")
            .ok_or("cannot read the property table")?;
        if index == 0 {
            page_fields = listing.page;
        }
        rows.extend(listing.items);
        match listing.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok((page_fields, rows))
}

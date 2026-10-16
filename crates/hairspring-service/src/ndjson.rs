//! The NDJSON form of a list: one JSON object per item, then one control line.
//!
//! Every line is a complete JSON object ending in a line feed. Item lines are exactly the objects the JSON form
//! lists. The last line is always the control line `{"_next": <cursor or null>}`: a string cursor means more pages
//! follow, `null` means the list is complete, and a stream that does not end in a control line was cut short. Page
//! level fields of the JSON form (a total, a claim, notes) ride on the control line under `page`, so item lines stay
//! uniform. Guest text is untouched here: it stays wherever the item put it, under `guest`.
use serde_json::{Map, Value};

/// The only key a control line must have.
pub const NEXT_KEY: &str = "_next";
/// The control line's key for page-level fields.
pub const PAGE_KEY: &str = "page";
/// The key a paged JSON form keeps its cursor under.
const CURSOR_KEY: &str = "next_cursor";

/// A list taken apart: its items, the cursor for the next page, and everything else the JSON form carried.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub items: Vec<Value>,
    pub next: Option<String>,
    pub page: Map<String, Value>,
}

impl Listing {
    /// Splits a JSON-form list whose items are the array under `items_key`. `None` when it is not that shape.
    pub fn from_json(value: Value, items_key: &str) -> Option<Listing> {
        let Value::Object(mut fields) = value else {
            return None;
        };
        let Value::Array(items) = fields.remove(items_key)? else {
            return None;
        };
        let next = match fields.remove(CURSOR_KEY) {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) => Some(cursor),
            Some(_) => return None,
        };
        Some(Listing {
            items,
            next,
            page: fields,
        })
    }

    /// The control line: `_next` first, then `page` when there are page-level fields.
    pub fn control_line(&self) -> Value {
        let mut line = Map::new();
        line.insert(NEXT_KEY.to_owned(), self.next.clone().into());
        if !self.page.is_empty() {
            line.insert(PAGE_KEY.to_owned(), Value::Object(self.page.clone()));
        }
        Value::Object(line)
    }

    /// The whole body: item lines, then the control line, each ending in a line feed.
    pub fn encode(&self) -> String {
        self.items
            .iter()
            .chain(std::iter::once(&self.control_line()))
            .map(|line| format!("{line}\n"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn items_come_first_and_the_control_line_closes_the_list() {
        let l = Listing::from_json(json!({"runs": [{"id": "a"}, {"id": "b"}]}), "runs").unwrap();
        assert_eq!(
            l.encode(),
            "{\"id\":\"a\"}\n{\"id\":\"b\"}\n{\"_next\":null}\n"
        );
    }

    #[test]
    fn an_empty_list_is_just_the_control_line() {
        let l = Listing::from_json(json!({"runs": []}), "runs").unwrap();
        assert_eq!(l.encode(), "{\"_next\":null}\n");
    }

    #[test]
    fn a_cursor_means_more_pages() {
        let v = json!({"trials": [{"seq": 1}], "next_cursor": "abc"});
        let l = Listing::from_json(v, "trials").unwrap();
        assert_eq!(l.encode(), "{\"seq\":1}\n{\"_next\":\"abc\"}\n");
    }

    #[test]
    fn page_level_fields_move_to_the_control_line() {
        let v = json!({"properties": [{"n": 1}], "total": 9, "next_cursor": null});
        let l = Listing::from_json(v, "properties").unwrap();
        assert_eq!(
            l.encode(),
            "{\"n\":1}\n{\"_next\":null,\"page\":{\"total\":9}}\n"
        );
    }

    #[test]
    fn guest_text_with_line_breaks_stays_on_one_line() {
        let v = json!({"trials": [{"guest": {"reason": "a\nb\r\u{2028}{\"_next\":1}"}}]});
        let body = Listing::from_json(v, "trials").unwrap().encode();
        assert_eq!(body.matches('\n').count(), 2);
        assert!(!body.contains('\r'));
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_refused() {
        assert!(Listing::from_json(json!([1]), "runs").is_none());
        assert!(Listing::from_json(json!({"other": []}), "runs").is_none());
        assert!(Listing::from_json(json!({"runs": 1}), "runs").is_none());
        assert!(Listing::from_json(json!({"runs": [], "next_cursor": 5}), "runs").is_none());
    }
}

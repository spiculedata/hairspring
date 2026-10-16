//! A strict JSON value: duplicate object keys and over-deep nesting are errors, never silently resolved.

use crate::limits::MAX_JSON_DEPTH;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Num {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(Num),
    Str(String),
    Arr(Vec<Json>),
    /// Entries in document order; keys are unique.
    Obj(Vec<(String, Json)>),
}

/// Why strict parsing failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonError {
    Malformed,
    DuplicateKey,
    TooDeep,
}

const DUPLICATE_KEY_MARK: &str = "hairspring:duplicate-key";

impl Json {
    /// Nesting depth: a scalar is 0, an object or array containing scalars is 1.
    fn depth(&self) -> usize {
        match self {
            Json::Arr(items) => 1 + items.iter().map(Json::depth).max().unwrap_or(0),
            Json::Obj(entries) => 1 + entries.iter().map(|(_, v)| v.depth()).max().unwrap_or(0),
            _ => 0,
        }
    }
}

/// Parse one JSON document from UTF-8 text. Trailing non-whitespace is an error.
pub fn parse_strict(text: &str) -> Result<Json, JsonError> {
    let value: Json = serde_json::from_str(text).map_err(|e| {
        if e.to_string().contains(DUPLICATE_KEY_MARK) {
            JsonError::DuplicateKey
        } else {
            JsonError::Malformed
        }
    })?;
    if value.depth() > MAX_JSON_DEPTH {
        return Err(JsonError::TooDeep);
    }
    Ok(value)
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Json, E> {
        Ok(Json::Bool(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Json, E> {
        Ok(Json::Num(Num::Unsigned(v)))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Json, E> {
        Ok(Json::Num(Num::Signed(v)))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Json, E> {
        Ok(Json::Num(Num::Float(v)))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Json, E> {
        Ok(Json::Str(v.to_owned()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Json, E> {
        Ok(Json::Str(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Json>()? {
            items.push(item);
        }
        Ok(Json::Arr(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut entries: Vec<(String, Json)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if entries.iter().any(|(k, _)| *k == key) {
                return Err(de::Error::custom(DUPLICATE_KEY_MARK));
            }
            entries.push((key, map.next_value::<Json>()?));
        }
        Ok(Json::Obj(entries))
    }
}

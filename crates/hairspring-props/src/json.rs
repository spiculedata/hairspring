//! A strict JSON value: duplicate object keys and over-deep nesting are errors, never silently resolved.
//!
//! Both checks happen inside a `DeserializeSeed`, which records the reason in a side channel. Nothing depends on the
//! wording of `serde_json` error messages, and nesting is cut off at `MAX_JSON_DEPTH` long before the library's own
//! recursion limit.

use crate::limits::MAX_JSON_DEPTH;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::cell::Cell;
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

/// Parse one JSON document from UTF-8 text. Trailing non-whitespace is an error.
pub fn parse_strict(text: &str) -> Result<Json, JsonError> {
    let failure = Cell::new(None);
    let seed = Seed {
        depth: 0,
        failure: &failure,
    };
    let mut de = serde_json::Deserializer::from_str(text);
    let value = seed.deserialize(&mut de).and_then(|v| de.end().map(|()| v));
    match value {
        Ok(v) => Ok(v),
        Err(_) => Err(failure.get().unwrap_or(JsonError::Malformed)),
    }
}

#[derive(Clone, Copy)]
struct Seed<'a> {
    /// Containers entered so far on the path to this value.
    depth: usize,
    failure: &'a Cell<Option<JsonError>>,
}

impl Seed<'_> {
    fn fail<E: de::Error>(&self, why: JsonError) -> E {
        self.failure.set(Some(why));
        E::custom("strict json")
    }

    fn enter<E: de::Error>(&self) -> Result<Seed<'_>, E> {
        if self.depth + 1 > MAX_JSON_DEPTH {
            return Err(self.fail(JsonError::TooDeep));
        }
        Ok(Seed {
            depth: self.depth + 1,
            failure: self.failure,
        })
    }
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Json;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
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
        let inner = self.enter()?;
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(inner)? {
            items.push(item);
        }
        Ok(Json::Arr(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let inner = self.enter()?;
        let mut entries: Vec<(String, Json)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if entries.iter().any(|(k, _)| *k == key) {
                return Err(self.fail(JsonError::DuplicateKey));
            }
            entries.push((key, map.next_value_seed(inner)?));
        }
        Ok(Json::Obj(entries))
    }
}

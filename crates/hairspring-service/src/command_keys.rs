//! Parses a command into a JSON value while noting whether any object repeats a key.
//!
//! `serde_json::Value` keeps the last of a repeated key and says nothing, so two commands that differ only in a
//! repeated key would read as one. This parser reports the repeat instead. The caller bounds the size and nesting
//! first, so the recursion here is shallow.
use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::cell::Cell;
use std::collections::HashSet;
use std::fmt;

/// How parsing ended.
#[derive(Debug, PartialEq)]
pub(crate) enum KeyedParse {
    Parsed(Value),
    RepeatedKey,
    NotJson,
}

/// Parses one JSON document, refusing a repeated key in any object.
pub(crate) fn parse_without_repeated_keys(text: &str) -> KeyedParse {
    let repeated = Cell::new(false);
    let mut de = serde_json::Deserializer::from_str(text);
    let parsed = Seed(&repeated)
        .deserialize(&mut de)
        .and_then(|v| de.end().map(|()| v));
    match parsed {
        Ok(v) => KeyedParse::Parsed(v),
        // The parse stops at the first repeat, so text that is malformed further on is still checked as JSON first.
        Err(_) if repeated.get() && serde_json::from_str::<Value>(text).is_ok() => {
            KeyedParse::RepeatedKey
        }
        Err(_) => KeyedParse::NotJson,
    }
}

struct Seed<'a>(&'a Cell<bool>);

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Number::from_f64(v).map_or(Value::Null, Value::Number))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Seed(self.0))? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut seen = HashSet::new();
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                self.0.set(true);
                return Err(serde::de::Error::custom("repeated key"));
            }
            out.insert(key, map.next_value_seed(Seed(self.0))?);
        }
        Ok(Value::Object(out))
    }
}

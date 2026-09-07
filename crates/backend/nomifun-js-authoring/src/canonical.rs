use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    DigestHex, canonical_json_bytes as platform_canonical_json_bytes,
    digest_payload,
};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Number, Value};

use crate::error::AuthoringError;

pub(crate) fn canonical_json_bytes<T>(value: &T) -> Result<Vec<u8>, AuthoringError>
where
    T: Serialize,
{
    platform_canonical_json_bytes(value)
        .map_err(|error| AuthoringError::CanonicalSerialization(error.to_string()))
}

pub(crate) fn canonical_digest<T>(value: &T) -> Result<DigestHex, AuthoringError>
where
    T: Serialize,
{
    digest_payload(value)
        .map_err(|error| AuthoringError::CanonicalSerialization(error.to_string()))
}

pub(crate) fn strict_json_from_slice<T>(bytes: &[u8]) -> Result<T, AuthoringError>
where
    T: for<'de> Deserialize<'de>,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictJsonValueSeed
        .deserialize(&mut deserializer)
        .map_err(|error| AuthoringError::InvalidPackageJson(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| AuthoringError::InvalidPackageJson(error.to_string()))?;
    serde_json::from_value(value)
        .map_err(|error| AuthoringError::InvalidPackageJson(error.to_string()))
}

struct StrictJsonValueSeed;

impl<'de> DeserializeSeed<'de> for StrictJsonValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonValueVisitor)
    }
}

struct StrictJsonValueVisitor;

impl<'de> Visitor<'de> for StrictJsonValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("strict JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictJsonValueSeed.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictJsonValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate decoded JSON object key: {key}"
                )));
            }
            let value = map.next_value_seed(StrictJsonValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

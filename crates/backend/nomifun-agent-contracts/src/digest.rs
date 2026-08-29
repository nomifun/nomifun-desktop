use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::DigestHex;

pub const CANONICAL_JSON_ALGORITHM: &str = "sorted-json-sha256-v1";

#[derive(Debug, Error)]
pub enum CanonicalDigestError {
    #[error("contract payload cannot be serialized: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("non-finite JSON numbers are not canonical contract values")]
    NonFiniteNumber,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactEnvelope<T> {
    pub digest_algorithm: String,
    pub payload: T,
    pub payload_digest: DigestHex,
}

impl<T> ArtifactEnvelope<T>
where
    T: Serialize,
{
    pub fn new(payload: T) -> Result<Self, CanonicalDigestError> {
        let payload_digest = digest_payload(&payload)?;
        Ok(Self {
            digest_algorithm: CANONICAL_JSON_ALGORITHM.to_owned(),
            payload,
            payload_digest,
        })
    }

    pub fn verify(&self) -> Result<bool, CanonicalDigestError> {
        Ok(self.digest_algorithm == CANONICAL_JSON_ALGORITHM
            && self.payload_digest == digest_payload(&self.payload)?)
    }
}

pub fn digest_payload<T: Serialize>(payload: &T) -> Result<DigestHex, CanonicalDigestError> {
    let bytes = canonical_json_bytes(payload)?;
    Ok(digest_bytes(&bytes))
}

pub fn digest_bytes(bytes: &[u8]) -> DigestHex {
    DigestHex(hex::encode(Sha256::digest(bytes)))
}

pub fn canonical_json_bytes<T: Serialize>(payload: &T) -> Result<Vec<u8>, CanonicalDigestError> {
    let value = serde_json::to_value(payload)?;
    let canonical = canonicalize_value(value)?;
    Ok(serde_json::to_vec(&canonical)?)
}

fn canonicalize_value(value: Value) -> Result<Value, CanonicalDigestError> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(canonicalize_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| Ok((key, canonicalize_value(value)?)))
            .collect::<Result<BTreeMap<_, _>, CanonicalDigestError>>()
            .map(|values| Value::Object(values.into_iter().collect())),
        Value::Number(number) => {
            if number.is_f64() && number.as_f64().is_none_or(|value| !value.is_finite()) {
                Err(CanonicalDigestError::NonFiniteNumber)
            } else {
                Ok(Value::Number(number))
            }
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    #[derive(Serialize)]
    struct Fixture {
        z: u8,
        a: Value,
    }

    #[test]
    fn canonical_json_sorts_every_object_key() {
        let fixture = Fixture {
            z: 1,
            a: json!({"z": 2, "a": 3}),
        };
        assert_eq!(
            canonical_json_bytes(&fixture).unwrap(),
            br#"{"a":{"a":3,"z":2},"z":1}"#
        );
    }

    #[test]
    fn envelope_digest_excludes_itself() {
        let envelope = ArtifactEnvelope::new(json!({"contract": "v2"})).unwrap();
        assert!(envelope.verify().unwrap());
    }
}

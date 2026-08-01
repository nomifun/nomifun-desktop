use std::borrow::Cow;

use nomifun_common::{ProviderId, ProviderWithModel};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};

const UUID_V7_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";

fn uuid_v7_schema(description: &str) -> Schema {
    schemars::json_schema!({
        "type": "string",
        "pattern": UUID_V7_PATTERN,
        "description": description
    })
}

pub(crate) fn canonical_uuid_v7_schema(_generator: &mut SchemaGenerator) -> Schema {
    uuid_v7_schema("Canonical lowercase hyphenated UUIDv7.")
}

pub(crate) fn optional_canonical_uuid_v7_schema(generator: &mut SchemaGenerator) -> Schema {
    let _ = generator;
    schemars::json_schema!({
        "type": ["string", "null"],
        "pattern": UUID_V7_PATTERN,
        "description": "Canonical lowercase hyphenated UUIDv7 or null."
    })
}

pub(crate) fn canonical_uuid_v7_array_schema(generator: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "type": "array",
        "items": canonical_uuid_v7_schema(generator)
    })
}

pub(crate) fn optional_canonical_uuid_v7_array_schema(
    generator: &mut SchemaGenerator,
) -> Schema {
    schemars::json_schema!({
        "type": ["array", "null"],
        "items": canonical_uuid_v7_schema(generator)
    })
}

/// Canonical bare UUIDv7 for an entity whose concrete domain is selected by a
/// separate discriminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalEntityId(String);

impl CanonicalEntityId {
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for CanonicalEntityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        nomifun_common::validate_uuidv7(&value).map_err(serde::de::Error::custom)?;
        Ok(Self(value))
    }
}

impl JsonSchema for CanonicalEntityId {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "CanonicalUuidV7".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        uuid_v7_schema("Canonical lowercase hyphenated UUIDv7.")
    }
}

/// Reject empty / untrimmed model natural keys at deserialization time.
pub(crate) fn deserialize_model_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.is_empty() || value.trim() != value {
        return Err(serde::de::Error::custom(
            "model must be a non-empty trimmed natural key",
        ));
    }
    Ok(value)
}

/// A `{ provider_id, model }` pair — the shared shape every capability domain
/// uses to reference a concrete model on a provider. Each domain keeps its own
/// `From<ModelRefParam>` conversion into its service type.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRefParam {
    /// Provider id (canonical UUIDv7 from the providers catalog).
    #[schemars(schema_with = "canonical_uuid_v7_schema")]
    pub(crate) provider_id: ProviderId,
    /// Model id/name available on that provider.
    #[serde(deserialize_with = "deserialize_model_name")]
    pub(crate) model: String,
}

impl From<ModelRefParam> for ProviderWithModel {
    fn from(value: ModelRefParam) -> Self {
        Self {
            provider_id: value.provider_id.into_string(),
            model: value.model,
            use_model: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SessionTargetKind {
    Conversation,
    Terminal,
}

impl From<SessionTargetKind> for nomifun_api_types::AutoWorkTargetKind {
    fn from(value: SessionTargetKind) -> Self {
        match value {
            SessionTargetKind::Conversation => Self::Conversation,
            SessionTargetKind::Terminal => Self::Terminal,
        }
    }
}

impl From<SessionTargetKind> for nomifun_api_types::IdmmTargetKind {
    fn from(value: SessionTargetKind) -> Self {
        match value {
            SessionTargetKind::Conversation => Self::Conversation,
            SessionTargetKind::Terminal => Self::Terminal,
        }
    }
}

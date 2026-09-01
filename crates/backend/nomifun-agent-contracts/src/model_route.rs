use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    CanonicalDigestError, ConnectionConfigRef, DigestHex, ModelRouteId, canonical_json_bytes,
};

pub const CHAT_ROUTE_RECORD_SCHEMA_V1: &str = "nomifun.chat-route-record.v1";
pub const CHAT_MODEL_TASK_AGENT_CHAT: &str = "agent_chat";

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ChatRouteRecordSchema {
    #[serde(rename = "nomifun.chat-route-record.v1")]
    V1,
}

impl ChatRouteRecordSchema {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => CHAT_ROUTE_RECORD_SCHEMA_V1,
        }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ChatRouteTask {
    #[serde(rename = "agent_chat")]
    AgentChat,
}

impl ChatRouteTask {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentChat => CHAT_MODEL_TASK_AGENT_CHAT,
        }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChatRouteProtocol {
    Anthropic,
    OpenaiChat,
    OpenaiResponses,
    Gemini,
    Bedrock,
    Vertex,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChatRouteFeature {
    TextInput,
    ImageInput,
    AudioInput,
    TextOutput,
    AudioOutput,
    ToolCalls,
    Reasoning,
    ReasoningSignature,
    PromptCache,
    StructuredOutput,
    ProviderRoundState,
    NativeResponsesItems,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatRouteCandidate {
    pub model_route_id: ModelRouteId,
    pub model_route_revision: u64,
    pub provider_id: String,
    pub model: String,
    pub protocol: ChatRouteProtocol,
    pub connection_config_ref: ConnectionConfigRef,
    pub config_revision_digest: DigestHex,
    pub credential_ref: String,
    pub features: BTreeSet<ChatRouteFeature>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatRouteRecord {
    pub schema: ChatRouteRecordSchema,
    pub task: ChatRouteTask,
    pub primary: ChatRouteCandidate,
    pub failovers: Vec<ChatRouteCandidate>,
}

pub type CanonicalChatRouteRecord = ChatRouteRecord;
pub type CanonicalChatRouteCandidate = ChatRouteCandidate;
pub type ChatRouteRecordCandidate = ChatRouteCandidate;

impl ChatRouteRecord {
    pub fn validate(&self) -> Result<(), ChatRouteRecordError> {
        if self.schema.as_str() != CHAT_ROUTE_RECORD_SCHEMA_V1 {
            return Err(ChatRouteRecordError::UnsupportedSchema);
        }
        if self.task.as_str() != CHAT_MODEL_TASK_AGENT_CHAT {
            return Err(ChatRouteRecordError::UnsupportedTask);
        }

        let mut identities = BTreeSet::new();
        for candidate in std::iter::once(&self.primary).chain(self.failovers.iter()) {
            candidate.validate()?;
            if !identities.insert((
                candidate.model_route_id.as_ref().to_owned(),
                candidate.model_route_revision,
            )) {
                return Err(ChatRouteRecordError::DuplicateCandidate);
            }
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        identity: &ChatRouteIdentity,
    ) -> Result<(), ChatRouteRecordError> {
        self.validate()?;
        identity
            .validate()
            .map_err(|error| match error {
                ChatRouteLookupError::InvalidKey(error)
                | ChatRouteLookupError::InvalidRecord(error) => error,
                ChatRouteLookupError::Missing | ChatRouteLookupError::DuplicateRows => {
                    ChatRouteRecordError::InvalidNaturalKey("route identity")
                }
            })?;
        if identity.model_task != self.task.as_str() {
            return Err(ChatRouteRecordError::TaskMismatch);
        }
        if identity.route_id != self.primary.model_route_id {
            return Err(ChatRouteRecordError::PrimaryRouteMismatch);
        }
        if identity.route_revision == 0
            || identity.route_revision != self.primary.model_route_revision
        {
            return Err(ChatRouteRecordError::PrimaryRouteRevisionMismatch);
        }
        Ok(())
    }

    pub fn to_canonical_json(&self) -> Result<String, CanonicalDigestError> {
        String::from_utf8(canonical_json_bytes(self)?)
            .map_err(|error| {
                CanonicalDigestError::Serialize(serde_json::Error::io(
                    std::io::Error::new(std::io::ErrorKind::InvalidData, error),
                ))
            })
    }

    pub fn from_json(value: &str) -> Result<Self, ChatRouteRecordError> {
        let record: Self = serde_json::from_str(value)
            .map_err(|error| ChatRouteRecordError::InvalidJson(error.to_string()))?;
        record.validate()?;
        Ok(record)
    }
}

impl ChatRouteCandidate {
    fn validate(&self) -> Result<(), ChatRouteRecordError> {
        validate_natural_key("model_route_id", self.model_route_id.as_ref())?;
        validate_natural_key("provider_id", &self.provider_id)?;
        validate_natural_key("model", &self.model)?;
        validate_natural_key(
            "connection_config_ref",
            self.connection_config_ref.as_ref(),
        )?;
        validate_natural_key("credential_ref", &self.credential_ref)?;
        if self.model_route_revision == 0 {
            return Err(ChatRouteRecordError::ZeroRouteRevision);
        }
        if !is_lower_hex_digest(self.config_revision_digest.as_ref()) {
            return Err(ChatRouteRecordError::InvalidDigest);
        }
        if !self.features.contains(&ChatRouteFeature::TextOutput) {
            return Err(ChatRouteRecordError::MissingTextOutput);
        }
        Ok(())
    }
}

/// The immutable identity of one model route as selected by one Preset
/// Revision.
///
/// This is deliberately a value object rather than four independently-carried
/// fields.  Every layer that resolves or admits a Chat route must carry this
/// exact identity unchanged.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ChatRouteIdentity {
    pub preset_revision_id: String,
    pub model_task: String,
    pub route_id: ModelRouteId,
    pub route_revision: u64,
}

impl ChatRouteIdentity {
    pub fn new(
        preset_revision_id: impl Into<String>,
        model_task: impl Into<String>,
        route_id: ModelRouteId,
        route_revision: u64,
    ) -> Self {
        Self {
            preset_revision_id: preset_revision_id.into(),
            model_task: model_task.into(),
            route_id,
            route_revision,
        }
    }

    pub fn validate(&self) -> Result<(), ChatRouteLookupError> {
        validate_natural_key("preset_revision_id", &self.preset_revision_id)
            .map_err(ChatRouteLookupError::InvalidKey)?;
        validate_natural_key("model_task", &self.model_task)
            .map_err(ChatRouteLookupError::InvalidKey)?;
        if self.model_task != CHAT_MODEL_TASK_AGENT_CHAT {
            return Err(ChatRouteLookupError::InvalidKey(
                ChatRouteRecordError::UnsupportedTask,
            ));
        }
        validate_natural_key("route_id", self.route_id.as_ref())
            .map_err(ChatRouteLookupError::InvalidKey)?;
        if self.route_revision == 0 {
            return Err(ChatRouteLookupError::InvalidKey(
                ChatRouteRecordError::ZeroRouteRevision,
            ));
        }
        Ok(())
    }

    pub fn matches_route(&self, route_id: &ModelRouteId, route_revision: u64) -> bool {
        &self.route_id == route_id && self.route_revision == route_revision
    }

    pub fn with_route(&self, route_id: ModelRouteId, route_revision: u64) -> Self {
        Self {
            preset_revision_id: self.preset_revision_id.clone(),
            model_task: self.model_task.clone(),
            route_id,
            route_revision,
        }
    }
}

/// Compatibility name for callers that perform an exact database lookup.
/// It is an alias, not a second route identity type.
pub type ChatRouteLookupKey = ChatRouteIdentity;

impl ChatRouteRecord {
    pub fn identity_for(
        &self,
        preset_revision_id: impl Into<String>,
        model_task: impl Into<String>,
    ) -> Result<ChatRouteIdentity, ChatRouteRecordError> {
        let identity = ChatRouteIdentity::new(
            preset_revision_id,
            model_task,
            self.primary.model_route_id.clone(),
            self.primary.model_route_revision,
        );
        identity
            .validate()
            .map_err(|error| match error {
                ChatRouteLookupError::InvalidKey(error)
                | ChatRouteLookupError::InvalidRecord(error) => error,
                ChatRouteLookupError::Missing => ChatRouteRecordError::InvalidNaturalKey(
                    "route identity",
                ),
                ChatRouteLookupError::DuplicateRows => {
                    ChatRouteRecordError::InvalidNaturalKey("route identity")
                }
            })?;
        self.validate_for(&identity)?;
        Ok(identity)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChatRouteRecordRow {
    pub revision_id: String,
    pub model_task: String,
    pub route_json: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ChatRouteRecordError {
    #[error("route record schema is unsupported")]
    UnsupportedSchema,
    #[error("route record task is unsupported")]
    UnsupportedTask,
    #[error("route record contains a duplicate candidate")]
    DuplicateCandidate,
    #[error("route record task does not match the lookup task")]
    TaskMismatch,
    #[error("route record primary route id does not match the lookup")]
    PrimaryRouteMismatch,
    #[error("route record primary route revision does not match the lookup")]
    PrimaryRouteRevisionMismatch,
    #[error("route record field {0} is empty, trimmed, or contains control characters")]
    InvalidNaturalKey(&'static str),
    #[error("route record route revision is zero")]
    ZeroRouteRevision,
    #[error("route record config revision digest is not lowercase hexadecimal")]
    InvalidDigest,
    #[error("route record does not advertise text output")]
    MissingTextOutput,
    #[error("route record JSON is invalid: {0}")]
    InvalidJson(String),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ChatRouteLookupError {
    #[error("exact chat route record is missing")]
    Missing,
    #[error("exact chat route lookup returned multiple rows")]
    DuplicateRows,
    #[error("exact chat route lookup key is invalid: {0}")]
    InvalidKey(#[source] ChatRouteRecordError),
    #[error("exact chat route record is invalid: {0}")]
    InvalidRecord(#[source] ChatRouteRecordError),
}

pub fn resolve_exact_chat_route_record<I>(
    rows: I,
    key: &ChatRouteIdentity,
) -> Result<ChatRouteRecord, ChatRouteLookupError>
where
    I: IntoIterator<Item = ChatRouteRecordRow>,
{
    key.validate()?;
    let matches = rows
        .into_iter()
        .filter(|row| {
            row.revision_id == key.preset_revision_id && row.model_task == key.model_task
        })
        .collect::<Vec<_>>();
    let row = match matches.as_slice() {
        [] => return Err(ChatRouteLookupError::Missing),
        [_first, _second, ..] => return Err(ChatRouteLookupError::DuplicateRows),
        [row] => row,
    };
    let record = ChatRouteRecord::from_json(&row.route_json)
        .map_err(ChatRouteLookupError::InvalidRecord)?;
    record
        .validate_for(key)
        .map_err(ChatRouteLookupError::InvalidRecord)?;
    Ok(record)
}

fn validate_natural_key(
    field: &'static str,
    value: &str,
) -> Result<(), ChatRouteRecordError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(ChatRouteRecordError::InvalidNaturalKey(field));
    }
    Ok(())
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn candidate(id: &str, revision: u64) -> ChatRouteCandidate {
        ChatRouteCandidate {
            model_route_id: ModelRouteId::from(id),
            model_route_revision: revision,
            provider_id: "provider-1".to_owned(),
            model: "model-1".to_owned(),
            protocol: ChatRouteProtocol::OpenaiChat,
            connection_config_ref: ConnectionConfigRef::from("connection-1"),
            config_revision_digest: DigestHex::from("a".repeat(64)),
            credential_ref: "credential-1".to_owned(),
            features: BTreeSet::from([
                ChatRouteFeature::TextInput,
                ChatRouteFeature::TextOutput,
            ]),
        }
    }

    fn record() -> ChatRouteRecord {
        ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: candidate("opaque-route", 7),
            failovers: vec![candidate("opaque-fallback", 2)],
        }
    }

    fn key() -> ChatRouteIdentity {
        ChatRouteIdentity {
            preset_revision_id: "preset@3".to_owned(),
            model_task: CHAT_MODEL_TASK_AGENT_CHAT.to_owned(),
            route_id: ModelRouteId::from("opaque-route"),
            route_revision: 7,
        }
    }

    #[test]
    fn canonical_json_is_an_object_with_an_opaque_route_id() {
        let json = record().to_canonical_json().unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert!(value.is_object());
        assert_eq!(
            value["primary"]["model_route_id"],
            json!("opaque-route")
        );
        assert_eq!(
            value["schema"],
            json!(CHAT_ROUTE_RECORD_SCHEMA_V1)
        );
    }

    #[test]
    fn legacy_string_route_json_is_rejected() {
        let error = ChatRouteRecord::from_json("\"opaque-route\"").unwrap_err();
        assert!(matches!(error, ChatRouteRecordError::InvalidJson(_)));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut value: Value = serde_json::to_value(record()).unwrap();
        value["derived_provider"] = json!("must-not-be-accepted");
        let error = ChatRouteRecord::from_json(&value.to_string()).unwrap_err();
        assert!(matches!(error, ChatRouteRecordError::InvalidJson(_)));
    }

    #[test]
    fn exact_lookup_requires_all_four_key_dimensions() {
        let rows = vec![ChatRouteRecordRow {
            revision_id: "preset@3".to_owned(),
            model_task: CHAT_MODEL_TASK_AGENT_CHAT.to_owned(),
            route_json: record().to_canonical_json().unwrap(),
        }];
        let resolved = resolve_exact_chat_route_record(rows.clone(), &key()).unwrap();
        assert_eq!(resolved.primary.model_route_id.as_ref(), "opaque-route");

        let mut wrong_revision = key();
        wrong_revision.route_revision = 8;
        assert!(matches!(
            resolve_exact_chat_route_record(rows.clone(), &wrong_revision),
            Err(ChatRouteLookupError::InvalidRecord(
                ChatRouteRecordError::PrimaryRouteRevisionMismatch
            ))
        ));

        let mut wrong_revision_id = key();
        wrong_revision_id.preset_revision_id = "preset@4".to_owned();
        assert!(matches!(
            resolve_exact_chat_route_record(rows, &wrong_revision_id),
            Err(ChatRouteLookupError::Missing)
        ));
    }

    #[test]
    fn identity_is_derived_from_the_canonical_record_without_route_guessing() {
        let identity = record()
            .identity_for("preset@3", CHAT_MODEL_TASK_AGENT_CHAT)
            .unwrap();
        assert_eq!(identity.preset_revision_id, "preset@3");
        assert_eq!(identity.model_task, CHAT_MODEL_TASK_AGENT_CHAT);
        assert_eq!(identity.route_id.as_ref(), "opaque-route");
        assert_eq!(identity.route_revision, 7);
    }

    #[test]
    fn duplicate_outer_rows_fail_closed() {
        let row = ChatRouteRecordRow {
            revision_id: "preset@3".to_owned(),
            model_task: CHAT_MODEL_TASK_AGENT_CHAT.to_owned(),
            route_json: record().to_canonical_json().unwrap(),
        };
        assert_eq!(
            resolve_exact_chat_route_record(vec![row.clone(), row], &key()),
            Err(ChatRouteLookupError::DuplicateRows)
        );
    }
}

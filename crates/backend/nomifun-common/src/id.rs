use std::fmt;

use uuid::{Uuid, Version};

/// Maximum length of an ID prefix.
///
/// Prefixes are intentionally short, stable type discriminators rather than
/// user-provided labels. Keeping a hard bound prevents unbounded identifiers
/// from entering database keys, URLs, logs, or filesystem paths.
pub const MAX_ID_PREFIX_LEN: usize = 32;

/// Length of a canonical lowercase hyphenated UUID.
pub const UUID_STRING_LEN: usize = 36;

/// Error returned when an entity-ID prefix is not canonical.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdPrefixError {
    #[error("ID prefix must not be empty")]
    Empty,
    #[error("ID prefix exceeds the maximum length of {MAX_ID_PREFIX_LEN} characters")]
    TooLong,
    #[error("ID prefix must start with an ASCII lowercase letter")]
    InvalidStart,
    #[error("ID prefix may contain only ASCII lowercase letters and digits")]
    InvalidCharacter,
}

/// Error returned when a prefixed UUIDv7 entity ID is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrefixedIdError {
    #[error(transparent)]
    InvalidPrefix(#[from] IdPrefixError),
    #[error("ID must use the exact prefix '{expected}_'")]
    WrongPrefix { expected: &'static str },
    #[error("ID must contain a canonical lowercase hyphenated UUID")]
    InvalidUuid,
    #[error("entity ID UUID must be version 7")]
    WrongUuidVersion,
    #[error("entity ID UUID must use the RFC 4122 variant")]
    WrongUuidVariant,
}

/// Error returned when a standalone dataset/namespace UUID is not canonical
/// UUIDv7.  Dataset generations are not entity IDs (they intentionally have no
/// prefix), but they still cross process, backup, and browser-cache boundaries
/// and therefore use the same strict lowercase RFC-4122 UUIDv7 grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UuidV7Error {
    #[error("UUID must be a canonical lowercase hyphenated UUID")]
    InvalidFormat,
    #[error("UUID must be version 7")]
    WrongVersion,
    #[error("UUID must use the RFC 4122 variant")]
    WrongVariant,
}

/// Validate a standalone canonical lowercase hyphenated RFC-4122 UUIDv7.
pub fn validate_uuidv7(value: &str) -> Result<Uuid, UuidV7Error> {
    if value.len() != UUID_STRING_LEN {
        return Err(UuidV7Error::InvalidFormat);
    }
    let uuid = Uuid::parse_str(value).map_err(|_| UuidV7Error::InvalidFormat)?;
    if uuid.hyphenated().to_string() != value {
        return Err(UuidV7Error::InvalidFormat);
    }
    if uuid.get_version() != Some(Version::SortRand) {
        return Err(UuidV7Error::WrongVersion);
    }
    if uuid.get_variant() != uuid::Variant::RFC4122 {
        return Err(UuidV7Error::WrongVariant);
    }
    Ok(uuid)
}

/// Validate the canonical syntax of an entity-ID prefix.
///
/// Prefixes:
/// - contain between 1 and 32 ASCII characters;
/// - start with `a`-`z`;
/// - contain only `a`-`z` and `0`-`9`.
///
/// Underscores are reserved for the single separator before the UUID body.
pub fn validate_id_prefix(prefix: &str) -> Result<(), IdPrefixError> {
    let bytes = prefix.as_bytes();
    if bytes.is_empty() {
        return Err(IdPrefixError::Empty);
    }
    if bytes.len() > MAX_ID_PREFIX_LEN {
        return Err(IdPrefixError::TooLong);
    }
    if !bytes[0].is_ascii_lowercase() {
        return Err(IdPrefixError::InvalidStart);
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(IdPrefixError::InvalidCharacter);
    }
    Ok(())
}

/// Validate a `{prefix}_{uuid-v7}` entity ID and return its UUID.
///
/// Validation is intentionally canonical rather than merely parseable:
/// uppercase, compact/braced UUIDs, whitespace, the wrong prefix, non-v7 UUIDs,
/// and non-RFC-4122 variants are rejected.
pub fn validate_prefixed_id(
    value: &str,
    expected_prefix: &'static str,
) -> Result<Uuid, PrefixedIdError> {
    validate_id_prefix(expected_prefix)?;

    let Some(body) = value.strip_prefix(expected_prefix) else {
        return Err(PrefixedIdError::WrongPrefix {
            expected: expected_prefix,
        });
    };
    let Some(body) = body.strip_prefix('_') else {
        return Err(PrefixedIdError::WrongPrefix {
            expected: expected_prefix,
        });
    };
    if body.len() != UUID_STRING_LEN {
        return Err(PrefixedIdError::InvalidUuid);
    }

    validate_uuidv7(body).map_err(|error| match error {
        UuidV7Error::InvalidFormat => PrefixedIdError::InvalidUuid,
        UuidV7Error::WrongVersion => PrefixedIdError::WrongUuidVersion,
        UuidV7Error::WrongVariant => PrefixedIdError::WrongUuidVariant,
    })
}

/// Generate a full UUIDv7 string.
///
/// This untyped helper is retained only for non-entity values such as opaque
/// operation tokens, credentials, and idempotency-key entropy. Persisted
/// entities must use [`generate_prefixed_id`] or a typed ID's `new` method.
pub fn generate_id() -> String {
    Uuid::now_v7().to_string()
}

/// Generate a globally unique entity ID as `{prefix}_{full_uuid_v7}`.
///
/// `prefix` must satisfy [`validate_id_prefix`]. This function keeps its
/// historical infallible signature so existing callers can migrate
/// incrementally; an invalid programmer-supplied prefix panics immediately
/// instead of minting an ambiguous ID.
pub fn generate_prefixed_id(prefix: &str) -> String {
    validate_id_prefix(prefix).unwrap_or_else(|error| {
        panic!("invalid entity ID prefix '{prefix}': {error}");
    });
    format!("{prefix}_{}", Uuid::now_v7())
}

/// Shared behavior implemented by every strongly typed entity ID.
pub trait EntityId:
    Clone
    + Eq
    + Ord
    + std::hash::Hash
    + AsRef<str>
    + fmt::Display
    + std::str::FromStr<Err = PrefixedIdError>
{
    /// Stable registered prefix for this entity kind.
    const PREFIX: &'static str;

    /// Mint a new globally unique ID.
    fn new() -> Self;

    /// Return the canonical string representation.
    fn as_str(&self) -> &str {
        self.as_ref()
    }
}

macro_rules! define_entity_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            serde::Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Stable registered prefix for this entity kind.
            pub const PREFIX: &'static str = $prefix;

            /// Mint a new globally unique ID of this entity kind.
            pub fn new() -> Self {
                Self(generate_prefixed_id(Self::PREFIX))
            }

            /// Parse and validate a canonical ID of this entity kind.
            pub fn parse(value: impl Into<String>) -> Result<Self, PrefixedIdError> {
                let value = value.into();
                validate_prefixed_id(&value, Self::PREFIX)?;
                Ok(Self(value))
            }

            /// Return the canonical string representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the typed ID and return its canonical string.
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl EntityId for $name {
            const PREFIX: &'static str = Self::PREFIX;

            fn new() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl std::str::FromStr for $name {
            type Err = PrefixedIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = PrefixedIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = PrefixedIdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <String as serde::Deserialize>::deserialize(deserializer)?;
                Self::parse(value).map_err(serde::de::Error::custom)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}

define_entity_id!(
    /// Globally unique conversation identifier.
    ConversationId,
    "conv"
);
define_entity_id!(
    /// Globally unique terminal-session identifier.
    TerminalId,
    "term"
);
define_entity_id!(
    /// Globally unique requirement identifier.
    RequirementId,
    "req"
);
define_entity_id!(
    /// Globally unique configured MCP-server identifier.
    McpServerId,
    "mcp"
);
define_entity_id!(
    /// Globally unique remote-agent identifier.
    RemoteAgentId,
    "ragent"
);
define_entity_id!(
    /// Globally unique webhook identifier.
    WebhookId,
    "webhook"
);
define_entity_id!(
    /// Globally unique conversation-artifact identifier.
    ConversationArtifactId,
    "artifact"
);
define_entity_id!(
    /// Globally unique knowledge-binding identifier.
    KnowledgeBindingId,
    "kbind"
);
define_entity_id!(
    /// Globally unique user identifier.
    UserId,
    "user"
);
define_entity_id!(
    /// Globally unique provider configuration identifier.
    ProviderId,
    "prov"
);
define_entity_id!(
    /// Globally unique custom-agent identifier.
    ///
    /// Builtin agents use stable installation keys (`agent_builtin_*`) and are
    /// intentionally not instances of this type.
    AgentId,
    "agent"
);
define_entity_id!(
    /// Globally unique user-authored preset identifier.
    ///
    /// Builtin and extension presets remain namespaced natural catalog keys.
    PresetId,
    "preset"
);
define_entity_id!(
    /// Globally unique user-authored preset-tag identifier.
    ///
    /// Builtin tag vocabulary entries remain stable manifest natural keys.
    PresetTagId,
    "presettag"
);
define_entity_id!(
    /// Globally unique message identifier.
    MessageId,
    "msg"
);
define_entity_id!(
    /// Globally unique knowledge-base identifier.
    KnowledgeBaseId,
    "kb"
);
define_entity_id!(
    /// Globally unique attachment identifier.
    AttachmentId,
    "att"
);
define_entity_id!(
    /// Globally unique connector-credential identifier.
    ConnectorCredentialId,
    "conn"
);
define_entity_id!(
    /// Globally unique cron-job identifier.
    CronJobId,
    "cron"
);
define_entity_id!(
    /// Globally unique cron-job-run identifier.
    CronJobRunId,
    "cronrun"
);
define_entity_id!(
    /// Globally unique IDMM intervention record identifier.
    IdmmInterventionId,
    "idmmrec"
);
define_entity_id!(
    /// Globally unique agent-execution template identifier.
    AgentExecutionTemplateId,
    "aext"
);
define_entity_id!(
    /// Globally unique agent-execution template participant identifier.
    AgentExecutionTemplateParticipantId,
    "aetp"
);
define_entity_id!(
    /// Globally unique agent-execution identifier.
    AgentExecutionId,
    "exec"
);
define_entity_id!(
    /// Globally unique participant identifier within an agent execution.
    AgentExecutionParticipantId,
    "execpart"
);
define_entity_id!(
    /// Globally unique step identifier within an agent execution.
    AgentExecutionStepId,
    "execstep"
);
define_entity_id!(
    /// Globally unique attempt identifier within an agent execution.
    AgentExecutionAttemptId,
    "eattempt"
);
define_entity_id!(
    /// Globally unique agent-execution event identifier.
    AgentExecutionEventId,
    "aevt"
);
define_entity_id!(
    /// Globally unique conversation/execution link identifier.
    ConversationExecutionLinkId,
    "execlink"
);
define_entity_id!(
    /// Globally unique channel configuration identifier.
    ChannelId,
    "chn"
);
define_entity_id!(
    /// Globally unique channel-user identifier.
    ChannelUserId,
    "chu"
);
define_entity_id!(
    /// Globally unique channel-session identifier.
    ChannelSessionId,
    "chs"
);
define_entity_id!(
    /// Globally unique companion identifier.
    CompanionId,
    "companion"
);
define_entity_id!(
    /// Globally unique companion memory identifier.
    CompanionMemoryId,
    "mem"
);
define_entity_id!(
    /// Globally unique companion suggestion identifier.
    CompanionSuggestionId,
    "sug"
);
define_entity_id!(
    /// Globally unique companion learn-run identifier.
    CompanionLearnRunId,
    "plr"
);
define_entity_id!(
    /// Globally unique companion session-window identifier.
    CompanionSessionWindowId,
    "csw"
);
define_entity_id!(
    /// Globally unique figure-library entry identifier.
    ///
    /// Figure metadata is stored in the durable figure index and the ID also
    /// names the corresponding image file, so it is an entity ID even though
    /// it does not live in the main SQLite database.
    FigureId,
    "figure"
);
define_entity_id!(
    /// Globally unique public-agent audit-entry identifier.
    ///
    /// Audit entries are durable JSONL records and may be paged or exported
    /// independently of the public-agent profile that owns them.
    PublicAgentAuditEntryId,
    "audit"
);
define_entity_id!(
    /// Globally unique companion evolution-feedback identifier.
    CompanionEvolutionFeedbackId,
    "evf"
);
define_entity_id!(
    /// Globally unique public-agent identifier.
    PublicAgentId,
    "pubagent"
);
define_entity_id!(
    /// Globally unique workshop-canvas identifier.
    WorkshopCanvasId,
    "wsc"
);
define_entity_id!(
    /// Globally unique workshop-asset identifier.
    WorkshopAssetId,
    "wsa"
);
define_entity_id!(
    /// Globally unique creation-task identifier.
    CreationTaskId,
    "wst"
);
define_entity_id!(
    /// Globally unique node identifier within a durable workshop canvas doc.
    WorkshopNodeId,
    "wsn"
);
define_entity_id!(
    /// Globally unique edge identifier within a durable workshop canvas doc.
    WorkshopEdgeId,
    "wse"
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generate_id_is_uuid_v7() {
        let id = generate_id();
        let uuid = Uuid::parse_str(&id).expect("valid UUID");
        assert_eq!(uuid.get_version(), Some(Version::SortRand));
        assert_eq!(uuid.hyphenated().to_string(), id);
    }

    #[test]
    fn prefixed_id_uses_full_canonical_uuid_v7() {
        let id = generate_prefixed_id("msg");
        assert_eq!(id.len(), "msg_".len() + UUID_STRING_LEN);
        let uuid = validate_prefixed_id(&id, "msg").expect("valid prefixed UUIDv7");
        assert_eq!(id, format!("msg_{uuid}"));
    }

    #[test]
    fn standalone_uuidv7_validation_is_strictly_canonical() {
        let valid = Uuid::now_v7().to_string();
        assert_eq!(validate_uuidv7(&valid).unwrap().to_string(), valid);

        assert_eq!(
            validate_uuidv7("550e8400-e29b-41d4-a716-446655440000"),
            Err(UuidV7Error::WrongVersion)
        );
        assert_eq!(
            validate_uuidv7(&valid.to_ascii_uppercase()),
            Err(UuidV7Error::InvalidFormat)
        );
        assert_eq!(
            validate_uuidv7(&valid.replace('-', "")),
            Err(UuidV7Error::InvalidFormat)
        );
        assert_eq!(
            validate_uuidv7(&format!("{valid}\n")),
            Err(UuidV7Error::InvalidFormat)
        );
    }

    #[test]
    fn generated_ids_are_unique() {
        let ids: HashSet<String> = (0..10_000)
            .map(|_| generate_prefixed_id("test"))
            .collect();
        assert_eq!(ids.len(), 10_000);
    }

    #[test]
    fn generated_uuid_v7_ids_sort_by_later_millisecond() {
        let earlier = generate_prefixed_id("conv");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let later = generate_prefixed_id("conv");
        assert!(later > earlier, "{later} should sort after {earlier}");
    }

    #[test]
    fn prefix_validation_accepts_canonical_prefixes() {
        for prefix in [
            "a",
            "conv",
            "cronrun",
            "mcp2",
            "a123",
        ] {
            assert_eq!(validate_id_prefix(prefix), Ok(()), "{prefix}");
        }
    }

    #[test]
    fn prefix_validation_rejects_empty_whitespace_and_separators() {
        for prefix in [
            "",
            " conv",
            "conv ",
            "con v",
            "Conv",
            "1conv",
            "conv_id",
            "conv-id",
            "会话",
        ] {
            assert!(validate_id_prefix(prefix).is_err(), "{prefix:?}");
        }
        assert_eq!(
            validate_id_prefix(&"a".repeat(MAX_ID_PREFIX_LEN + 1)),
            Err(IdPrefixError::TooLong)
        );
    }

    #[test]
    #[should_panic(expected = "invalid entity ID prefix")]
    fn generator_panics_on_invalid_prefix() {
        let _ = generate_prefixed_id("bad prefix");
    }

    #[test]
    fn validation_rejects_wrong_prefix_uuid_version_and_noncanonical_uuid() {
        let valid = generate_prefixed_id("conv");
        assert_eq!(
            validate_prefixed_id(&valid, "term"),
            Err(PrefixedIdError::WrongPrefix { expected: "term" })
        );

        let v4 = "conv_550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            validate_prefixed_id(v4, "conv"),
            Err(PrefixedIdError::WrongUuidVersion)
        );

        let uppercase = valid.to_ascii_uppercase();
        assert_eq!(
            validate_prefixed_id(&uppercase, "conv"),
            Err(PrefixedIdError::WrongPrefix { expected: "conv" })
        );

        let compact = valid.replace('-', "");
        assert_eq!(
            validate_prefixed_id(&compact, "conv"),
            Err(PrefixedIdError::InvalidUuid)
        );
    }

    #[test]
    fn typed_id_roundtrips_display_parse_as_ref_and_string() {
        let id = ConversationId::new();
        let text = id.to_string();
        assert_eq!(id.as_ref(), text);
        assert_eq!(id.as_str(), text);
        assert_eq!(text.parse::<ConversationId>().unwrap(), id);
        assert_eq!(String::from(id.clone()), text);
        assert_eq!(id.into_string(), text);
    }

    #[test]
    fn typed_id_serde_is_transparent_and_validating() {
        let id = RequirementId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<RequirementId>(&json).unwrap(), id);

        let wrong_type = format!("\"{}\"", ConversationId::new());
        assert!(serde_json::from_str::<RequirementId>(&wrong_type).is_err());
        assert!(serde_json::from_str::<RequirementId>("42").is_err());
    }

    #[test]
    fn registered_entity_types_have_distinct_canonical_prefixes() {
        let prefixes = [
            ConversationId::PREFIX,
            TerminalId::PREFIX,
            RequirementId::PREFIX,
            McpServerId::PREFIX,
            RemoteAgentId::PREFIX,
            WebhookId::PREFIX,
            ConversationArtifactId::PREFIX,
            KnowledgeBindingId::PREFIX,
            UserId::PREFIX,
            ProviderId::PREFIX,
            AgentId::PREFIX,
            PresetId::PREFIX,
            PresetTagId::PREFIX,
            MessageId::PREFIX,
            KnowledgeBaseId::PREFIX,
            AttachmentId::PREFIX,
            ConnectorCredentialId::PREFIX,
            CronJobId::PREFIX,
            CronJobRunId::PREFIX,
            IdmmInterventionId::PREFIX,
            AgentExecutionTemplateId::PREFIX,
            AgentExecutionTemplateParticipantId::PREFIX,
            AgentExecutionId::PREFIX,
            AgentExecutionParticipantId::PREFIX,
            AgentExecutionStepId::PREFIX,
            AgentExecutionAttemptId::PREFIX,
            AgentExecutionEventId::PREFIX,
            ConversationExecutionLinkId::PREFIX,
            ChannelId::PREFIX,
            ChannelUserId::PREFIX,
            ChannelSessionId::PREFIX,
            CompanionId::PREFIX,
            CompanionMemoryId::PREFIX,
            CompanionSuggestionId::PREFIX,
            CompanionLearnRunId::PREFIX,
            CompanionSessionWindowId::PREFIX,
            FigureId::PREFIX,
            PublicAgentAuditEntryId::PREFIX,
            CompanionEvolutionFeedbackId::PREFIX,
            PublicAgentId::PREFIX,
            WorkshopCanvasId::PREFIX,
            WorkshopAssetId::PREFIX,
            CreationTaskId::PREFIX,
            WorkshopNodeId::PREFIX,
            WorkshopEdgeId::PREFIX,
        ];
        for prefix in prefixes {
            validate_id_prefix(prefix).unwrap();
        }
        assert_eq!(
            prefixes.into_iter().collect::<HashSet<_>>().len(),
            prefixes.len()
        );
    }
}

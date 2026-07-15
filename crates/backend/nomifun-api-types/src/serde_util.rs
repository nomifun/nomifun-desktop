//! Strict serde helpers shared by public DTOs.

use nomifun_common::{
    AgentExecutionAttemptId, AgentExecutionEventId, AgentExecutionId, AgentExecutionParticipantId,
    AgentExecutionStepId, AgentExecutionTemplateId, AgentExecutionTemplateParticipantId, AgentId,
    AttachmentId, ChannelId, ChannelSessionId, ChannelUserId, CompanionId,
    ConversationArtifactId, ConversationId, CronJobId, CronJobRunId, MessageId, PresetId,
    PresetTagId, ProviderId, PublicAgentId, RequirementId, TerminalId, UserId,
};

macro_rules! string_id_deserializers {
    ($required:ident, $optional:ident, $id:ty) => {
        #[allow(dead_code)]
        pub(crate) fn $required<'de, D>(deserializer: D) -> Result<String, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let value = <String as serde::Deserialize>::deserialize(deserializer)?;
            <$id>::parse(value.clone())
                .map(|_| value)
                .map_err(serde::de::Error::custom)
        }

        #[allow(dead_code)]
        pub(crate) fn $optional<'de, D>(
            deserializer: D,
        ) -> Result<Option<String>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let value = <Option<String> as serde::Deserialize>::deserialize(deserializer)?;
            value
                .map(|value| {
                    <$id>::parse(value.clone())
                        .map(|_| value)
                        .map_err(serde::de::Error::custom)
                })
                .transpose()
        }
    };
}

string_id_deserializers!(
    deserialize_conversation_id,
    deserialize_optional_conversation_id,
    ConversationId
);
string_id_deserializers!(
    deserialize_terminal_id,
    deserialize_optional_terminal_id,
    TerminalId
);
string_id_deserializers!(
    deserialize_cron_job_id,
    deserialize_optional_cron_job_id,
    CronJobId
);
string_id_deserializers!(
    deserialize_execution_participant_id,
    deserialize_optional_execution_participant_id,
    AgentExecutionParticipantId
);
string_id_deserializers!(
    deserialize_execution_id,
    deserialize_optional_execution_id,
    AgentExecutionId
);
string_id_deserializers!(
    deserialize_execution_step_id,
    deserialize_optional_execution_step_id,
    AgentExecutionStepId
);
string_id_deserializers!(
    deserialize_execution_attempt_id,
    deserialize_optional_execution_attempt_id,
    AgentExecutionAttemptId
);
string_id_deserializers!(
    deserialize_execution_event_id,
    deserialize_optional_execution_event_id,
    AgentExecutionEventId
);
string_id_deserializers!(
    deserialize_execution_template_id,
    deserialize_optional_execution_template_id,
    AgentExecutionTemplateId
);
string_id_deserializers!(
    deserialize_execution_template_participant_id,
    deserialize_optional_execution_template_participant_id,
    AgentExecutionTemplateParticipantId
);
string_id_deserializers!(
    deserialize_message_id,
    deserialize_optional_message_id,
    MessageId
);
string_id_deserializers!(
    deserialize_conversation_artifact_id,
    deserialize_optional_conversation_artifact_id,
    ConversationArtifactId
);
string_id_deserializers!(deserialize_user_id, deserialize_optional_user_id, UserId);
string_id_deserializers!(
    deserialize_provider_id,
    deserialize_optional_provider_id,
    ProviderId
);
string_id_deserializers!(
    deserialize_channel_id,
    deserialize_optional_channel_id,
    ChannelId
);
string_id_deserializers!(
    deserialize_channel_user_id,
    deserialize_optional_channel_user_id,
    ChannelUserId
);
string_id_deserializers!(
    deserialize_channel_session_id,
    deserialize_optional_channel_session_id,
    ChannelSessionId
);
string_id_deserializers!(
    deserialize_companion_id,
    deserialize_optional_companion_id,
    CompanionId
);
string_id_deserializers!(
    deserialize_public_agent_id,
    deserialize_optional_public_agent_id,
    PublicAgentId
);
string_id_deserializers!(
    deserialize_preset_id,
    deserialize_optional_preset_id,
    PresetId
);
string_id_deserializers!(
    deserialize_requirement_id,
    deserialize_optional_requirement_id,
    RequirementId
);
string_id_deserializers!(
    deserialize_attachment_id,
    deserialize_optional_attachment_id,
    AttachmentId
);
string_id_deserializers!(
    deserialize_cron_job_run_id,
    deserialize_optional_cron_job_run_id,
    CronJobRunId
);

macro_rules! string_id_vec_deserializer {
    ($name:ident, $id:ty) => {
        pub(crate) fn $name<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <Vec<String> as serde::Deserialize>::deserialize(deserializer)?
                .into_iter()
                .map(|value| {
                    <$id>::parse(value.clone())
                        .map(|_| value)
                        .map_err(serde::de::Error::custom)
                })
                .collect()
        }
    };
}

string_id_vec_deserializer!(deserialize_requirement_ids, RequirementId);
string_id_vec_deserializer!(deserialize_attachment_ids, AttachmentId);

/// Preset references are either durable user preset IDs or stable catalog
/// natural keys supplied by builtin/extension manifests. A value claiming the
/// `preset_` entity namespace must always satisfy the UUIDv7 contract.
pub(crate) fn deserialize_optional_preset_reference<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <Option<String> as serde::Deserialize>::deserialize(deserializer)?;
    value.map(validate_preset_reference::<D::Error>).transpose()
}

pub(crate) fn deserialize_preset_reference<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    validate_preset_reference::<D::Error>(value)
}

fn validate_preset_reference<E>(value: String) -> Result<String, E>
where
    E: serde::de::Error,
{
    if value.starts_with("preset_") {
        PresetId::parse(value.clone())
            .map(|_| value)
            .map_err(E::custom)
    } else if is_catalog_natural_key(&value) {
        Ok(value)
    } else {
        Err(E::custom(
            "expected a canonical preset ID or stable catalog natural key",
        ))
    }
}

pub(crate) fn deserialize_preset_tag_reference<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    if value.starts_with("presettag_") {
        PresetTagId::parse(value.clone())
            .map(|_| value)
            .map_err(serde::de::Error::custom)
    } else if is_catalog_natural_key(&value) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "expected a canonical preset tag ID or stable catalog natural key",
        ))
    }
}

pub(crate) fn deserialize_agent_reference<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    validate_agent_reference::<D::Error>(value)
}

pub(crate) fn deserialize_optional_agent_reference<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <Option<String> as serde::Deserialize>::deserialize(deserializer)?;
    value.map(validate_agent_reference::<D::Error>).transpose()
}

fn validate_agent_reference<E>(value: String) -> Result<String, E>
where
    E: serde::de::Error,
{
    if AgentId::parse(value.clone()).is_ok()
        || (is_catalog_natural_key(&value)
            && (!value.starts_with("agent_") || value.starts_with("agent_builtin_")))
    {
        Ok(value)
    } else {
        Err(E::custom(
            "expected a canonical custom-agent ID or stable builtin/extension agent key",
        ))
    }
}

fn is_catalog_natural_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

/// Deserialize a canonical conversation-or-terminal entity ID.
///
/// The wire representation is string-only. Numeric JSON values, malformed
/// UUIDs, and IDs from any other entity namespace are rejected.
pub(crate) fn deserialize_session_target_id<'de, D>(
    deserializer: D,
) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    if ConversationId::parse(value.clone()).is_ok() || TerminalId::parse(value.clone()).is_ok() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "expected a canonical conversation or terminal entity ID",
        ))
    }
}

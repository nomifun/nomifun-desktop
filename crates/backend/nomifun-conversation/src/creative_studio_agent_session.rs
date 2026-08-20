use std::collections::HashSet;

use nomifun_api_types::CreateConversationRequest;
use nomifun_common::{
    AgentType, AppError, ConversationSource, DecisionPolicy, DelegationPolicy, ProviderId,
    ProviderWithModel, validate_uuidv7,
};
use nomifun_db::models::{ConversationRow, MessageRow};
use serde::{Deserialize, Serialize};

use super::{ConversationService, CreativeStudioAgentCreationTarget, parse_provider_with_model};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreativeStudioAgentHistoryRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreativeStudioAgentHistoryStatus {
    Complete,
    Running,
    Failed,
    Stopped,
}

/// Exact renderer history projection. Field declaration order is part of the
/// history-key contract and matches `serializeCreativeStudioAgentHistory`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeStudioAgentHistoryMessage {
    pub id: String,
    pub role: CreativeStudioAgentHistoryRole,
    pub status: CreativeStudioAgentHistoryStatus,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreativeStudioAgentModelRef {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveCreativeStudioAgentSessionRequest {
    pub project_id: String,
    pub session_id: String,
    pub model: CreativeStudioAgentModelRef,
    pub history: Vec<CreativeStudioAgentHistoryMessage>,
    pub history_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreativeStudioAgentSessionBindingResponse {
    pub ownership: &'static str,
    pub project_id: String,
    pub session_id: String,
    pub conversation_id: String,
    pub model: CreativeStudioAgentModelRef,
    pub history_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolveCreativeStudioAgentSessionResponse {
    pub binding: CreativeStudioAgentSessionBindingResponse,
    pub history: Vec<CreativeStudioAgentHistoryMessage>,
    pub created: bool,
}

fn serialize_history(
    history: &[CreativeStudioAgentHistoryMessage],
) -> Result<String, AppError> {
    serde_json::to_string(history).map_err(|error| {
        AppError::Internal(format!(
            "Creative Studio Agent history could not be serialized: {error}"
        ))
    })
}

fn validate_request(request: &ResolveCreativeStudioAgentSessionRequest) -> Result<(), AppError> {
    nomifun_common::CreativeStudioProjectId::parse(&request.project_id).map_err(|error| {
        AppError::BadRequest(format!("invalid Creative Studio project_id: {error}"))
    })?;
    validate_uuidv7(&request.session_id).map_err(|error| {
        AppError::BadRequest(format!("invalid Creative Studio session_id: {error}"))
    })?;
    ProviderId::parse(&request.model.provider_id).map_err(|error| {
        AppError::BadRequest(format!("invalid Creative Studio provider_id: {error}"))
    })?;
    if request.model.model.trim().is_empty() || request.model.model.trim() != request.model.model {
        return Err(AppError::BadRequest(
            "Creative Studio Agent model must be trimmed and non-empty".to_owned(),
        ));
    }

    let mut ids = HashSet::with_capacity(request.history.len());
    for message in &request.history {
        validate_uuidv7(&message.id).map_err(|error| {
            AppError::BadRequest(format!(
                "invalid Creative Studio history message id '{}': {error}",
                message.id
            ))
        })?;
        if !ids.insert(message.id.as_str()) {
            return Err(AppError::BadRequest(
                "Creative Studio history contains a duplicate message id".to_owned(),
            ));
        }
        if message.status != CreativeStudioAgentHistoryStatus::Complete
            || message.activity_label.is_some()
            || message.error_message.is_some()
        {
            return Err(AppError::Conflict(
                "Creative Studio session recovery accepts only durable completed history"
                    .to_owned(),
            ));
        }
    }
    if serialize_history(&request.history)? != request.history_key {
        return Err(AppError::Conflict(
            "Creative Studio Agent history_key does not match its canonical history"
                .to_owned(),
        ));
    }
    Ok(())
}

fn content_text(row: &MessageRow) -> Result<String, AppError> {
    let value: serde_json::Value = serde_json::from_str(&row.content).map_err(|error| {
        AppError::Internal(format!(
            "Creative Studio Agent message '{}' has invalid content JSON: {error}",
            row.message_id
        ))
    })?;
    value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "Creative Studio Agent message '{}' has no text content",
                row.message_id
            ))
        })
}

fn message_turn_id(row: &MessageRow) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(&row.content)
        .ok()
        .and_then(|value| {
            value
                .get("turn_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
}

#[derive(Debug)]
struct AssistantGroup {
    turn_id: String,
    id: String,
    text: String,
}

fn project_completed_history(
    rows: &[MessageRow],
) -> Result<Vec<CreativeStudioAgentHistoryMessage>, AppError> {
    let failed_turns = rows
        .iter()
        .filter(|row| !row.hidden && row.status.as_deref() == Some("error"))
        .filter_map(message_turn_id)
        .collect::<HashSet<_>>();
    let mut history = Vec::new();
    let mut pending_user: Option<CreativeStudioAgentHistoryMessage> = None;
    let mut assistant: Option<AssistantGroup> = None;

    let flush = |assistant: &mut Option<AssistantGroup>,
                 pending_user: &mut Option<CreativeStudioAgentHistoryMessage>,
                 history: &mut Vec<CreativeStudioAgentHistoryMessage>| {
        if let Some(group) = assistant.take()
            && !failed_turns.contains(&group.turn_id)
            && let Some(user) = pending_user.take()
        {
            history.push(user);
            history.push(CreativeStudioAgentHistoryMessage {
                id: group.id,
                role: CreativeStudioAgentHistoryRole::Assistant,
                status: CreativeStudioAgentHistoryStatus::Complete,
                text: group.text,
                activity_label: None,
                error_message: None,
            });
        }
    };

    for row in rows.iter().filter(|row| !row.hidden) {
        if row.r#type != "text" {
            continue;
        }
        match row.position.as_deref() {
            Some("right") => {
                flush(&mut assistant, &mut pending_user, &mut history);
                if row.status.as_deref() != Some("finish") {
                    continue;
                }
                validate_uuidv7(&row.message_id).map_err(|error| {
                    AppError::Internal(format!(
                        "persisted Creative Studio user message id is invalid: {error}"
                    ))
                })?;
                pending_user = Some(CreativeStudioAgentHistoryMessage {
                    id: row.message_id.clone(),
                    role: CreativeStudioAgentHistoryRole::User,
                    status: CreativeStudioAgentHistoryStatus::Complete,
                    text: content_text(row)?,
                    activity_label: None,
                    error_message: None,
                });
            }
            Some("left") if row.status.as_deref() == Some("finish") => {
                validate_uuidv7(&row.message_id).map_err(|error| {
                    AppError::Internal(format!(
                        "persisted Creative Studio assistant message id is invalid: {error}"
                    ))
                })?;
                let value: serde_json::Value = serde_json::from_str(&row.content).map_err(|error| {
                    AppError::Internal(format!(
                        "Creative Studio Agent assistant message has invalid content JSON: {error}"
                    ))
                })?;
                let turn_id = value
                    .get("turn_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        AppError::Internal(
                            "Creative Studio Agent assistant message has no durable turn_id"
                                .to_owned(),
                        )
                    })?;
                validate_uuidv7(turn_id).map_err(|error| {
                    AppError::Internal(format!(
                        "persisted Creative Studio assistant turn_id is invalid: {error}"
                    ))
                })?;
                let text = value
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        AppError::Internal(
                            "Creative Studio Agent assistant message has no text content"
                                .to_owned(),
                        )
                    })?;
                if assistant.as_ref().is_some_and(|group| group.turn_id != turn_id) {
                    flush(&mut assistant, &mut pending_user, &mut history);
                }
                let group = assistant.get_or_insert_with(|| AssistantGroup {
                    turn_id: turn_id.to_owned(),
                    id: row.message_id.clone(),
                    text: String::new(),
                });
                group.id = row.message_id.clone();
                group.text.push_str(text);
            }
            Some("left") if matches!(row.status.as_deref(), Some("work" | "error")) => {
                // An active or interrupted assistant row is not a completed
                // projection and therefore cannot satisfy a recovery fence.
            }
            _ => {}
        }
    }
    flush(&mut assistant, &mut pending_user, &mut history);
    Ok(history)
}

fn validate_persisted_binding(
    owner_id: &str,
    request: &ResolveCreativeStudioAgentSessionRequest,
    row: &ConversationRow,
) -> Result<CreativeStudioAgentModelRef, AppError> {
    if row.user_id != owner_id
        || row.r#type != "nomi"
        || row.source.as_deref() != Some("nomifun")
    {
        return Err(AppError::Conflict(
            "Creative Studio session binding points to an invalid Conversation aggregate"
                .to_owned(),
        ));
    }
    let stored = row
        .model
        .as_deref()
        .ok_or_else(|| AppError::Conflict("Creative Studio Conversation has no model".to_owned()))
        .and_then(parse_provider_with_model)?;
    let effective_model = stored.use_model.as_deref().unwrap_or(&stored.model);
    if stored.provider_id != request.model.provider_id || effective_model != request.model.model {
        return Err(AppError::Conflict(
            "Creative Studio Agent session model is immutable and does not match the request"
                .to_owned(),
        ));
    }
    Ok(CreativeStudioAgentModelRef {
        provider_id: stored.provider_id,
        model: effective_model.to_owned(),
    })
}

impl ConversationService {
    async fn validate_creative_studio_model(
        &self,
        model: &CreativeStudioAgentModelRef,
    ) -> Result<(), AppError> {
        let Some((providers, models, capabilities, _)) = self.failover_deps() else {
            return Err(AppError::Internal(
                "Creative Studio Agent model repositories are not wired".to_owned(),
            ));
        };
        let provider = providers
            .find_by_id(&model.provider_id)
            .await?
            .filter(|provider| provider.enabled)
            .ok_or_else(|| {
                AppError::ProviderUnavailable(
                    "selected Creative Studio provider is missing or disabled".to_owned(),
                )
            })?;
        let configured = models
            .get(&provider.provider_id, &model.model)
            .await?
            .filter(|configured| configured.enabled)
            .ok_or_else(|| {
                AppError::ProviderUnavailable(
                    "selected Creative Studio model is missing or disabled".to_owned(),
                )
            })?;
        capabilities
            .get(&configured.provider_id, &configured.model, "chat")
            .await?
            .ok_or_else(|| {
                AppError::ProviderUnavailable(
                    "selected Creative Studio model has no chat capability".to_owned(),
                )
            })?;
        Ok(())
    }

    pub async fn resolve_creative_studio_agent_session(
        &self,
        owner_id: &str,
        request: ResolveCreativeStudioAgentSessionRequest,
    ) -> Result<ResolveCreativeStudioAgentSessionResponse, AppError> {
        if owner_id != self.authoritative_user_id.as_ref() {
            return Err(AppError::Forbidden(
                "Creative Studio Agent sessions are restricted to the installation owner"
                    .to_owned(),
            ));
        }
        validate_request(&request)?;
        self.validate_creative_studio_model(&request.model).await?;

        let provider_model = ProviderWithModel {
            provider_id: request.model.provider_id.clone(),
            model: request.model.model.clone(),
            use_model: Some(request.model.model.clone()),
        };
        let (conversation, created) = self
            .create_inner(
                owner_id,
                CreateConversationRequest {
                    r#type: AgentType::Nomi,
                    name: Some("Creative Studio Agent".to_owned()),
                    model: Some(provider_model),
                    source: Some(ConversationSource::Nomifun),
                    channel_chat_id: None,
                    preset_id: None,
                    preset_overrides: None,
                    delegation_policy: DelegationPolicy::Disabled,
                    execution_model_pool: None,
                    decision_policy: DecisionPolicy::default(),
                    execution_template_id: None,
                    extra: serde_json::json!({}),
                },
                None,
                None,
                Some(CreativeStudioAgentCreationTarget {
                    project_id: request.project_id.clone(),
                    session_id: request.session_id.clone(),
                    create_if_missing: request.history.is_empty(),
                }),
            )
            .await?;

        let row = self
            .conversation_repo
            .get(&conversation.conversation_id)
            .await?
            .ok_or_else(|| {
                AppError::Conflict(
                    "Creative Studio Agent binding lost its Conversation after creation"
                        .to_owned(),
                )
            })?;
        let resolved = self
            .conversation_repo
            .resolve_or_create_creative_studio_agent_session(
                &nomifun_db::ResolveCreativeStudioAgentSessionParams {
                    owner_id: owner_id.to_owned(),
                    project_id: request.project_id.clone(),
                    session_id: request.session_id.clone(),
                    conversation: row,
                    create_if_missing: false,
                },
            )
            .await?;
        let persisted_model = validate_persisted_binding(owner_id, &request, &resolved.conversation)?;
        let history = project_completed_history(&resolved.messages)?;
        let projected_ids = history
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();
        let project_ids = resolved
            .project_message_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if projected_ids != project_ids || history != request.history {
            return Err(AppError::Conflict(
                "Creative Studio project history does not match the dedicated Conversation transcript"
                    .to_owned(),
            ));
        }
        let history_key = serialize_history(&history)?;
        if history_key != request.history_key {
            return Err(AppError::Conflict(
                "Creative Studio Agent history projection changed during resolution".to_owned(),
            ));
        }

        Ok(ResolveCreativeStudioAgentSessionResponse {
            binding: CreativeStudioAgentSessionBindingResponse {
                ownership: "creative-studio-exclusive",
                project_id: resolved.binding.project_id,
                session_id: resolved.binding.session_id,
                conversation_id: resolved.binding.conversation_id,
                model: persisted_model,
                history_key,
            },
            history,
            created,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000601";
    const TURN_ID: &str = "0190f5fe-7c00-7a00-8000-000000000602";
    const ASSISTANT_A: &str = "0190f5fe-7c00-7a00-8000-000000000603";
    const ASSISTANT_B: &str = "0190f5fe-7c00-7a00-8000-000000000604";

    fn row(id: &str, position: &str, content: serde_json::Value) -> MessageRow {
        MessageRow {
            id: 0,
            message_id: id.to_owned(),
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000605".to_owned(),
            msg_id: Some(id.to_owned()),
            r#type: "text".to_owned(),
            content: content.to_string(),
            position: Some(position.to_owned()),
            status: Some("finish".to_owned()),
            hidden: false,
            created_at: 1,
        }
    }

    #[test]
    fn canonical_history_matches_renderer_field_order() {
        let history = vec![CreativeStudioAgentHistoryMessage {
            id: USER_ID.to_owned(),
            role: CreativeStudioAgentHistoryRole::User,
            status: CreativeStudioAgentHistoryStatus::Complete,
            text: "制作海报".to_owned(),
            activity_label: None,
            error_message: None,
        }];
        assert_eq!(
            serialize_history(&history).unwrap(),
            format!(
                r#"[{{"id":"{USER_ID}","role":"user","status":"complete","text":"制作海报"}}]"#
            )
        );
    }

    #[test]
    fn projection_groups_assistant_segments_under_last_durable_id() {
        let history = project_completed_history(&[
            row(USER_ID, "right", serde_json::json!({ "content": "请制作" })),
            row(
                ASSISTANT_A,
                "left",
                serde_json::json!({ "content": "已经", "turn_id": TURN_ID }),
            ),
            row(
                ASSISTANT_B,
                "left",
                serde_json::json!({ "content": "完成", "turn_id": TURN_ID }),
            ),
        ])
        .unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].id, ASSISTANT_B);
        assert_eq!(history[1].text, "已经完成");
    }
}

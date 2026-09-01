use std::collections::HashSet;

use nomifun_api_types::CreateConversationRequest;
use nomifun_common::{
    AgentType, AppError, ConversationSource, DecisionPolicy, DelegationPolicy, ProviderId,
    ProviderWithModel, validate_uuidv7,
};
use nomifun_db::models::{ConversationRow, MessageRow};
use serde::{Deserialize, Serialize};

use super::{ConversationService, CreativeStudioAgentCreationTarget, parse_provider_with_model};

const CREATIVE_STUDIO_PLANNING_TURN_KIND: &str =
    "nomifun.creative-studio.planning-turn";

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

/// Canvas-first wire request. Persistence still uses the historical
/// `project_id` column internally, but the product boundary never exposes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveCreativeStudioCanvasAgentSessionRequest {
    pub canvas_id: String,
    pub session_id: String,
    pub model: CreativeStudioAgentModelRef,
    pub pending_turn_idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreativeStudioCanvasAgentSessionBindingResponse {
    pub ownership: &'static str,
    pub canvas_id: String,
    pub session_id: String,
    pub conversation_id: String,
    pub model: CreativeStudioAgentModelRef,
    pub history_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolveCreativeStudioCanvasAgentSessionResponse {
    pub binding: CreativeStudioCanvasAgentSessionBindingResponse,
    pub history: Vec<CreativeStudioAgentHistoryMessage>,
    pub applied_proposal_message_ids: Vec<String>,
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

fn validate_canvas_request(
    request: &ResolveCreativeStudioCanvasAgentSessionRequest,
) -> Result<(), AppError> {
    nomifun_common::CreativeStudioCanvasId::parse(&request.canvas_id).map_err(|error| {
        AppError::BadRequest(format!("invalid Creative Studio canvas_id: {error}"))
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
    if let Some(key) = request.pending_turn_idempotency_key.as_deref() {
        validate_uuidv7(key).map_err(|error| {
            AppError::BadRequest(format!(
                "invalid Creative Studio pending_turn_idempotency_key: {error}"
            ))
        })?;
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

fn projected_user_text(row: &MessageRow) -> Result<String, AppError> {
    let content = content_text(row)?;
    let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Ok(content);
    };
    if envelope.get("kind").and_then(serde_json::Value::as_str)
        != Some(CREATIVE_STUDIO_PLANNING_TURN_KIND)
    {
        return Ok(content);
    }
    if envelope.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err(AppError::Internal(format!(
            "Creative Studio Agent user message '{}' has an unsupported planning envelope version",
            row.message_id
        )));
    }
    let prompt = envelope
        .get("userRequest")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AppError::Internal(format!(
                "Creative Studio Agent user message '{}' has no visible userRequest",
                row.message_id
            ))
        })?;
    Ok(prompt.to_owned())
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

/// Only terminal transcript rows can invalidate an otherwise completed
/// user/assistant pair. Tool calls are allowed to fail inside a successful
/// turn (for example, one optional Skill lookup may fail before the model
/// produces a valid final answer), so their `status = error` must not poison
/// the whole turn projection.
fn is_terminal_failure_row(row: &MessageRow) -> bool {
    !row.hidden
        && row.status.as_deref() == Some("error")
        && matches!(row.r#type.as_str(), "text" | "tips")
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
        .filter(|row| is_terminal_failure_row(row))
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
                    text: projected_user_text(row)?,
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

fn validate_history_reconciliation(
    project_message_ids: &[String],
    projected_history: &[CreativeStudioAgentHistoryMessage],
    has_pending_turn: bool,
) -> Result<(), AppError> {
    let project_ids = project_message_ids
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let projected_ids = projected_history
        .iter()
        .map(|message| message.id.as_str())
        .collect::<Vec<_>>();
    if projected_ids.len() < project_ids.len()
        || projected_ids[..project_ids.len()] != project_ids[..]
    {
        return Err(AppError::Conflict(
            "Creative Studio Canvas message references are not a prefix of the dedicated Conversation transcript"
                .to_owned(),
        ));
    }
    let recovered = &projected_history[project_ids.len()..];
    let recovered_is_complete_pairs = recovered.len() % 2 == 0
        && recovered.chunks_exact(2).all(|pair| {
            pair[0].role == CreativeStudioAgentHistoryRole::User
                && pair[1].role == CreativeStudioAgentHistoryRole::Assistant
        });
    if !recovered.is_empty()
        && (!recovered_is_complete_pairs || (has_pending_turn && recovered.len() != 2))
    {
        return Err(AppError::Conflict(
            "Creative Studio session recovery must reconcile complete user/assistant Agent turns"
                .to_owned(),
        ));
    }
    Ok(())
}

fn validate_persisted_binding(
    owner_id: &str,
    request: &ResolveCreativeStudioCanvasAgentSessionRequest,
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

    pub async fn resolve_creative_studio_canvas_agent_session(
        &self,
        owner_id: &str,
        request: ResolveCreativeStudioCanvasAgentSessionRequest,
    ) -> Result<ResolveCreativeStudioCanvasAgentSessionResponse, AppError> {
        validate_canvas_request(&request)?;
        if owner_id != self.authoritative_user_id.as_ref() {
            return Err(AppError::Forbidden(
                "Creative Studio Agent sessions are restricted to the installation owner"
                    .to_owned(),
            ));
        }
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
                    project_id: request.canvas_id.clone(),
                    session_id: request.session_id.clone(),
                    expected_provider_id: request.model.provider_id.clone(),
                    expected_model: request.model.model.clone(),
                    expected_pending_turn_idempotency_key: request
                        .pending_turn_idempotency_key
                        .clone(),
                    create_if_missing: request.pending_turn_idempotency_key.is_some(),
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
                    project_id: request.canvas_id.clone(),
                    session_id: request.session_id.clone(),
                    expected_provider_id: request.model.provider_id.clone(),
                    expected_model: request.model.model.clone(),
                    expected_pending_turn_idempotency_key: request
                        .pending_turn_idempotency_key
                        .clone(),
                    conversation: row,
                    create_if_missing: false,
                },
            )
            .await?;
        let persisted_model = validate_persisted_binding(owner_id, &request, &resolved.conversation)?;
        let history = project_completed_history(&resolved.messages)?;
        validate_history_reconciliation(
            &resolved.project_message_ids,
            &history,
            request.pending_turn_idempotency_key.is_some(),
        )?;
        let history_key = serialize_history(&history)?;

        Ok(ResolveCreativeStudioCanvasAgentSessionResponse {
            binding: CreativeStudioCanvasAgentSessionBindingResponse {
                ownership: "creative-studio-exclusive",
                canvas_id: resolved.binding.project_id,
                session_id: resolved.binding.session_id,
                conversation_id: resolved.binding.conversation_id,
                model: persisted_model,
                history_key,
            },
            history,
            applied_proposal_message_ids: resolved.applied_proposal_message_ids,
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

    #[test]
    fn canvas_response_does_not_expose_storage_project_field() {
        let canvas_id = "0190f5fe-7c00-7a00-8000-000000000606";
        let response = ResolveCreativeStudioCanvasAgentSessionResponse {
            binding: CreativeStudioCanvasAgentSessionBindingResponse {
                ownership: "creative-studio-exclusive",
                canvas_id: canvas_id.to_owned(),
                session_id: "0190f5fe-7c00-7a00-8000-000000000607".to_owned(),
                conversation_id: "0190f5fe-7c00-7a00-8000-000000000609".to_owned(),
                model: CreativeStudioAgentModelRef {
                    provider_id: "0190f5fe-7c00-7a00-8000-000000000608".to_owned(),
                    model: "nomi-chat".to_owned(),
                },
                history_key: "[]".to_owned(),
            },
            history: Vec::new(),
            applied_proposal_message_ids: Vec::new(),
            created: true,
        };
        let wire = serde_json::to_value(response).unwrap();
        assert_eq!(wire["binding"]["canvas_id"], canvas_id);
        assert!(wire["binding"].get("project_id").is_none());
    }

    #[test]
    fn canvas_request_validation_uses_canvas_vocabulary() {
        let request = ResolveCreativeStudioCanvasAgentSessionRequest {
            canvas_id: "legacy-canvas".to_owned(),
            session_id: "0190f5fe-7c00-7a00-8000-000000000607".to_owned(),
            model: CreativeStudioAgentModelRef {
                provider_id: "0190f5fe-7c00-7a00-8000-000000000608".to_owned(),
                model: "nomi-chat".to_owned(),
            },
            pending_turn_idempotency_key: None,
        };
        let error = validate_canvas_request(&request).unwrap_err();
        assert!(
            matches!(error, AppError::BadRequest(message) if message.contains("canvas_id") && !message.contains("project"))
        );
    }

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

    fn history_message(
        id: &str,
        role: CreativeStudioAgentHistoryRole,
        text: &str,
    ) -> CreativeStudioAgentHistoryMessage {
        CreativeStudioAgentHistoryMessage {
            id: id.to_owned(),
            role,
            status: CreativeStudioAgentHistoryStatus::Complete,
            text: text.to_owned(),
            activity_label: None,
            error_message: None,
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

    #[test]
    fn projection_keeps_a_successful_pair_when_intermediate_tools_failed() {
        let mut failed_tool = row(
            "0190f5fe-7c00-7a00-8000-000000000607",
            "left",
            serde_json::json!({
                "call_id": "skill-call",
                "name": "Skill",
                "status": "error",
                "turn_id": TURN_ID,
                "output": "Skill not found"
            }),
        );
        failed_tool.r#type = "tool_call".to_owned();
        failed_tool.status = Some("error".to_owned());

        let history = project_completed_history(&[
            row(
                USER_ID,
                "right",
                serde_json::json!({ "content": "请规划画布" }),
            ),
            failed_tool,
            row(
                ASSISTANT_B,
                "left",
                serde_json::json!({
                    "content": "这是最终规划。",
                    "turn_id": TURN_ID
                }),
            ),
        ])
        .unwrap();

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text, "请规划画布");
        assert_eq!(history[1].text, "这是最终规划。");
    }

    #[test]
    fn projection_still_rejects_a_pair_with_a_terminal_failure_tip() {
        let mut terminal_tip = row(
            "0190f5fe-7c00-7a00-8000-000000000607",
            "left",
            serde_json::json!({
                "content": "The turn failed",
                "turn_id": TURN_ID
            }),
        );
        terminal_tip.r#type = "tips".to_owned();
        terminal_tip.status = Some("error".to_owned());

        let history = project_completed_history(&[
            row(
                USER_ID,
                "right",
                serde_json::json!({ "content": "请规划画布" }),
            ),
            row(
                ASSISTANT_B,
                "left",
                serde_json::json!({
                    "content": "未确认的草稿",
                    "turn_id": TURN_ID
                }),
            ),
            terminal_tip,
        ])
        .unwrap();

        assert!(history.is_empty());
    }

    #[test]
    fn projection_keeps_the_visible_prompt_out_of_the_internal_planning_envelope() {
        let planning_envelope = serde_json::json!({
            "kind": CREATIVE_STUDIO_PLANNING_TURN_KIND,
            "version": 1,
            "userRequest": "请整理当前画布",
            "selectedSkills": ["creative-studio-canvas"],
            "canvasContext": {
                "canvasRevision": "7",
                "nodes": [{ "id": "private-node", "details": { "text": "内部上下文" } }]
            },
            "responseContract": { "mode": "plan-and-propose" }
        })
        .to_string();
        let history = project_completed_history(&[
            row(
                USER_ID,
                "right",
                serde_json::json!({ "content": planning_envelope }),
            ),
            row(
                ASSISTANT_B,
                "left",
                serde_json::json!({ "content": "已完成", "turn_id": TURN_ID }),
            ),
        ])
        .unwrap();

        assert_eq!(history[0].text, "请整理当前画布");
        assert!(!serialize_history(&history).unwrap().contains("private-node"));
        assert!(!serialize_history(&history).unwrap().contains("responseContract"));
    }

    #[test]
    fn recovery_accepts_only_one_pending_completed_pair_after_project_history() {
        let recovered_pair = vec![
            history_message(USER_ID, CreativeStudioAgentHistoryRole::User, "制作海报"),
            history_message(
                ASSISTANT_B,
                CreativeStudioAgentHistoryRole::Assistant,
                "已经完成",
            ),
        ];
        validate_history_reconciliation(&[], &recovered_pair, true).unwrap();
        validate_history_reconciliation(&[], &recovered_pair, false).unwrap();

        let mut two_pairs = recovered_pair.clone();
        two_pairs.extend(recovered_pair.clone());
        assert!(validate_history_reconciliation(&[], &two_pairs, true).is_err());
        validate_history_reconciliation(&[], &two_pairs, false).unwrap();

        validate_history_reconciliation(
            &[USER_ID.to_owned(), ASSISTANT_B.to_owned()],
            &recovered_pair,
            false,
        )
        .unwrap();
        assert!(
            validate_history_reconciliation(
                &[USER_ID.to_owned()],
                &recovered_pair,
                false,
            )
            .is_err()
        );
    }
}

use std::path::Path;

use nomifun_api_types::{ConversationArtifactResponse, ConversationResponse, MessageResponse, MessageSearchItem};
use nomifun_common::{
    AgentExecutionTemplateId, AgentType, AppError, ConversationArtifactId, ConversationId,
    ConversationSource, ConversationStatus, CronJobId, MessageId, MessagePosition, MessageStatus,
    MessageType, ProviderId, ProviderWithModel, now_ms,
};
use nomifun_db::MessageSearchRow;
use nomifun_db::models::{ConversationArtifactRow, ConversationRow, MessageRow};

pub(crate) const TOOL_CONTENT_COMPACT_THRESHOLD_BYTES: usize = 64 * 1024;
const TOOL_CONTENT_PREVIEW_CHARS: usize = 4096;
const WRITEBACK_RUNNING_STALE_MS: i64 = 5 * 60 * 1000;

/// Convert a database row into an API response DTO.
///
/// Parses string enum fields and JSON text fields back into typed values.
/// `data_dir` is required so the response can expose a derived
/// `is_temporary_workspace` flag without storing that attribute on disk —
/// see [`row_to_response_with_extra`].
pub fn row_to_response(row: ConversationRow, data_dir: &Path) -> Result<ConversationResponse, AppError> {
    let extra: serde_json::Value =
        serde_json::from_str(&row.extra).map_err(|e| AppError::Internal(format!("Invalid extra JSON: {e}")))?;
    row_to_response_with_extra(row, extra, data_dir)
}

/// Same as [`row_to_response`] but takes a pre-parsed `extra` value. Used
/// by callers that need to mutate `extra` (e.g. lazy `skills` backfill)
/// before building the response DTO.
///
/// Injects a derived `is_temporary_workspace: bool` into the returned
/// `extra` blob by checking whether `extra.workspace` sits under the
/// backend-managed `data_dir`. The flag is not persisted — it is
/// computed on every read so the frontend never has to pattern-match
/// the directory name. Old rows that have no such flag on disk
/// automatically gain it on read, which means no migration is needed.
pub fn row_to_response_with_extra(
    row: ConversationRow,
    mut extra: serde_json::Value,
    data_dir: &Path,
) -> Result<ConversationResponse, AppError> {
    ConversationId::try_from(row.id.as_str()).map_err(|error| {
        AppError::Internal(format!("Invalid persisted conversation id '{}': {error}", row.id))
    })?;
    if let Some(template_id) = row.execution_template_id.as_deref() {
        AgentExecutionTemplateId::try_from(template_id).map_err(|error| {
            AppError::Internal(format!(
                "Invalid persisted execution_template_id '{template_id}': {error}"
            ))
        })?;
    }
    let is_temporary_workspace = {
        let ws = extra.get("workspace").and_then(|v| v.as_str()).unwrap_or("");
        // Companion sessions own a fixed, permanent per-companion work folder.
        // It sits under the data dir but is NOT a throwaway temp workspace —
        // mark it non-temporary so the chat tab keeps the "open workspace folder"
        // affordance and doesn't mislabel a locked, browsable work path.
        let is_companion = extra.get("companion_session").and_then(|v| v.as_bool()).unwrap_or(false);
        !is_companion && !ws.is_empty() && Path::new(ws).starts_with(data_dir)
    };
    if let Some(obj) = extra.as_object_mut() {
        obj.insert(
            "is_temporary_workspace".to_owned(),
            serde_json::Value::Bool(is_temporary_workspace),
        );
    }

    let agent_type: AgentType = string_to_enum(&row.r#type)?;
    let status: ConversationStatus = match row.status.as_deref() {
        None | Some("") => ConversationStatus::Finished,
        Some(s) => string_to_enum(s)?,
    };

    let source: Option<ConversationSource> = row.source.as_deref().map(string_to_enum).transpose()?;

    let model: Option<ProviderWithModel> = row.model.as_deref().map(parse_provider_with_model).transpose()?;
    let preset_snapshot = row
        .preset_snapshot
        .as_deref()
        .map(serde_json::from_str::<nomifun_api_types::ResolvedPresetSnapshot>)
        .transpose()
        .map_err(|error| AppError::Internal(format!("Invalid preset snapshot JSON: {error}")))?;
    let delegation_policy = string_to_enum(&row.delegation_policy)?;
    let execution_model_pool = row
        .execution_model_pool
        .as_deref()
        .map(serde_json::from_str::<nomifun_api_types::ExecutionModelPool>)
        .transpose()
        .map_err(|error| AppError::Internal(format!("Invalid execution model pool JSON: {error}")))?;
    if let Some(pool) = execution_model_pool.as_ref() {
        pool.validate().map_err(|error| {
            AppError::Internal(format!("Invalid persisted execution model pool: {error}"))
        })?;
    }
    let decision_policy = string_to_enum(&row.decision_policy)?;

    Ok(ConversationResponse {
        id: row.id,
        name: row.name,
        r#type: agent_type,
        model,
        status,
        runtime: None,
        source,
        pinned: row.pinned,
        pinned_at: row.pinned_at,
        channel_chat_id: row.channel_chat_id,
        preset_id: row.preset_id,
        preset_revision: row.preset_revision,
        preset_snapshot,
        delegation_policy,
        execution_model_pool,
        decision_policy,
        execution_template_id: row.execution_template_id,
        linked_execution_id: None,
        execution_step_id: None,
        execution_attempt_id: None,
        created_at: row.created_at,
        modified_at: row.updated_at,
        extra,
    })
}

/// Parse the model JSON column into `ProviderWithModel`.
///
/// Only the canonical persisted shape is accepted. Provider IDs use the
/// `prov_{uuid-v7}` entity grammar; model strings and optional overrides must
/// already be trimmed and non-empty.
pub(crate) fn parse_provider_with_model(s: &str) -> Result<ProviderWithModel, AppError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StoredProviderWithModel {
        provider_id: String,
        model: String,
        use_model: Option<String>,
    }

    let stored: StoredProviderWithModel = serde_json::from_str(s)
        .map_err(|e| AppError::Internal(format!("Invalid canonical model JSON: {e}")))?;
    ProviderId::try_from(stored.provider_id.as_str()).map_err(|error| {
        AppError::Internal(format!(
            "Invalid persisted provider_id '{}': {error}",
            stored.provider_id
        ))
    })?;
    if stored.model.trim().is_empty() || stored.model.trim() != stored.model {
        return Err(AppError::Internal(
            "Invalid persisted conversation model name".to_owned(),
        ));
    }
    if stored.use_model.as_deref().is_some_and(|model| {
        model.trim().is_empty() || model.trim() != model
    }) {
        return Err(AppError::Internal(
            "Invalid persisted conversation model override".to_owned(),
        ));
    }
    Ok(ProviderWithModel {
        provider_id: stored.provider_id,
        model: stored.model,
        use_model: stored.use_model,
    })
}

/// Parse a DB string value into a typed enum via serde.
///
/// e.g. `"acp"` → `AgentType::Acp`
pub fn string_to_enum<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, AppError> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|e| AppError::Internal(format!("Invalid enum value '{s}': {e}")))
}

/// Convert a message database row into an API response DTO.
pub fn row_to_message_response(row: MessageRow) -> Result<MessageResponse, AppError> {
    MessageId::try_from(row.id.as_str()).map_err(|error| {
        AppError::Internal(format!("Invalid persisted message id '{}': {error}", row.id))
    })?;
    ConversationId::try_from(row.conversation_id.as_str()).map_err(|error| {
        AppError::Internal(format!(
            "Invalid persisted message conversation_id '{}': {error}",
            row.conversation_id
        ))
    })?;
    if let Some(msg_id) = row.msg_id.as_deref() {
        MessageId::try_from(msg_id).map_err(|error| {
            AppError::Internal(format!("Invalid persisted message msg_id '{msg_id}': {error}"))
        })?;
    }
    let msg_type: MessageType = string_to_enum(&row.r#type)?;

    let position: Option<MessagePosition> = row.position.as_deref().map(string_to_enum).transpose()?;

    let status: Option<MessageStatus> = row.status.as_deref().map(string_to_enum).transpose()?;

    let mut content: serde_json::Value = serde_json::from_str(&row.content)
        .map_err(|e| AppError::Internal(format!("Invalid message content JSON: {e}")))?;
    project_interrupted_writeback_state(&mut content);

    Ok(MessageResponse {
        id: row.id,
        conversation_id: row.conversation_id,
        msg_id: row.msg_id,
        r#type: msg_type,
        content,
        position,
        status,
        hidden: row.hidden,
        created_at: row.created_at,
    })
}

fn project_interrupted_writeback_state(content: &mut serde_json::Value) {
    let Some(writeback) = content
        .as_object_mut()
        .and_then(|obj| obj.get_mut("knowledge_writeback"))
    else {
        return;
    };
    let Some(status) = writeback.get("status").and_then(|v| v.as_str()) else {
        return;
    };
    if !matches!(status, "started" | "extracting" | "writing") {
        return;
    }
    let updated_at = writeback
        .get("updated_at")
        .and_then(|v| v.as_i64())
        .or_else(|| writeback.get("started_at").and_then(|v| v.as_i64()))
        .unwrap_or_default();
    if now_ms().saturating_sub(updated_at) < WRITEBACK_RUNNING_STALE_MS {
        return;
    }
    if let Some(obj) = writeback.as_object_mut() {
        obj.insert("status".to_owned(), serde_json::json!("interrupted"));
        obj.insert("retryable".to_owned(), serde_json::json!(true));
        obj.insert("interrupted_at".to_owned(), serde_json::json!(now_ms()));
    }
}

/// Convert a message row for history-list use, compacting oversized tool payloads.
pub fn row_to_message_response_compact(row: MessageRow) -> Result<MessageResponse, AppError> {
    let original_size = row.content.len();
    let mut response = row_to_message_response(row)?;
    if !is_tool_message(response.r#type) || original_size <= TOOL_CONTENT_COMPACT_THRESHOLD_BYTES {
        return Ok(response);
    }

    let mut truncated = false;
    truncate_large_strings(&mut response.content, TOOL_CONTENT_PREVIEW_CHARS, &mut truncated);
    if truncated && let Some(obj) = response.content.as_object_mut() {
        obj.insert(
            "_compact".to_string(),
            serde_json::json!({
                "truncated": true,
                "original_size": original_size,
                "preview_chars": TOOL_CONTENT_PREVIEW_CHARS
            }),
        );
    }

    Ok(response)
}

fn is_tool_message(msg_type: MessageType) -> bool {
    matches!(
        msg_type,
        MessageType::ToolCall | MessageType::ToolGroup | MessageType::AcpToolCall
    )
}

fn truncate_large_strings(value: &mut serde_json::Value, max_chars: usize, truncated: &mut bool) {
    match value {
        serde_json::Value::String(text) if text.chars().count() > max_chars => {
            let preview: String = text.chars().take(max_chars).collect();
            *text = format!("{preview}\n...[truncated]");
            *truncated = true;
        }
        serde_json::Value::Array(items) => {
            for item in items {
                truncate_large_strings(item, max_chars, truncated);
            }
        }
        serde_json::Value::Object(map) => {
            for entry in map.values_mut() {
                truncate_large_strings(entry, max_chars, truncated);
            }
        }
        _ => {}
    }
}

/// Convert an artifact database row into an API response DTO.
pub fn row_to_artifact_response(row: ConversationArtifactRow) -> Result<ConversationArtifactResponse, AppError> {
    ConversationArtifactId::try_from(row.id.as_str()).map_err(|error| {
        AppError::Internal(format!(
            "Invalid persisted conversation artifact id '{}': {error}",
            row.id
        ))
    })?;
    ConversationId::try_from(row.conversation_id.as_str()).map_err(|error| {
        AppError::Internal(format!(
            "Invalid persisted artifact conversation_id '{}': {error}",
            row.conversation_id
        ))
    })?;
    if let Some(cron_job_id) = row.cron_job_id.as_deref() {
        CronJobId::try_from(cron_job_id).map_err(|error| {
            AppError::Internal(format!(
                "Invalid persisted artifact cron_job_id '{cron_job_id}': {error}"
            ))
        })?;
    }
    let kind = string_to_enum(&row.kind)?;
    let status = string_to_enum(&row.status)?;
    let payload: serde_json::Value = serde_json::from_str(&row.payload)
        .map_err(|e| AppError::Internal(format!("Invalid artifact payload JSON: {e}")))?;

    Ok(ConversationArtifactResponse {
        id: row.id,
        conversation_id: row.conversation_id,
        cron_job_id: row.cron_job_id,
        kind,
        status,
        payload,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Extract plain-text preview from a message content field.
///
/// Message content is stored as JSON (arrays, objects with nested strings).
/// This recursively collects all string values and joins them with spaces,
/// producing a flat preview suitable for search snippet display.
fn extract_preview_text(raw_content: &str) -> String {
    fn collect_strings(value: &serde_json::Value, bucket: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    bucket.push(trimmed.to_owned());
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    collect_strings(item, bucket);
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values() {
                    collect_strings(item, bucket);
                }
            }
            _ => {}
        }
    }

    match serde_json::from_str::<serde_json::Value>(raw_content) {
        Ok(parsed) => {
            let mut bucket = Vec::new();
            collect_strings(&parsed, &mut bucket);
            let joined = bucket.join(" ");
            let normalized = joined.split_whitespace().collect::<Vec<_>>().join(" ");
            if normalized.is_empty() {
                raw_content.split_whitespace().collect::<Vec<_>>().join(" ")
            } else {
                normalized
            }
        }
        Err(_) => raw_content.split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

/// Convert a search result row into an API search item DTO.
pub fn search_row_to_item(row: MessageSearchRow, data_dir: &Path) -> Result<MessageSearchItem, AppError> {
    MessageId::try_from(row.message_id.as_str()).map_err(|error| {
        AppError::Internal(format!(
            "Invalid persisted search message_id '{}': {error}",
            row.message_id
        ))
    })?;
    let conversation_row = ConversationRow {
        id: row.conversation_id,
        user_id: String::new(),
        name: row.conversation_name,
        r#type: row.conversation_type,
        extra: row.conversation_extra,
        delegation_policy: row.conversation_delegation_policy,
        execution_model_pool: row.conversation_execution_model_pool,
        decision_policy: row.conversation_decision_policy,
        execution_template_id: row.conversation_execution_template_id,
        model: row.conversation_model,
        status: row.conversation_status,
        source: row.conversation_source,
        channel_chat_id: row.conversation_channel_chat_id,
        pinned: row.conversation_pinned,
        pinned_at: row.conversation_pinned_at,
        // Search rows don't project `cron_job_id`; it isn't needed for the
        // search-result conversation summary (no artifact card rendered there).
        cron_job_id: None,
        preset_id: None,
        preset_revision: None,
        preset_snapshot: None,
        created_at: row.conversation_created_at,
        updated_at: row.conversation_updated_at,
    };

    let conversation = row_to_response(conversation_row, data_dir)?;
    let preview_text = extract_preview_text(&row.content);

    Ok(MessageSearchItem {
        message_id: row.message_id,
        message_type: row.r#type,
        message_created_at: row.created_at,
        preview_text,
        conversation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{AgentType, ConversationId, ConversationSource, ConversationStatus};
    use serde_json::json;

    const PROVIDER_ID: &str = "prov_0190f5fe-7c00-7a00-8000-000000000001";
    const MESSAGE_ID: &str = "msg_0190f5fe-7c00-7a00-8000-000000000001";

    fn make_row(
        agent_type: &str,
        status: &str,
        source: Option<&str>,
        model_json: Option<&str>,
        extra_json: &str,
    ) -> ConversationRow {
        ConversationRow {
            id: ConversationId::new().into_string(),
            user_id: "user_1".into(),
            name: "Test".into(),
            r#type: agent_type.into(),
            extra: extra_json.into(),
            delegation_policy: "automatic".into(),
            execution_model_pool: None,
            decision_policy: "automatic".into(),
            execution_template_id: None,
            model: model_json.map(|s| s.into()),
            status: Some(status.into()),
            source: source.map(|s| s.into()),
            channel_chat_id: None,
            pinned: false,
            pinned_at: None,
            cron_job_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            created_at: 1000,
            updated_at: 2000,
        }
    }

    fn make_message_row(content: serde_json::Value) -> MessageRow {
        MessageRow {
            id: MESSAGE_ID.into(),
            conversation_id: ConversationId::new().into_string(),
            msg_id: Some(MESSAGE_ID.into()),
            r#type: "text".into(),
            content: content.to_string(),
            position: Some("left".into()),
            status: Some("finish".into()),
            hidden: false,
            created_at: 1000,
        }
    }

    #[test]
    fn row_to_response_basic() {
        let model = json!({"provider_id": PROVIDER_ID, "model": "m1"});
        let row = make_row(
            "acp",
            "pending",
            Some("nomifun"),
            Some(&model.to_string()),
            r#"{"workspace": "/project"}"#,
        );
        let resp = row_to_response(row, Path::new("/tmp/data")).unwrap();
        assert!(ConversationId::try_from(resp.id.as_str()).is_ok());
        assert_eq!(resp.r#type, AgentType::Acp);
        assert_eq!(resp.status, ConversationStatus::Pending);
        assert_eq!(resp.source, Some(ConversationSource::Nomifun));
        assert_eq!(resp.model.unwrap().model, "m1");
        assert_eq!(resp.extra["workspace"], "/project");
        assert_eq!(resp.modified_at, 2000);
    }

    #[test]
    fn row_to_response_no_source() {
        let row = make_row("acp", "running", None, None, "{}");
        let resp = row_to_response(row, Path::new("/tmp/data")).unwrap();
        assert!(resp.source.is_none());
        assert!(resp.model.is_none());
    }

    #[test]
    fn row_to_response_invalid_type() {
        let row = make_row("invalid", "pending", None, None, "{}");
        let err = row_to_response(row, Path::new("/tmp/data")).unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn row_to_response_invalid_extra_json() {
        let row = ConversationRow {
            id: ConversationId::new().into_string(),
            user_id: "user_1".into(),
            name: "Test".into(),
            r#type: "acp".into(),
            extra: "not-json".into(),
            delegation_policy: "automatic".into(),
            execution_model_pool: None,
            decision_policy: "automatic".into(),
            execution_template_id: None,
            model: None,
            status: Some("pending".into()),
            source: None,
            channel_chat_id: None,
            pinned: false,
            pinned_at: None,
            cron_job_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            created_at: 1000,
            updated_at: 2000,
        };
        let err = row_to_response(row, Path::new("/tmp/data")).unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn string_to_enum_valid() {
        let agent: AgentType = string_to_enum("acp").unwrap();
        assert_eq!(agent, AgentType::Acp);

        let status: ConversationStatus = string_to_enum("finished").unwrap();
        assert_eq!(status, ConversationStatus::Finished);

        let src: ConversationSource = string_to_enum("telegram").unwrap();
        assert_eq!(src, ConversationSource::Telegram);
    }

    #[test]
    fn string_to_enum_invalid() {
        let err = string_to_enum::<AgentType>("not_valid").unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn parse_provider_with_model_canonical_format() {
        let json = format!(r#"{{"provider_id":"{PROVIDER_ID}","model":"claude-sonnet-4-20250514","use_model":"claude-sonnet"}}"#);
        let result = parse_provider_with_model(&json).unwrap();
        assert_eq!(result.provider_id, PROVIDER_ID);
        assert_eq!(result.model, "claude-sonnet-4-20250514");
        assert_eq!(result.use_model.as_deref(), Some("claude-sonnet"));
    }

    #[test]
    fn parse_provider_with_model_rejects_legacy_provider_object() {
        let json = format!(r#"{{"id":"{PROVIDER_ID}","platform":"openai","useModel":"gpt-4-turbo"}}"#);
        assert!(parse_provider_with_model(&json).is_err());
    }

    #[test]
    fn parse_provider_with_model_missing_both_ids() {
        let json = r#"{"name":"invalid"}"#;
        assert!(parse_provider_with_model(json).is_err());
    }

    #[test]
    fn row_to_response_marks_workspace_inside_data_dir_as_temporary() {
        let row = make_row(
            "acp",
            "pending",
            Some("nomifun"),
            None,
            r#"{"workspace":"/srv/nomifun-data/conversations/claude-temp-abc"}"#,
        );
        let resp = row_to_response(row, Path::new("/srv/nomifun-data")).unwrap();
        assert_eq!(resp.extra["is_temporary_workspace"], true);
    }

    #[test]
    fn row_to_response_marks_workspace_outside_data_dir_as_non_temporary() {
        let row = make_row(
            "acp",
            "pending",
            Some("nomifun"),
            None,
            r#"{"workspace":"/Users/alice/my-project"}"#,
        );
        let resp = row_to_response(row, Path::new("/srv/nomifun-data")).unwrap();
        assert_eq!(resp.extra["is_temporary_workspace"], false);
    }

    #[test]
    fn row_to_response_marks_missing_workspace_as_non_temporary() {
        let row = make_row("acp", "pending", Some("nomifun"), None, r#"{}"#);
        let resp = row_to_response(row, Path::new("/srv/nomifun-data")).unwrap();
        assert_eq!(resp.extra["is_temporary_workspace"], false);
    }

    #[test]
    fn row_to_response_marks_companion_workspace_as_non_temporary() {
        // A companion's fixed work folder sits under the data dir but is a
        // permanent per-companion workspace, not a throwaway temp one — the
        // `companion_session` flag must override the under-data-dir heuristic.
        let row = make_row(
            "nomi",
            "pending",
            Some("nomifun"),
            None,
            r#"{"companion_session":true,"workspace":"/srv/nomifun-data/companion/companions/companion_x/workspace"}"#,
        );
        let resp = row_to_response(row, Path::new("/srv/nomifun-data")).unwrap();
        assert_eq!(resp.extra["is_temporary_workspace"], false);
    }

    #[test]
    fn row_with_pinned_at() {
        let row = ConversationRow {
            id: ConversationId::new().into_string(),
            user_id: "user_1".into(),
            name: "Pinned".into(),
            r#type: "acp".into(),
            extra: "{}".into(),
            delegation_policy: "automatic".into(),
            execution_model_pool: None,
            decision_policy: "automatic".into(),
            execution_template_id: None,
            model: None,
            status: Some("pending".into()),
            source: Some("nomifun".into()),
            channel_chat_id: Some("chat:1".into()),
            pinned: true,
            pinned_at: Some(5000),
            cron_job_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            created_at: 1000,
            updated_at: 3000,
        };
        let resp = row_to_response(row, Path::new("/tmp/data")).unwrap();
        assert!(resp.pinned);
        assert_eq!(resp.pinned_at, Some(5000));
        assert_eq!(resp.channel_chat_id.as_deref(), Some("chat:1"));
    }

    // ── extract_preview_text ───────────────────────────────────────────

    #[test]
    fn test_extract_preview_text_json_array() {
        let content = r#"[{"type":"text","content":"Hello world"},{"type":"text","content":"How are you?"}]"#;
        let result = extract_preview_text(content);
        assert!(result.contains("Hello world"));
        assert!(result.contains("How are you?"));
    }

    #[test]
    fn test_extract_preview_text_plain_string() {
        let content = "Just plain text message";
        let result = extract_preview_text(content);
        assert_eq!(result, "Just plain text message");
    }

    #[test]
    fn test_extract_preview_text_nested_object() {
        let content = r#"{"text":"nested value","items":[{"content":"inner"}]}"#;
        let result = extract_preview_text(content);
        assert!(result.contains("nested value"));
        assert!(result.contains("inner"));
    }

    #[test]
    fn test_extract_preview_text_malformed_json() {
        let content = "this is not { json at all";
        let result = extract_preview_text(content);
        assert_eq!(result, "this is not { json at all");
    }

    #[test]
    fn test_extract_preview_text_empty_content() {
        let result = extract_preview_text("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_extract_preview_text_whitespace_normalization() {
        let content = r#"{"content":"  hello   world  "}"#;
        let result = extract_preview_text(content);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn row_to_message_response_projects_stale_writeback_as_interrupted() {
        let stale_at = now_ms() - WRITEBACK_RUNNING_STALE_MS - 1;
        let row = make_message_row(json!({
            "content": "answer",
            "knowledge_writeback": {
                "status": "writing",
                "attempt_id": "msg_1:1",
                "started_at": stale_at,
                "updated_at": stale_at,
                "retryable": false
            }
        }));

        let resp = row_to_message_response(row).unwrap();

        assert_eq!(resp.content["knowledge_writeback"]["status"], "interrupted");
        assert_eq!(resp.content["knowledge_writeback"]["retryable"], true);
        assert!(resp.content["knowledge_writeback"]["interrupted_at"].is_i64());
    }

    // ── search_row_to_item ─────────────────────────────────────────────

    #[test]
    fn test_search_row_to_item_builds_nested_conversation() {
        let conversation_id = ConversationId::new().into_string();
        let row = MessageSearchRow {
            message_id: MESSAGE_ID.into(),
            r#type: "text".into(),
            content: r#"{"content":"hello world"}"#.into(),
            created_at: 5000,
            conversation_id: conversation_id.clone(),
            conversation_name: "Test Conv".into(),
            conversation_type: "acp".into(),
            conversation_extra: r#"{"workspace":"/project"}"#.into(),
            conversation_delegation_policy: "prefer_parallel".into(),
            conversation_execution_model_pool: Some(
                format!(r#"{{"mode":"range","models":[{{"provider_id":"{PROVIDER_ID}","model":"model-1"}}]}}"#),
            ),
            conversation_decision_policy: "ask_user".into(),
            conversation_execution_template_id: None,
            conversation_model: None,
            conversation_status: Some("finished".into()),
            conversation_source: Some("nomifun".into()),
            conversation_channel_chat_id: None,
            conversation_pinned: false,
            conversation_pinned_at: None,
            conversation_created_at: 1000,
            conversation_updated_at: 2000,
        };

        let item = search_row_to_item(row, Path::new("/tmp/data")).unwrap();

        assert_eq!(item.message_id, MESSAGE_ID);
        assert_eq!(item.message_type, "text");
        assert_eq!(item.message_created_at, 5000);
        assert_eq!(item.preview_text, "hello world");

        assert_eq!(item.conversation.id, conversation_id);
        assert_eq!(item.conversation.name, "Test Conv");
        assert_eq!(item.conversation.r#type, AgentType::Acp);
        assert_eq!(item.conversation.source, Some(ConversationSource::Nomifun));
        assert_eq!(
            item.conversation.delegation_policy,
            nomifun_common::DelegationPolicy::PreferParallel
        );
        assert_eq!(
            item.conversation.execution_model_pool,
            Some(nomifun_api_types::ExecutionModelPool::Range {
                models: vec![nomifun_api_types::ExecutionModelRef {
                    provider_id: PROVIDER_ID.into(),
                    model: "model-1".into(),
                }],
            })
        );
        assert_eq!(
            item.conversation.decision_policy,
            nomifun_common::DecisionPolicy::AskUser
        );
        assert_eq!(item.conversation.extra["workspace"], "/project");
        assert_eq!(item.conversation.modified_at, 2000);
    }

    #[test]
    fn test_search_row_to_item_invalid_conversation_type() {
        let row = MessageSearchRow {
            message_id: MESSAGE_ID.into(),
            r#type: "text".into(),
            content: "plain text".into(),
            created_at: 5000,
            conversation_id: ConversationId::new().into_string(),
            conversation_name: "Test".into(),
            conversation_type: "invalid_type".into(),
            conversation_extra: "{}".into(),
            conversation_delegation_policy: "automatic".into(),
            conversation_execution_model_pool: None,
            conversation_decision_policy: "automatic".into(),
            conversation_execution_template_id: None,
            conversation_model: None,
            conversation_status: Some("finished".into()),
            conversation_source: None,
            conversation_channel_chat_id: None,
            conversation_pinned: false,
            conversation_pinned_at: None,
            conversation_created_at: 1000,
            conversation_updated_at: 2000,
        };

        let err = search_row_to_item(row, Path::new("/tmp/data")).unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn test_search_row_to_item_invalid_conversation_extra_json() {
        let row = MessageSearchRow {
            message_id: MESSAGE_ID.into(),
            r#type: "text".into(),
            content: r#"{"content":"hello"}"#.into(),
            created_at: 5000,
            conversation_id: ConversationId::new().into_string(),
            conversation_name: "Test".into(),
            conversation_type: "acp".into(),
            conversation_extra: "not valid json".into(),
            conversation_delegation_policy: "automatic".into(),
            conversation_execution_model_pool: None,
            conversation_decision_policy: "automatic".into(),
            conversation_execution_template_id: None,
            conversation_model: None,
            conversation_status: Some("finished".into()),
            conversation_source: None,
            conversation_channel_chat_id: None,
            conversation_pinned: false,
            conversation_pinned_at: None,
            conversation_created_at: 1000,
            conversation_updated_at: 2000,
        };

        let err = search_row_to_item(row, Path::new("/tmp/data")).unwrap_err();
        assert!(matches!(err, AppError::Internal(_)));
    }
}

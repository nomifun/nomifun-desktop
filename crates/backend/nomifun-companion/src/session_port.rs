//! Typed Session boundary for Companion thread management.

use std::sync::Arc;

use async_trait::async_trait;
#[cfg(test)]
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ResolvedPresetSnapshot,
    UpdateConversationRequest,
};
#[cfg(test)]
use nomifun_api_types::{ListMessagesQuery, MessageResponse};
use nomifun_common::AppError;
#[cfg(test)]
use nomifun_common::{MessagePosition, MessageType};
#[cfg(test)]
use nomifun_conversation::ConversationService;
use nomifun_db::MessageDayBucket;

use crate::archive_port::{CompanionArchiveSessionPort, SessionArchivePort};
use crate::archiver::ArchiveConversationPort;
#[cfg(test)]
use crate::archiver::WindowMessage;
#[cfg(test)]
use crate::evolution::ConversationTranscriptSource;
use crate::evolution::TranscriptSource;

/// Narrow command/query surface used by Companion thread management.
#[async_trait]
pub trait CompanionSessionPort: Send + Sync {
    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn replace_skill_snapshot(
        &self,
        session_id: &str,
        skills: &[String],
    ) -> Result<bool, AppError>;

    async fn update_extra(
        &self,
        session_id: &str,
        patch: serde_json::Value,
    ) -> Result<(), AppError>;

    async fn create(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<ResolvedPresetSnapshot>,
    ) -> Result<ConversationResponse, AppError>;

    async fn delete(&self, owner_id: &str, session_id: &str) -> Result<(), AppError>;

    async fn update(
        &self,
        owner_id: &str,
        session_id: &str,
        request: UpdateConversationRequest,
    ) -> Result<ConversationResponse, AppError>;

    async fn message_local_day_index(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<Vec<MessageDayBucket>, AppError>;
}

/// All late-bound Session collaborators needed by Companion.
pub struct CompanionHostPorts {
    pub sessions: Arc<dyn CompanionSessionPort>,
    pub archive: Arc<dyn ArchiveConversationPort>,
    pub transcript: Arc<dyn TranscriptSource>,
}

#[cfg(test)]
struct ConversationCompanionSessionPort {
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

/// Transitional host implementation for the archive-specific Session contract.
///
/// All Conversation DTO/metadata translation is kept here, at the edge. The
/// archiver and its typed host contract only see normalized dialogue messages.
#[cfg(test)]
struct ConversationCompanionArchiveSessionPort {
    service: Arc<ConversationService>,
}

/// Best-effort plain-text extraction from a message's `content` JSON. Structured
/// tool payloads intentionally do not cross the archive Session boundary.
#[cfg(test)]
fn extract_archive_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => map
            .get("text")
            .or_else(|| map.get("content"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
fn project_archive_message(
    message: nomifun_api_types::MessageResponse,
    since_ts: i64,
) -> Option<WindowMessage> {
    if message.hidden
        || message.created_at <= since_ts
        || message.r#type != MessageType::Text
    {
        return None;
    }
    let is_user = match message.position {
        Some(MessagePosition::Right) => true,
        Some(MessagePosition::Left) => false,
        _ => return None,
    };
    let content = extract_archive_text(&message.content);
    if content.trim().is_empty() {
        return None;
    }
    Some(WindowMessage {
        is_user,
        content,
        created_at: message.created_at,
    })
}

#[cfg(test)]
#[async_trait]
impl CompanionSessionPort for ConversationCompanionSessionPort {
    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError> {
        self.service.get(owner_id, session_id).await
    }

    async fn replace_skill_snapshot(
        &self,
        session_id: &str,
        skills: &[String],
    ) -> Result<bool, AppError> {
        self.service
            .replace_skill_snapshot(session_id, skills)
            .await
    }

    async fn update_extra(
        &self,
        session_id: &str,
        patch: serde_json::Value,
    ) -> Result<(), AppError> {
        self.service.update_extra(session_id, patch).await
    }

    async fn create(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<ResolvedPresetSnapshot>,
    ) -> Result<ConversationResponse, AppError> {
        match snapshot {
            Some(snapshot) => {
                self.service
                    .create_from_preset_snapshot(owner_id, request, snapshot)
                    .await
            }
            None => self.service.create(owner_id, request).await,
        }
    }

    async fn delete(&self, owner_id: &str, session_id: &str) -> Result<(), AppError> {
        self.service.delete(owner_id, session_id).await
    }

    async fn update(
        &self,
        owner_id: &str,
        session_id: &str,
        request: UpdateConversationRequest,
    ) -> Result<ConversationResponse, AppError> {
        self.service
            .update(
                owner_id,
                session_id,
                request,
                &self.runtime_registry,
            )
            .await
    }

    async fn message_local_day_index(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<Vec<MessageDayBucket>, AppError> {
        self.service
            .message_local_day_index(owner_id, session_id)
            .await
    }
}

#[cfg(test)]
#[async_trait]
impl CompanionArchiveSessionPort for ConversationCompanionArchiveSessionPort {
    async fn window_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        since_ts: i64,
        limit: u32,
    ) -> Result<Vec<WindowMessage>, AppError> {
        // Keyset pagination returns the newest bounded window; the conversation
        // service presents that page oldest-first for history consumers.
        let query = ListMessagesQuery {
            cursor: Some(String::new()),
            page_size: Some(limit),
            ..Default::default()
        };
        let response = self.service.list_messages(owner_id, session_id, query).await?;
        Ok(response
            .items
            .into_iter()
            .filter_map(|message| project_archive_message(message, since_ts))
            .collect())
    }

    async fn reset_context(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<(), AppError> {
        // `clear_context` handles both warm and cold sessions without
        // manufacturing a runtime merely to erase archived context.
        self.service.clear_context(owner_id, session_id).await
    }
}

/// Build Companion's transitional Conversation-backed ports.
///
/// All adapters delegate to the same owner/repository. They retain no Session
/// facts, runtime cache, fallback, or alternate identity.
#[cfg(test)]
pub fn conversation_companion_ports(
    owner_id: Arc<str>,
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> CompanionHostPorts {
    companion_ports_with_session(
        owner_id,
        service.clone(),
        Arc::new(ConversationCompanionSessionPort {
            service,
            runtime_registry,
        }),
    )
}

/// Build Companion's host ports from an already-composed Session owner.
///
/// Production Nomi-core assembly uses this entry point so the domain does not
/// construct another Conversation-backed Session adapter. The archive and
/// transcript readers remain small, read-only adapters over the same service.
#[cfg(test)]
pub fn companion_ports_with_session(
    owner_id: Arc<str>,
    service: Arc<ConversationService>,
    sessions: Arc<dyn CompanionSessionPort>,
) -> CompanionHostPorts {
    let repo = service.conversation_repo().clone();
    let archive_sessions: Arc<dyn CompanionArchiveSessionPort> =
        Arc::new(ConversationCompanionArchiveSessionPort {
            service: service.clone(),
        });
    companion_ports_from_typed_host(
        owner_id,
        sessions,
        archive_sessions,
        Arc::new(ConversationTranscriptSource::new(repo)),
    )
}

/// Compose Companion from host-owned typed Session collaborators.
///
/// This is the migration target for the application host: the caller supplies
/// the normal thread-management port, the archive-specific port, and the
/// transcript source independently. No Conversation implementation is needed
/// by this composition function.
pub fn companion_ports_from_typed_host(
    owner_id: Arc<str>,
    sessions: Arc<dyn CompanionSessionPort>,
    archive_sessions: Arc<dyn CompanionArchiveSessionPort>,
    transcript: Arc<dyn TranscriptSource>,
) -> CompanionHostPorts {
    CompanionHostPorts {
        sessions,
        archive: Arc::new(SessionArchivePort::new(owner_id, archive_sessions)),
        transcript,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        message_type: MessageType,
        position: Option<MessagePosition>,
        hidden: bool,
        created_at: i64,
        content: serde_json::Value,
    ) -> MessageResponse {
        MessageResponse {
            message_id: "0190f5fe-7c00-7a00-8abc-000000000001".to_owned(),
            conversation_id: "0190f5fe-7c00-7a00-8abc-000000000002".to_owned(),
            msg_id: None,
            r#type: message_type,
            content,
            position,
            status: None,
            hidden,
            created_at,
        }
    }

    #[test]
    fn archive_projection_keeps_only_visible_text_dialogue() {
        let messages = vec![
            message(
                MessageType::Text,
                Some(MessagePosition::Right),
                false,
                11,
                serde_json::json!("owner"),
            ),
            message(
                MessageType::Text,
                Some(MessagePosition::Left),
                false,
                12,
                serde_json::json!({"text": "companion"}),
            ),
            message(
                MessageType::Text,
                Some(MessagePosition::Left),
                false,
                13,
                serde_json::json!({"content": "fallback"}),
            ),
            message(
                MessageType::Text,
                Some(MessagePosition::Center),
                false,
                14,
                serde_json::json!("system"),
            ),
            message(
                MessageType::ToolCall,
                Some(MessagePosition::Right),
                false,
                15,
                serde_json::json!("tool"),
            ),
            message(
                MessageType::Text,
                Some(MessagePosition::Right),
                true,
                16,
                serde_json::json!("hidden"),
            ),
            message(
                MessageType::Text,
                Some(MessagePosition::Right),
                false,
                10,
                serde_json::json!("before-boundary"),
            ),
        ];

        let projected: Vec<_> = messages
            .into_iter()
            .filter_map(|message| project_archive_message(message, 10))
            .collect();
        assert_eq!(
            projected
                .iter()
                .map(|message| (message.is_user, message.content.as_str(), message.created_at))
                .collect::<Vec<_>>(),
            vec![
                (true, "owner", 11),
                (false, "companion", 12),
                (false, "fallback", 13),
            ]
        );
    }
}

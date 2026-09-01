//! Typed Session boundary for Companion thread management.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ResolvedPresetSnapshot,
    UpdateConversationRequest,
};
use nomifun_common::AppError;
use nomifun_conversation::ConversationService;
use nomifun_db::MessageDayBucket;

use crate::archive_port::ConversationArchivePort;
use crate::archiver::ArchiveConversationPort;
use crate::evolution::{ConversationTranscriptSource, TranscriptSource};

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

struct ConversationCompanionSessionPort {
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
}

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

/// Build Companion's transitional Conversation-backed ports.
///
/// All adapters delegate to the same owner/repository. They retain no Session
/// facts, runtime cache, fallback, or alternate identity.
pub fn conversation_companion_ports(
    owner_id: Arc<str>,
    service: Arc<ConversationService>,
    runtime_registry: Arc<dyn AgentRuntimeRegistry>,
) -> CompanionHostPorts {
    let repo = service.conversation_repo().clone();
    CompanionHostPorts {
        sessions: Arc::new(ConversationCompanionSessionPort {
            service: service.clone(),
            runtime_registry,
        }),
        archive: Arc::new(ConversationArchivePort::new(owner_id, service)),
        transcript: Arc::new(ConversationTranscriptSource::new(repo)),
    }
}

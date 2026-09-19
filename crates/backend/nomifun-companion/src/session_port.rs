//! Typed Session boundary for Companion thread management.

use std::sync::Arc;

use async_trait::async_trait;

use nomifun_api_types::{ConversationResponse, CreateConversationRequest};
use nomifun_common::AppError;

use nomifun_db::MessageDayBucket;

use crate::archive_port::{CompanionArchiveSessionPort, SessionArchivePort};
use crate::archiver::ArchiveConversationPort;

use crate::evolution::TranscriptSource;

/// Narrow command/query surface used by Companion thread management.
#[async_trait]
pub trait CompanionSessionPort: Send + Sync {
    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn create(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
    ) -> Result<ConversationResponse, AppError>;

    async fn delete(&self, owner_id: &str, session_id: &str) -> Result<(), AppError>;

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

/// Transitional host implementation for the archive-specific Session contract.
///
/// All Conversation DTO/metadata translation is kept here, at the edge. The
/// archiver and its typed host contract only see normalized dialogue messages.

/// Best-effort plain-text extraction from a message's `content` JSON. Structured
/// tool payloads intentionally do not cross the archive Session boundary.

/// Build Companion's transitional Conversation-backed ports.
///
/// All adapters delegate to the same owner/repository. They retain no Session
/// facts, runtime cache, fallback, or alternate identity.

/// Build Companion's host ports from an already-composed Session owner.
///
/// Production Nomi-core assembly uses this entry point so the domain does not
/// construct another Conversation-backed Session adapter. The archive and
/// transcript readers remain small, read-only adapters over the same service.

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

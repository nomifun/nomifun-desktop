use nomifun_common::{PaginatedResult, TimestampMs};
use serde::{Deserialize, Serialize};

use crate::error::DbError;
use crate::models::{ConversationArtifactRow, ConversationRow, MessageRow};

/// Conversation + message data access abstraction.
///
/// Covers conversation CRUD, extended queries (source/chat, cron-job,
/// associated workspace), and message operations (list, insert, update,
/// delete, search).
///
/// Object-safe via `async_trait` to support `Arc<dyn IConversationRepository>`.
#[async_trait::async_trait]
pub trait IConversationRepository: Send + Sync {
    // ── Conversation CRUD ───────────────────────────────────────────

    /// Returns a conversation by ID, or `None` if not found.
    async fn get(&self, id: i64) -> Result<Option<ConversationRow>, DbError>;

    /// Inserts a new conversation row. The `id` field of `row` is ignored: the
    /// id is allocated by SQLite (INTEGER PK AUTOINCREMENT) and returned.
    async fn create(&self, row: &ConversationRow) -> Result<i64, DbError>;

    /// Partially updates a conversation. Returns `DbError::NotFound` if ID is missing.
    async fn update(&self, id: i64, updates: &ConversationRowUpdate) -> Result<(), DbError>;

    /// Deletes a conversation (messages cascade via FK).
    /// Returns `DbError::NotFound` if ID is missing.
    async fn delete(&self, id: i64) -> Result<(), DbError>;

    /// Lists conversations with cursor-based pagination and optional filters.
    async fn list_paginated(
        &self,
        user_id: &str,
        filters: &ConversationFilters,
    ) -> Result<PaginatedResult<ConversationRow>, DbError>;

    // ── Extended queries ────────────────────────────────────────────

    /// Finds a conversation by source, channel chat ID, and agent type.
    async fn find_by_source_and_chat(
        &self,
        user_id: &str,
        source: &str,
        chat_id: &str,
        agent_type: &str,
    ) -> Result<Option<ConversationRow>, DbError>;

    /// Lists conversations created by the given cron job (`cron_job_id` column).
    async fn list_by_cron_job(&self, user_id: &str, cron_job_id: &str) -> Result<Vec<ConversationRow>, DbError>;

    /// Lists conversations sharing the same `extra.workspace` value.
    /// The conversation identified by `conversation_id` is excluded.
    async fn list_associated(&self, user_id: &str, conversation_id: i64) -> Result<Vec<ConversationRow>, DbError>;

    // ── conversation_mcp_servers junction ───────────────────────────

    /// Returns the MCP server IDs selected for a conversation, ordered by
    /// `sort_order`. Replaces the legacy `extra.selected_mcp_server_ids` array.
    async fn list_mcp_server_ids(&self, _conversation_id: i64) -> Result<Vec<i64>, DbError> {
        Ok(Vec::new())
    }

    /// Replaces the conversation's selected MCP server set with `ids`, preserving
    /// order via `sort_order`. Implemented as a single DELETE + ordered INSERT
    /// transaction. Replaces writes to `extra.selected_mcp_server_ids`.
    async fn set_mcp_server_ids(&self, _conversation_id: i64, _ids: &[i64]) -> Result<(), DbError> {
        Ok(())
    }

    // ── Message operations ──────────────────────────────────────────

    /// Returns paginated messages for a conversation, ordered by `created_at`.
    async fn get_messages(
        &self,
        conv_id: i64,
        page: u32,
        page_size: u32,
        order: SortOrder,
    ) -> Result<PaginatedResult<MessageRow>, DbError>;

    /// Keyset (cursor) pagination: returns up to `limit` messages strictly OLDER
    /// than `before` `(created_at, id)`, newest-first (`created_at DESC, id DESC`);
    /// `before: None` returns the newest `limit`. `has_more` means an older page
    /// exists. Used to incrementally load an ever-growing conversation (e.g. a
    /// companion's single session) without fetching the whole transcript, and is
    /// stable under concurrent appends (unlike OFFSET). `total` is not computed
    /// (returned as 0). Default returns empty so mock repos compile; the SQLite
    /// repo overrides it.
    async fn get_messages_keyset(
        &self,
        _conv_id: i64,
        _before: Option<(i64, String)>,
        _limit: u32,
    ) -> Result<PaginatedResult<MessageRow>, DbError> {
        Ok(PaginatedResult {
            items: Vec::new(),
            total: 0,
            has_more: false,
        })
    }

    /// Returns a single message scoped to a conversation.
    async fn get_message(&self, _conv_id: i64, _message_id: &str) -> Result<Option<MessageRow>, DbError> {
        Ok(None)
    }

    /// Inserts a new message row.
    async fn insert_message(&self, message: &MessageRow) -> Result<(), DbError>;

    /// Partially updates a message. Returns `DbError::NotFound` if ID is missing.
    async fn update_message(&self, id: &str, updates: &MessageRowUpdate) -> Result<(), DbError>;

    /// Deletes all messages belonging to a conversation.
    async fn delete_messages_by_conversation(&self, conv_id: i64) -> Result<(), DbError>;

    /// Finds a message by (conversation_id, msg_id, type) triple.
    async fn get_message_by_msg_id(
        &self,
        conv_id: i64,
        msg_id: &str,
        msg_type: &str,
    ) -> Result<Option<MessageRow>, DbError>;

    /// Full-text search across messages, joining conversation name.
    async fn search_messages(
        &self,
        user_id: &str,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<PaginatedResult<MessageSearchRow>, DbError>;

    /// Returns persisted conversation artifacts ordered by `created_at`.
    async fn list_artifacts(&self, _conversation_id: i64) -> Result<Vec<ConversationArtifactRow>, DbError> {
        Ok(Vec::new())
    }

    /// Returns a conversation artifact by ID scoped to a conversation.
    async fn get_artifact(
        &self,
        _conversation_id: i64,
        _artifact_id: i64,
    ) -> Result<Option<ConversationArtifactRow>, DbError> {
        Ok(None)
    }

    /// Inserts or updates a conversation artifact.
    ///
    /// Idempotency is keyed by `kind`:
    /// - `cron_trigger`: always a fresh INSERT (one row per trigger), returning
    ///   the row with its auto-assigned `id`.
    /// - `skill_suggest`: upsert against the partial UNIQUE
    ///   `(conversation_id, cron_job_id) WHERE kind = 'skill_suggest'`.
    ///
    /// The `id` field of the input is ignored (it is allocated by SQLite).
    async fn upsert_artifact(&self, artifact: &ConversationArtifactRow) -> Result<ConversationArtifactRow, DbError> {
        Ok(artifact.clone())
    }

    /// Updates artifact status and returns the updated row if found.
    async fn update_artifact_status(
        &self,
        _conversation_id: i64,
        _artifact_id: i64,
        _status: &str,
        _updated_at: TimestampMs,
    ) -> Result<Option<ConversationArtifactRow>, DbError> {
        Ok(None)
    }

    /// Marks all skill suggestion artifacts for a cron job as saved.
    async fn mark_skill_suggest_artifacts_saved(
        &self,
        _cron_job_id: &str,
        _updated_at: TimestampMs,
    ) -> Result<Vec<ConversationArtifactRow>, DbError> {
        Ok(Vec::new())
    }

    /// Deletes all artifacts belonging to a conversation.
    async fn delete_artifacts_by_conversation(&self, _conversation_id: i64) -> Result<(), DbError> {
        Ok(())
    }

    /// Returns legacy persisted cron trigger rows so callers can synthesize
    /// artifact cards for historical conversations created before artifact migration.
    async fn list_legacy_cron_trigger_messages(&self, _conversation_id: i64) -> Result<Vec<MessageRow>, DbError> {
        Ok(Vec::new())
    }
}

// ── Supporting types ────────────────────────────────────────────────

/// Sort direction for message listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_sql(&self) -> &'static str {
        match self {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        }
    }
}

/// Filters for paginated conversation listing.
#[derive(Debug, Clone, Default)]
pub struct ConversationFilters {
    /// Cursor: the ID of the last conversation from the previous page.
    pub cursor: Option<i64>,
    /// Max items per page (default 20).
    pub limit: u32,
    /// Filter by conversation source.
    pub source: Option<String>,
    /// Filter by `cron_job_id` column.
    pub cron_job_id: Option<String>,
    /// Filter by pinned status.
    pub pinned: Option<bool>,
    /// Exclude companion companion (work-partner) sessions — rows whose
    /// `extra.companionSession` is `1`. Used by the companion's own conversation
    /// listing/count so its single companion thread does not inflate the
    /// "how many conversations" total. Default `false` (companion rows
    /// returned, matching the normal `/api/conversations` behavior).
    pub exclude_companion_companion: bool,
}

impl ConversationFilters {
    pub fn effective_limit(&self) -> u32 {
        if self.limit == 0 { 20 } else { self.limit }
    }
}

/// Partial update payload for a conversation row.
///
/// `None` = keep existing value; `Some(v)` = set to `v`.
#[derive(Debug, Clone, Default)]
pub struct ConversationRowUpdate {
    pub name: Option<String>,
    pub pinned: Option<bool>,
    pub pinned_at: Option<Option<TimestampMs>>,
    pub model: Option<Option<String>>,
    pub extra: Option<String>,
    pub status: Option<String>,
    /// Set/clear the owning cron job. `Some(Some(id))` sets, `Some(None)` clears
    /// (used by the cron executor's atomic backfill on `new_conversation`).
    pub cron_job_id: Option<Option<String>>,
    pub updated_at: Option<TimestampMs>,
}

/// Partial update payload for a message row.
#[derive(Debug, Clone, Default)]
pub struct MessageRowUpdate {
    pub content: Option<String>,
    pub status: Option<Option<String>>,
    pub hidden: Option<bool>,
}

/// A single result row from cross-conversation message search.
/// Includes full conversation fields for building nested response.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MessageSearchRow {
    // Message fields
    pub message_id: String,
    #[sqlx(rename = "type")]
    pub r#type: String,
    pub content: String,
    pub created_at: TimestampMs,
    // Conversation fields
    pub conversation_id: i64,
    pub conversation_name: String,
    pub conversation_type: String,
    pub conversation_extra: String,
    pub conversation_model: Option<String>,
    pub conversation_status: Option<String>,
    pub conversation_source: Option<String>,
    pub conversation_channel_chat_id: Option<String>,
    pub conversation_pinned: bool,
    pub conversation_pinned_at: Option<TimestampMs>,
    pub conversation_created_at: TimestampMs,
    pub conversation_updated_at: TimestampMs,
}

use nomifun_common::{KnowledgeBaseId, KnowledgeTreeOperationId, TimestampMs};
use serde_json::Value;

use crate::error::DbError;
use crate::models::KnowledgeTreeOperationRow;

/// Hard upper bound for one recovery/outbox polling page.
pub const MAX_KNOWLEDGE_TREE_OPERATION_PAGE_SIZE: u32 = 512;

/// Stable keyset cursor for journal/outbox scans. `timestamp` is `created_at`
/// for recovery pages and `committed_at` for event pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeTreeOperationPageCursor {
    pub timestamp: TimestampMs,
    pub operation_id: KnowledgeTreeOperationId,
}

/// Immutable intent written before touching the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareKnowledgeTreeOperationParams {
    pub knowledge_base_id: KnowledgeBaseId,
    /// Idempotency key scoped by `knowledge_base_id`.
    pub request_id: String,
    /// Lowercase SHA-256 of the canonical command payload.
    pub fingerprint: String,
    /// Canonical non-empty paths relative to the knowledge-base root.
    pub source_rel_path: String,
    pub destination_rel_path: String,
    /// Physical identity captured before rename when the platform exposes it.
    pub source_fs_identity: Option<String>,
    pub created_at: TimestampMs,
}

/// A replay with the same fingerprint returns the original row. `created`
/// tells the coordinator whether this call inserted the journal intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedKnowledgeTreeOperation {
    pub operation: KnowledgeTreeOperationRow,
    pub created: bool,
}

/// Receipt and event become visible in one SQLite commit. The repository
/// canonicalizes both JSON values before persisting them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitKnowledgeTreeOperationParams {
    pub operation_id: KnowledgeTreeOperationId,
    pub receipt: Value,
    pub event_payload: Value,
    pub committed_at: TimestampMs,
}

/// Durable mutation-journal and transactional-outbox persistence boundary.
///
/// Implementations must make `prepare_operation` idempotent by
/// `(knowledge_base_id, request_id)`, reject reuse with another fingerprint,
/// and write the receipt plus pending outbox event atomically.
#[async_trait::async_trait]
pub trait IKnowledgeTreeOperationRepository: Send + Sync {
    async fn prepare_operation(
        &self,
        params: &PrepareKnowledgeTreeOperationParams,
    ) -> Result<PreparedKnowledgeTreeOperation, DbError>;

    /// Advance `prepared` (or a reconciled `needs_recovery`) after the atomic
    /// filesystem rename is known to have committed. Replays are idempotent.
    async fn mark_filesystem_committed(
        &self,
        operation_id: &KnowledgeTreeOperationId,
        committed_at: TimestampMs,
    ) -> Result<KnowledgeTreeOperationRow, DbError>;

    /// Atomically persist the final receipt and create exactly one pending
    /// outbox event on the same operation row.
    async fn commit_operation(
        &self,
        params: &CommitKnowledgeTreeOperationParams,
    ) -> Result<KnowledgeTreeOperationRow, DbError>;

    /// Mark a non-terminal operation for explicit recovery. A committed
    /// receipt can never be changed back into a recovery row.
    async fn mark_needs_recovery(
        &self,
        operation_id: &KnowledgeTreeOperationId,
        error_message: &str,
        updated_at: TimestampMs,
    ) -> Result<KnowledgeTreeOperationRow, DbError>;

    async fn load_by_request(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        request_id: &str,
    ) -> Result<Option<KnowledgeTreeOperationRow>, DbError>;

    async fn load_by_operation(
        &self,
        operation_id: &KnowledgeTreeOperationId,
    ) -> Result<Option<KnowledgeTreeOperationRow>, DbError>;

    /// List every non-terminal row, including `prepared`: the process may
    /// have crashed after the filesystem effect but before its marker write.
    async fn list_pending_recovery_after(
        &self,
        limit: u32,
        after: Option<&KnowledgeTreeOperationPageCursor>,
    ) -> Result<Vec<KnowledgeTreeOperationRow>, DbError>;

    async fn list_pending_recovery(
        &self,
        limit: u32,
    ) -> Result<Vec<KnowledgeTreeOperationRow>, DbError> {
        self.list_pending_recovery_after(limit, None).await
    }

    async fn list_pending_events_after(
        &self,
        limit: u32,
        after: Option<&KnowledgeTreeOperationPageCursor>,
    ) -> Result<Vec<KnowledgeTreeOperationRow>, DbError>;

    async fn list_pending_events(
        &self,
        limit: u32,
    ) -> Result<Vec<KnowledgeTreeOperationRow>, DbError> {
        self.list_pending_events_after(limit, None).await
    }

    /// Idempotently acknowledge publication of the operation's sole event.
    async fn mark_event_published(
        &self,
        operation_id: &KnowledgeTreeOperationId,
        published_at: TimestampMs,
    ) -> Result<KnowledgeTreeOperationRow, DbError>;
}

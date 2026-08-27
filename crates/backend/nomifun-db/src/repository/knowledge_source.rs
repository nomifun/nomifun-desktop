use nomifun_common::{
    KnowledgeBaseId, KnowledgeEntryId, KnowledgeSourceId, KnowledgeSourceItemId, TimestampMs,
};

use crate::error::DbError;
use crate::models::{
    KnowledgeEntryProvenanceRow, KnowledgeSourceItemRow, KnowledgeSourceItemSyncStatus,
    KnowledgeSourceKind, KnowledgeSourceMode, KnowledgeSourceRow, KnowledgeSourceState,
};

/// Idempotent source-aggregate creation. At most one non-removed source of one
/// kind exists per knowledge base; an existing row is returned without being
/// overwritten by stale caller configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureKnowledgeSourceParams {
    pub knowledge_source_id: KnowledgeSourceId,
    pub knowledge_base_id: KnowledgeBaseId,
    pub kind: KnowledgeSourceKind,
    pub mode: KnowledgeSourceMode,
    pub default_parent_entry_id: Option<KnowledgeEntryId>,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredKnowledgeSource {
    pub source: KnowledgeSourceRow,
    pub created: bool,
}

/// Complete CAS update of source configuration/lifecycle. Source kind and base
/// ownership are immutable; every successful mutation increments `revision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateKnowledgeSourceParams {
    pub knowledge_source_id: KnowledgeSourceId,
    pub expected_revision: i64,
    pub mode: KnowledgeSourceMode,
    pub state: KnowledgeSourceState,
    pub default_parent_entry_id: Option<KnowledgeEntryId>,
    pub removed_at: Option<TimestampMs>,
    pub updated_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateKnowledgeSourceItemParams {
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub knowledge_source_id: KnowledgeSourceId,
    pub requested_url: String,
    pub normalized_url: String,
    pub final_url: Option<String>,
    pub rendered: bool,
    pub title: Option<String>,
    pub ordinal: i64,
    pub state: KnowledgeSourceState,
    pub sync_status: KnowledgeSourceItemSyncStatus,
    pub etag: Option<String>,
    pub http_last_modified: Option<String>,
    pub last_attempt_at: Option<TimestampMs>,
    pub last_success_at: Option<TimestampMs>,
    pub last_error: Option<String>,
    pub last_published_hash: Option<String>,
    pub pending_published_hash: Option<String>,
    pub pending_final_url: Option<String>,
    pub pending_title: Option<String>,
    pub pending_publication_at: Option<TimestampMs>,
    pub removed_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
}

/// Complete mutable item snapshot used for URL/title/order/lifecycle edits.
/// Source ownership is immutable and every successful update bumps revision.
/// Synchronization fields must echo the current row unchanged; only the
/// dedicated attempt/success/failure methods may advance sync history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateKnowledgeSourceItemParams {
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub expected_revision: i64,
    pub requested_url: String,
    pub normalized_url: String,
    pub final_url: Option<String>,
    pub rendered: bool,
    pub title: Option<String>,
    pub ordinal: i64,
    pub state: KnowledgeSourceState,
    pub sync_status: KnowledgeSourceItemSyncStatus,
    pub etag: Option<String>,
    pub http_last_modified: Option<String>,
    pub last_attempt_at: Option<TimestampMs>,
    pub last_success_at: Option<TimestampMs>,
    pub last_error: Option<String>,
    pub last_published_hash: Option<String>,
    pub pending_published_hash: Option<String>,
    pub pending_final_url: Option<String>,
    pub pending_title: Option<String>,
    pub pending_publication_at: Option<TimestampMs>,
    pub removed_at: Option<TimestampMs>,
    pub updated_at: TimestampMs,
}

/// Durable filesystem-publication intent. The repository records this after a
/// fetch is prepared and before the managed file is replaced; crash recovery
/// can then compare the current file hash with `pending_published_hash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageKnowledgeSourcePublicationParams {
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub expected_revision: i64,
    pub pending_published_hash: String,
    pub pending_final_url: Option<String>,
    pub pending_title: Option<String>,
    pub staged_at: TimestampMs,
}

/// Successful source publication. The caller supplies the exact content hash
/// that reached the filesystem; future refreshes use it for external-edit CAS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKnowledgeSourceSyncSuccessParams {
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub expected_revision: i64,
    pub final_url: Option<String>,
    pub title: Option<String>,
    pub etag: Option<String>,
    pub http_last_modified: Option<String>,
    pub last_published_hash: String,
    pub succeeded_at: TimestampMs,
}

/// Failed, conflicted, or missing terminal result for one attempt. `status`
/// rejects every non-failure value at the repository boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKnowledgeSourceSyncFailureParams {
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub expected_revision: i64,
    pub status: KnowledgeSourceItemSyncStatus,
    pub error: String,
    pub failed_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindManagedKnowledgeEntryParams {
    pub knowledge_entry_id: KnowledgeEntryId,
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub created_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordKnowledgeEntryCopyParams {
    pub knowledge_entry_id: KnowledgeEntryId,
    pub knowledge_source_item_id: KnowledgeSourceItemId,
    pub derived_from_entry_id: KnowledgeEntryId,
    pub created_at: TimestampMs,
}

/// Persistence boundary for normalized knowledge sources and stable entry
/// provenance. Source and item state is authoritative; paths remain exclusively
/// in the rebuildable knowledge-entry projection.
#[async_trait::async_trait]
pub trait IKnowledgeSourceRepository: Send + Sync {
    async fn ensure_source(
        &self,
        params: &EnsureKnowledgeSourceParams,
    ) -> Result<EnsuredKnowledgeSource, DbError>;

    async fn get_source(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
    ) -> Result<Option<KnowledgeSourceRow>, DbError>;

    async fn list_sources_for_base(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        include_removed: bool,
    ) -> Result<Vec<KnowledgeSourceRow>, DbError>;

    async fn update_source(
        &self,
        params: &UpdateKnowledgeSourceParams,
    ) -> Result<KnowledgeSourceRow, DbError>;

    async fn create_source_item(
        &self,
        params: &CreateKnowledgeSourceItemParams,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    async fn get_source_item(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
    ) -> Result<Option<KnowledgeSourceItemRow>, DbError>;

    async fn get_live_source_item_by_url(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
        normalized_url: &str,
    ) -> Result<Option<KnowledgeSourceItemRow>, DbError>;

    async fn list_source_items(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
        include_removed: bool,
    ) -> Result<Vec<KnowledgeSourceItemRow>, DbError>;

    async fn update_source_item(
        &self,
        params: &UpdateKnowledgeSourceItemParams,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    async fn record_sync_attempt(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
        expected_revision: i64,
        attempted_at: TimestampMs,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    /// Persist publication intent while the item is active + syncing. Every
    /// successful stage bumps the item revision; callers must use the returned
    /// revision for success/failure settlement.
    async fn stage_sync_publication(
        &self,
        params: &StageKnowledgeSourcePublicationParams,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    async fn record_sync_success(
        &self,
        params: &RecordKnowledgeSourceSyncSuccessParams,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    async fn record_sync_failure(
        &self,
        params: &RecordKnowledgeSourceSyncFailureParams,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    /// Tombstone an item after its active managed binding has been detached or
    /// deleted. Rejecting a remaining managed relationship prevents refresh
    /// configuration from disappearing while its document can still resurrect.
    async fn remove_source_item(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
        expected_revision: i64,
        removed_at: TimestampMs,
    ) -> Result<KnowledgeSourceItemRow, DbError>;

    /// Idempotently bind one stable entry as the source-controlled primary
    /// document. A source item may have at most one managed entry.
    async fn bind_managed_entry(
        &self,
        params: &BindManagedKnowledgeEntryParams,
    ) -> Result<KnowledgeEntryProvenanceRow, DbError>;

    async fn detach_managed_entry(
        &self,
        knowledge_entry_id: &KnowledgeEntryId,
        expected_revision: i64,
        detached_at: TimestampMs,
    ) -> Result<KnowledgeEntryProvenanceRow, DbError>;

    /// Atomically detach a managed document and tombstone its source item.
    /// Replays of the same completed transition, and completion from an already
    /// detached/paused pair, return the durable terminal rows.
    async fn remove_managed_source_item(
        &self,
        knowledge_entry_id: &KnowledgeEntryId,
        expected_provenance_revision: i64,
        expected_item_revision: i64,
        removed_at: TimestampMs,
    ) -> Result<(KnowledgeEntryProvenanceRow, KnowledgeSourceItemRow), DbError>;

    /// Idempotently record an independently editable copy while retaining the
    /// source item and original managed relationship.
    async fn record_entry_copy(
        &self,
        params: &RecordKnowledgeEntryCopyParams,
    ) -> Result<KnowledgeEntryProvenanceRow, DbError>;

    async fn get_entry_provenance(
        &self,
        knowledge_entry_id: &KnowledgeEntryId,
    ) -> Result<Option<KnowledgeEntryProvenanceRow>, DbError>;

    async fn get_managed_entry_provenance(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
    ) -> Result<Option<KnowledgeEntryProvenanceRow>, DbError>;

    async fn list_entry_provenance_for_source(
        &self,
        knowledge_source_id: &KnowledgeSourceId,
    ) -> Result<Vec<KnowledgeEntryProvenanceRow>, DbError>;

    async fn list_entry_provenance_for_item(
        &self,
        knowledge_source_item_id: &KnowledgeSourceItemId,
    ) -> Result<Vec<KnowledgeEntryProvenanceRow>, DbError>;
}

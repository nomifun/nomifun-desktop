use nomifun_common::{KnowledgeBaseId, KnowledgeEntryId, TimestampMs};

use crate::error::DbError;
use crate::models::KnowledgeEntryRow;

/// Complete persisted shape supplied by a filesystem scan or a single-entry
/// reconciliation. The projection owns no document content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertKnowledgeEntryParams {
    pub knowledge_entry_id: KnowledgeEntryId,
    pub knowledge_base_id: KnowledgeBaseId,
    pub parent_entry_id: Option<KnowledgeEntryId>,
    pub name: String,
    pub kind: String,
    pub origin: String,
    pub rel_path: String,
    /// Caller-normalized collision key for `rel_path` (portable case and
    /// Unicode policy). SQLite stores and indexes this value opaquely.
    pub portable_rel_path: String,
    pub fs_identity: Option<String>,
    pub content_hash: Option<String>,
    pub revision: i64,
    pub deleted_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<&KnowledgeEntryRow> for UpsertKnowledgeEntryParams {
    fn from(row: &KnowledgeEntryRow) -> Self {
        Self {
            knowledge_entry_id: row.knowledge_entry_id.clone(),
            knowledge_base_id: row.knowledge_base_id.clone(),
            parent_entry_id: row.parent_entry_id.clone(),
            name: row.name.clone(),
            kind: row.kind.clone(),
            origin: row.origin.clone(),
            rel_path: row.rel_path.clone(),
            portable_rel_path: row.portable_rel_path.clone(),
            fs_identity: row.fs_identity.clone(),
            content_hash: row.content_hash.clone(),
            revision: row.revision,
            deleted_at: row.deleted_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Compare-and-swap metadata change after the corresponding filesystem rename
/// has committed (or while a higher-level mutation journal is coordinating
/// it). Descendant paths are rewritten atomically when the entry is a
/// directory; descendant identities and parent links are preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocateKnowledgeEntryProjectionParams {
    pub knowledge_base_id: KnowledgeBaseId,
    pub knowledge_entry_id: KnowledgeEntryId,
    pub destination_parent_entry_id: Option<KnowledgeEntryId>,
    pub new_name: String,
    pub new_rel_path: String,
    pub new_portable_rel_path: String,
    pub expected_revision: i64,
    pub updated_at: TimestampMs,
}

/// Result of one logical projection mutation. `tree_revision` changes once
/// per command, while every affected entry receives its own revision bump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeEntryMutation {
    pub entry: KnowledgeEntryRow,
    pub affected_entries: u64,
    pub tree_revision: i64,
}

/// Result of atomically replacing a knowledge base's complete entry
/// projection from a filesystem reconciliation snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnowledgeProjectionReplacement {
    pub replaced_entries: u64,
    pub tree_revision: i64,
}

/// Persistence boundary for the rebuildable knowledge-entry identity
/// projection. Kept separate from `IKnowledgeRepository` so lightweight test
/// repositories for base registration do not need to emulate filesystem
/// reconciliation semantics.
#[async_trait::async_trait]
pub trait IKnowledgeEntryRepository: Send + Sync {
    async fn get_entry(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        knowledge_entry_id: &KnowledgeEntryId,
    ) -> Result<Option<KnowledgeEntryRow>, DbError>;

    /// Resolve a live entry by the caller-normalized portable relative path.
    async fn get_entry_by_path(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        portable_rel_path: &str,
    ) -> Result<Option<KnowledgeEntryRow>, DbError>;

    /// List the exact persisted projection in deterministic path/identity
    /// order. Tombstones are excluded unless explicitly requested.
    async fn list_entries_for_base(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        include_deleted: bool,
    ) -> Result<Vec<KnowledgeEntryRow>, DbError>;

    async fn tree_revision(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
    ) -> Result<i64, DbError>;

    /// Reconcile one complete entry row and increment the base tree revision.
    async fn upsert_entry(
        &self,
        params: &UpsertKnowledgeEntryParams,
    ) -> Result<KnowledgeEntryMutation, DbError>;

    /// Replace all rows for one base in a single transaction. When supplied,
    /// `expected_tree_revision` makes the rebuild a CAS operation so a stale
    /// scan cannot overwrite a concurrent mutation.
    async fn replace_projection(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        expected_tree_revision: Option<i64>,
        entries: &[UpsertKnowledgeEntryParams],
    ) -> Result<KnowledgeProjectionReplacement, DbError>;

    /// Atomically relocate/rename an entry projection and rewrite every live
    /// descendant path prefix. The target entry revision is checked first.
    async fn relocate_entry(
        &self,
        params: &RelocateKnowledgeEntryProjectionParams,
    ) -> Result<KnowledgeEntryMutation, DbError>;

    /// Soft-delete a live entry and all live descendants as one CAS mutation.
    async fn soft_delete_entry_subtree(
        &self,
        knowledge_base_id: &KnowledgeBaseId,
        knowledge_entry_id: &KnowledgeEntryId,
        expected_revision: i64,
        deleted_at: TimestampMs,
    ) -> Result<KnowledgeEntryMutation, DbError>;
}

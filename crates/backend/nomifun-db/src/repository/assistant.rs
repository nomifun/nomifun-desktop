//! Repository traits for the assistants and assistant_overrides tables.

use crate::error::DbError;
use crate::models::{
    AssistantOverrideRow, AssistantRow, AssistantTagRow, CreateAssistantParams, CreateAssistantTagParams,
    UpdateAssistantParams, UpdateAssistantTagParams, UpsertOverrideParams,
};

/// CRUD access for user-authored assistant rows.
///
/// Object-safe via `async_trait` to support `Arc<dyn IAssistantRepository>`.
#[async_trait::async_trait]
pub trait IAssistantRepository: Send + Sync {
    /// Return all user-authored assistants, ordered by `updated_at` descending.
    async fn list(&self) -> Result<Vec<AssistantRow>, DbError>;

    /// Look up a single assistant by id.
    async fn get(&self, id: &str) -> Result<Option<AssistantRow>, DbError>;

    /// Insert a new assistant row. Primary-key conflict surfaces as
    /// `DbError::Conflict`.
    async fn create(&self, params: &CreateAssistantParams<'_>) -> Result<AssistantRow, DbError>;

    /// Partial update of an existing assistant row. Returns `Ok(None)` if
    /// no row matches.
    async fn update(&self, id: &str, params: &UpdateAssistantParams<'_>) -> Result<Option<AssistantRow>, DbError>;

    /// Delete an assistant row by id. Returns `true` if a row was removed.
    async fn delete(&self, id: &str) -> Result<bool, DbError>;

    /// Insert or replace by id. Exists for callers outside of the
    /// migration/import path; the import endpoint must use `create` and
    /// skip on conflict per spec §6.3.
    async fn upsert(&self, params: &CreateAssistantParams<'_>) -> Result<AssistantRow, DbError>;
}

/// Per-assistant user state (enabled flag, sort order, last-used timestamp).
#[async_trait::async_trait]
pub trait IAssistantOverrideRepository: Send + Sync {
    /// Fetch the override row for a given assistant id, if any.
    async fn get(&self, assistant_id: &str) -> Result<Option<AssistantOverrideRow>, DbError>;

    /// Fetch all override rows.
    async fn get_all(&self) -> Result<Vec<AssistantOverrideRow>, DbError>;

    /// Insert or update the override row for an assistant.
    async fn upsert(&self, params: &UpsertOverrideParams<'_>) -> Result<AssistantOverrideRow, DbError>;

    /// Delete the override row for an assistant. Returns `true` if a row was
    /// removed.
    async fn delete(&self, assistant_id: &str) -> Result<bool, DbError>;

    /// Remove override rows whose `assistant_id` is not in `valid_ids`.
    /// Returns the number of rows deleted.
    async fn delete_orphans(&self, valid_ids: &[&str]) -> Result<u64, DbError>;
}

/// CRUD for the user-created assistant tag vocabulary.
#[async_trait::async_trait]
pub trait IAssistantTagRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<AssistantTagRow>, DbError>;
    async fn get(&self, key: &str) -> Result<Option<AssistantTagRow>, DbError>;
    async fn create(&self, params: &CreateAssistantTagParams<'_>) -> Result<AssistantTagRow, DbError>;
    async fn update(&self, key: &str, params: &UpdateAssistantTagParams<'_>) -> Result<Option<AssistantTagRow>, DbError>;
    async fn delete(&self, key: &str) -> Result<bool, DbError>;
}

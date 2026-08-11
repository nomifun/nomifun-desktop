use nomifun_common::MiniAppId;

use crate::error::DbError;
use crate::models::{MiniAppDocumentRow, MiniAppRow};

/// Mini-app data access. Every method takes `user_id` first and filters by it,
/// so a cross-owner id is indistinguishable from NotFound. The single exception
/// is [`IMiniAppRepository::find_by_id_any_owner`], documented at its own site.
#[async_trait::async_trait]
pub trait IMiniAppRepository: Send + Sync {
    /// All apps owned by `user_id`, most recently updated first (ties broken by
    /// insertion order, newest first, so the order is total).
    async fn list(&self, user_id: &str) -> Result<Vec<MiniAppRow>, DbError>;

    /// One app by id, scoped to `user_id`; `None` if absent or owned by another.
    async fn find(&self, user_id: &str, id: &MiniAppId) -> Result<Option<MiniAppRow>, DbError>;

    /// The stored HTML document of any owner's app — the ONE unscoped read.
    ///
    /// Exists solely for the auth-exempt serve route: an iframe subresource load
    /// presents no credentials, so there is no owner to scope by. Safe because it
    /// takes an unguessable bare UUIDv7 and returns the body alone
    /// ([`MiniAppDocumentRow`]) — no name, no owner, no provenance. Every other
    /// read, and every write, stays owner-scoped.
    async fn find_by_id_any_owner(
        &self,
        id: &MiniAppId,
    ) -> Result<Option<MiniAppDocumentRow>, DbError>;

    /// Create an app owned by `user_id`; returns the inserted row.
    async fn create(&self, user_id: &str, params: CreateMiniAppParams<'_>)
        -> Result<MiniAppRow, DbError>;

    /// Update an owned app. `DbError::NotFound` if absent or not owned.
    async fn update(
        &self,
        user_id: &str,
        id: &MiniAppId,
        params: UpdateMiniAppParams<'_>,
    ) -> Result<MiniAppRow, DbError>;

    /// Delete an owned app. `DbError::NotFound` if absent or not owned.
    async fn delete(&self, user_id: &str, id: &MiniAppId) -> Result<(), DbError>;

    /// Record when the stored snapshot became current, without touching
    /// `updated_at` or the body. Used when the working copy is materialized from
    /// the snapshot: the two are byte-identical at that instant, so the app has
    /// no unpublished changes and the timestamp must say so.
    async fn mark_published_at(
        &self,
        user_id: &str,
        id: &MiniAppId,
        published_at: i64,
    ) -> Result<MiniAppRow, DbError>;
}

/// Parameters for creating an app. `html` is the complete self-contained
/// document; the repository stores it verbatim.
#[derive(Debug, Default)]
pub struct CreateMiniAppParams<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub icon: Option<&'a str>,
    pub html: &'a str,
    pub source_conversation_id: Option<&'a str>,
}

/// Parameters for updating an app. `None` fields are left unchanged; `icon`
/// wrapped in `Some(None)` explicitly clears the stored icon.
#[derive(Debug, Default)]
pub struct UpdateMiniAppParams<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub icon: Option<Option<&'a str>>,
    pub html: Option<&'a str>,
    /// When the supplied `html` became the published snapshot. Set by the
    /// publish path together with `html` so one statement writes the body, its
    /// size and its publish instant.
    pub published_at: Option<i64>,
}

use nomifun_common::MiniAppId;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{MiniAppDocumentRow, MiniAppRow};
use crate::repository::miniapp::{CreateMiniAppParams, IMiniAppRepository, UpdateMiniAppParams};

/// The metadata projection every owner-scoped read (and every write's
/// `RETURNING`) shares. `html` itself is never listed: `html_size` is a stored
/// column this repository maintains, so a list never reads a single body byte.
const METADATA_COLUMNS: &str = "id, miniapp_id, user_id, name, description, icon, \
     source_conversation_id, html_size, published_at, \
     created_at, updated_at";

/// SQLite-backed [`IMiniAppRepository`]. Every query except the serve-path
/// document read is owner-scoped: the `user_id` predicate makes a cross-owner id
/// return `None`/`NotFound`.
#[derive(Clone, Debug)]
pub struct SqliteMiniAppRepository {
    pool: SqlitePool,
}

impl SqliteMiniAppRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IMiniAppRepository for SqliteMiniAppRepository {
    async fn list(&self, user_id: &str) -> Result<Vec<MiniAppRow>, DbError> {
        let rows = sqlx::query_as::<_, MiniAppRow>(&format!(
            "SELECT {METADATA_COLUMNS} FROM miniapps WHERE user_id = ? \
             ORDER BY updated_at DESC, id DESC"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn find(&self, user_id: &str, id: &MiniAppId) -> Result<Option<MiniAppRow>, DbError> {
        let row = sqlx::query_as::<_, MiniAppRow>(&format!(
            "SELECT {METADATA_COLUMNS} FROM miniapps WHERE user_id = ? AND miniapp_id = ?"
        ))
        .bind(user_id)
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn find_by_id_any_owner(
        &self,
        id: &MiniAppId,
    ) -> Result<Option<MiniAppDocumentRow>, DbError> {
        let row = sqlx::query_as::<_, MiniAppDocumentRow>(
            "SELECT html FROM miniapps WHERE miniapp_id = ?",
        )
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create(
        &self,
        user_id: &str,
        params: CreateMiniAppParams<'_>,
    ) -> Result<MiniAppRow, DbError> {
        let now = nomifun_common::now_ms();
        let miniapp_id = MiniAppId::new();
        let row = sqlx::query_as::<_, MiniAppRow>(&format!(
            "INSERT INTO miniapps \
                (miniapp_id, user_id, name, description, icon, html, html_size, \
                 source_conversation_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING {METADATA_COLUMNS}"
        ))
        .bind(miniapp_id.as_str())
        .bind(user_id)
        .bind(params.name)
        .bind(params.description)
        .bind(params.icon)
        .bind(params.html)
        .bind(params.html.len() as i64)
        .bind(params.source_conversation_id)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    async fn update(
        &self,
        user_id: &str,
        id: &MiniAppId,
        params: UpdateMiniAppParams<'_>,
    ) -> Result<MiniAppRow, DbError> {
        // One owner-scoped statement: `COALESCE` leaves an unsupplied field
        // alone, `CASE WHEN` keeps the icon's "clear vs leave" semantics
        // (`Option<Option<_>>`) unambiguous, and `RETURNING` hands back the
        // metadata row without a second read. The body — and therefore
        // `html_size` — is written only when the caller supplies one, so a
        // rename never rewrites megabytes. A missing or non-owned id matches no
        // row, so `RETURNING` yields nothing: that is the NotFound.
        let row = sqlx::query_as::<_, MiniAppRow>(&format!(
            "UPDATE miniapps SET \
                name = COALESCE(?, name), \
                description = COALESCE(?, description), \
                icon = CASE WHEN ? THEN ? ELSE icon END, \
                html = COALESCE(?, html), \
                html_size = COALESCE(?, html_size), \
                published_at = COALESCE(?, published_at), \
                updated_at = ? \
             WHERE user_id = ? AND miniapp_id = ? \
             RETURNING {METADATA_COLUMNS}"
        ))
        .bind(params.name)
        .bind(params.description)
        .bind(params.icon.is_some())
        .bind(params.icon.flatten())
        .bind(params.html)
        .bind(params.html.map(|html| html.len() as i64))
        .bind(params.published_at)
        .bind(nomifun_common::now_ms())
        .bind(user_id)
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| DbError::NotFound("miniapp".into()))
    }

    async fn delete(&self, user_id: &str, id: &MiniAppId) -> Result<(), DbError> {
        let affected = sqlx::query("DELETE FROM miniapps WHERE user_id = ? AND miniapp_id = ?")
            .bind(user_id)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(DbError::NotFound("miniapp".into()));
        }
        Ok(())
    }

    async fn mark_published_at(
        &self,
        user_id: &str,
        id: &MiniAppId,
        published_at: i64,
    ) -> Result<MiniAppRow, DbError> {
        let row = sqlx::query_as::<_, MiniAppRow>(&format!(
            "UPDATE miniapps SET published_at = ? \
             WHERE user_id = ? AND miniapp_id = ? \
             RETURNING {METADATA_COLUMNS}"
        ))
        .bind(published_at)
        .bind(user_id)
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.ok_or_else(|| DbError::NotFound("miniapp".into()))
    }
}

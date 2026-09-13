//! Owner-scoped, revision-checked documents for the Plugin product workspace.
use crate::{DbError, SqlitePool};

#[derive(Clone)]
pub struct PluginProductDocuments {
    pool: SqlitePool,
}

impl PluginProductDocuments {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn owned_plugin_ids(&self, owner: &str) -> Result<Vec<String>, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT mount.mount_id FROM plugin_mounts mount
             WHERE EXISTS (
                 SELECT 1 FROM plugin_projects project
                 WHERE project.linked_mount_id = mount.mount_id AND project.owner_user_id = ?
             ) OR (
                 NOT EXISTS (SELECT 1 FROM plugin_projects project WHERE project.linked_mount_id = mount.mount_id)
                 AND EXISTS (SELECT 1 FROM installation_identity WHERE singleton_key = 'installation' AND owner_user_id = ?)
             )
             UNION SELECT miniapp_id FROM miniapp_products WHERE owner_user_id = ?",
        )
            .bind(owner).bind(owner).bind(owner).fetch_all(&self.pool).await?)
    }

    pub async fn get(&self, owner: &str, key: &str) -> Result<Option<(i64, String)>, DbError> {
        Ok(sqlx::query_as("SELECT revision, content_json FROM plugin_product_documents WHERE owner_user_id = ? AND document_key = ?")
            .bind(owner).bind(key).fetch_optional(&self.pool).await?)
    }

    pub async fn list(
        &self,
        owner: &str,
        prefix: &str,
    ) -> Result<Vec<(String, i64, String)>, DbError> {
        Ok(sqlx::query_as("SELECT document_key, revision, content_json FROM plugin_product_documents WHERE owner_user_id = ? AND document_key LIKE ? ORDER BY updated_at DESC")
            .bind(owner).bind(format!("{prefix}%")).fetch_all(&self.pool).await?)
    }

    /// `expected = 0` creates; all subsequent changes compare the known revision.
    pub async fn put(
        &self,
        owner: &str,
        key: &str,
        expected: i64,
        json: &str,
    ) -> Result<i64, DbError> {
        if expected < 0 || expected == i64::MAX {
            return Err(DbError::Conflict("Invalid document revision".into()));
        }
        let result = if expected == 0 {
            sqlx::query("INSERT OR IGNORE INTO plugin_product_documents (owner_user_id, document_key, revision, content_json, updated_at) SELECT ?, ?, 1, ?, ? WHERE EXISTS (SELECT 1 FROM users WHERE user_id = ?)")
                .bind(owner).bind(key).bind(json).bind(nomifun_common::now_ms()).bind(owner).execute(&self.pool).await?
        } else {
            sqlx::query("UPDATE plugin_product_documents SET revision = revision + 1, content_json = ?, updated_at = ? WHERE owner_user_id = ? AND document_key = ? AND revision = ?")
                .bind(json).bind(nomifun_common::now_ms()).bind(owner).bind(key).bind(expected).execute(&self.pool).await?
        };
        if result.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "The Plugin document changed. Reload before retrying.".into(),
            ));
        }
        Ok(expected + 1)
    }

    pub async fn delete(&self, owner: &str, key: &str, expected: i64) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM plugin_product_documents WHERE owner_user_id = ? AND document_key = ? AND revision = ?")
            .bind(owner).bind(key).bind(expected).execute(&self.pool).await?;
        if result.rows_affected() != 1 {
            return Err(DbError::Conflict("The Plugin document changed".into()));
        }
        Ok(())
    }
}

/// Shared finalization for every plugin execution role. The caller's existing
/// lifecycle transaction owns both data deletion and workspace cleanup.
pub(crate) async fn remove_plugin_documents(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, owner: &str, plugin_id: &str, now: i64,
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM plugin_product_documents WHERE owner_user_id = ? AND document_key LIKE 'draft:%' AND json_extract(content_json, '$.plugin_id') = ?")
        .bind(owner).bind(plugin_id).execute(&mut **tx).await?;
    let item_path = format!("$.items.\"{plugin_id}\"");
    sqlx::query("UPDATE plugin_product_documents SET content_json = json_set(json_remove(content_json, ?), '$.revision', revision + 1), revision = revision + 1, updated_at = ? WHERE owner_user_id = ? AND document_key = 'library' AND json_type(content_json, ?) IS NOT NULL")
        .bind(&item_path).bind(now).bind(owner).bind(&item_path).execute(&mut **tx).await?;
    Ok(())
}

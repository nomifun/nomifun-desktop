use nomifun_common::ProviderId;
use nomifun_common::now_ms;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{ProviderConnectionRow, UpsertProviderConnectionParams};
use crate::repository::provider_connection::IProviderConnectionRepository;
use crate::repository::sqlite_provider_model_capability::{
    bump_provider_config_revision_tx, clear_health_for_connection_role_tx,
};

/// SQLite-backed implementation of [`IProviderConnectionRepository`].
#[derive(Clone, Debug)]
pub struct SqliteProviderConnectionRepository {
    pool: SqlitePool,
}

impl SqliteProviderConnectionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}
#[async_trait::async_trait]
impl IProviderConnectionRepository for SqliteProviderConnectionRepository {
    async fn list_for_provider(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderConnectionRow>, DbError> {
        let rows = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? ORDER BY role ASC",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get(
        &self,
        provider_id: &str,
        role: &str,
    ) -> Result<Option<ProviderConnectionRow>, DbError> {
        let row = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id)
        .bind(role)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn upsert(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        params: &UpsertProviderConnectionParams<'_>,
    ) -> Result<ProviderConnectionRow, DbError> {
        let role = params.role.trim();
        if role.is_empty() || role == "default" {
            return Err(DbError::Conflict(
                "named provider connection role must be nonblank and cannot be 'default'".into(),
            ));
        }
        let base_url = params.base_url.trim();
        if base_url.is_empty() {
            return Err(DbError::Conflict(
                "named provider connection base_url must not be blank".into(),
            ));
        }
        let auth_scheme = params.auth_scheme.trim();
        if auth_scheme.is_empty() {
            return Err(DbError::Conflict(
                "named provider connection auth_scheme must not be blank".into(),
            ));
        }
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let provider_id = ProviderId::parse(provider_id).map_err(|error| {
            DbError::Conflict(format!(
                "Provider connection provider_id '{provider_id}' is not a canonical UUIDv7: {error}"
            ))
        })?;
        // Parent-existence write lock: a concurrent provider delete cannot
        // race this child write within the transaction.
        let parent = sqlx::query(
            "UPDATE providers SET config_revision = config_revision \
             WHERE provider_id = ? AND config_revision = ?",
        )
        .bind(provider_id.as_str())
        .bind(expected_config_revision)
        .execute(&mut *tx)
        .await?;
        if parent.rows_affected() == 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)")
                    .bind(provider_id.as_str())
                    .fetch_one(&mut *tx)
                    .await?;
            return if exists {
                Err(DbError::Conflict(format!(
                    "provider invocation graph changed while saving connection; expected revision {expected_config_revision}"
                )))
            } else {
                Err(DbError::Conflict(format!(
                    "Provider connection provider '{provider_id}' does not exist"
                )))
            };
        }

        let existing = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id.as_str())
        .bind(role)
        .fetch_optional(&mut *tx)
        .await?;
        let invocation_changed = existing.as_ref().map_or(true, |existing| {
            existing.base_url != base_url
                || existing.auth_scheme != auth_scheme
                || existing.credentials_encrypted != params.credentials_encrypted
                || existing.extra != params.extra
        });

        // connection_id is a bare UUIDv7 minted only on first insert; the
        // conflict arm deliberately leaves connection_id and created_at as-is.
        let connection_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO provider_connections \
                (connection_id, provider_id, role, label, base_url, auth_scheme, \
                 credentials_encrypted, extra, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(provider_id, role) DO UPDATE SET \
                label = excluded.label, \
                base_url = excluded.base_url, \
                auth_scheme = excluded.auth_scheme, \
                credentials_encrypted = excluded.credentials_encrypted, \
                extra = excluded.extra, \
                updated_at = excluded.updated_at",
        )
        .bind(&connection_id)
        .bind(provider_id.as_str())
        .bind(role)
        .bind(params.label)
        .bind(base_url)
        .bind(auth_scheme)
        .bind(params.credentials_encrypted)
        .bind(params.extra)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if invocation_changed {
            clear_health_for_connection_role_tx(&mut tx, provider_id.as_str(), role).await?;
            bump_provider_config_revision_tx(&mut tx, provider_id.as_str()).await?;
        }
        let stored = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id.as_str())
        .bind(role)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stored)
    }

    async fn delete(&self, provider_id: &str, role: &str) -> Result<bool, DbError> {
        if role.trim().is_empty() || role.trim() == "default" {
            return Err(DbError::Conflict(
                "the provider default connection is owned by the provider row".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let provider_id = ProviderId::parse(provider_id).map_err(|error| {
            DbError::Conflict(format!(
                "Provider connection provider_id '{provider_id}' is not a canonical UUIDv7: {error}"
            ))
        })?;

        // Serialize this delete with every ProviderModel create/update (those
        // operations take the same parent-row write lock). The reference check
        // and delete therefore form one atomic decision: a model cannot acquire
        // this role between the check and the DELETE.
        let parent =
            sqlx::query("UPDATE providers SET updated_at = updated_at WHERE provider_id = ?")
                .bind(provider_id.as_str())
                .execute(&mut *tx)
                .await?;
        if parent.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Provider connection provider '{provider_id}' does not exist"
            )));
        }

        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM provider_connections WHERE provider_id = ? AND role = ?)",
        )
        .bind(provider_id.as_str())
        .bind(role)
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            tx.commit().await?;
            return Ok(false);
        }

        let referenced_by = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT model FROM provider_model_capabilities \
             WHERE provider_id = ? AND connection_role = ? ORDER BY model ASC",
        )
        .bind(provider_id.as_str())
        .bind(role)
        .fetch_all(&mut *tx)
        .await?;
        if !referenced_by.is_empty() {
            return Err(DbError::Conflict(format!(
                "Cannot delete provider connection role {role:?}: still referenced by model(s): {}",
                referenced_by.join(", ")
            )));
        }

        let result =
            sqlx::query("DELETE FROM provider_connections WHERE provider_id = ? AND role = ?")
                .bind(provider_id.as_str())
                .bind(role)
                .execute(&mut *tx)
                .await?;
        if result.rows_affected() > 0 {
            bump_provider_config_revision_tx(&mut tx, provider_id.as_str()).await?;
        }
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;
    use crate::models::{NewProviderModel, NewProviderModelCapability};
    use crate::repository::provider::{CreateProviderParams, IProviderRepository};
    use crate::repository::sqlite_provider::SqliteProviderRepository;

    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";

    static CHAT: [NewProviderModelCapability<'static>; 1] = [NewProviderModelCapability {
        task: "chat",
        traits: "[]",
        protocol: "openai.chat_text",
        connection_role: "default",
        base_url_override: None,
        endpoint: None,
        poll_endpoint: None,
        content_endpoint: None,
        realtime_endpoint: None,
        allow_cross_origin_credentials: false,
        provider_params: "{}",
        context_limit: None,
        output_limit: None,
    }];

    async fn seed(pool: &SqlitePool) {
        SqliteProviderRepository::new(pool.clone())
            .create(
                CreateProviderParams {
                    provider_id: Some(PROVIDER_ID),
                    platform: "openai",
                    name: "OpenAI",
                    base_url: "https://api.openai.com/v1",
                    auth_scheme: "bearer",
                    credentials_encrypted: "cipher",
                    enabled: true,
                    bedrock_config: None,
                    sort_order: None,
                },
                &NewProviderModel {
                    model: "gpt-5",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &CHAT,
                },
                &[],
            )
            .await
            .unwrap();
    }

    fn connection<'a>(credentials: &'a str) -> UpsertProviderConnectionParams<'a> {
        UpsertProviderConnectionParams {
            role: "voice",
            label: Some("Voice"),
            base_url: "https://voice.example/v1",
            auth_scheme: "token",
            credentials_encrypted: credentials,
            extra: "{}",
        }
    }

    #[tokio::test]
    async fn upsert_keeps_stable_id_and_delete_succeeds_when_unreferenced() {
        let db = init_database_memory().await.unwrap();
        seed(db.pool()).await;
        let repo = SqliteProviderConnectionRepository::new(db.pool().clone());
        let first = repo
            .upsert(PROVIDER_ID, 0, &connection("one"))
            .await
            .unwrap();
        let second = repo
            .upsert(PROVIDER_ID, 1, &connection("two"))
            .await
            .unwrap();
        assert_eq!(first.connection_id, second.connection_id);
        assert_eq!(second.auth_scheme, "token");
        assert!(repo.delete(PROVIDER_ID, "voice").await.unwrap());
        let revision: i64 =
            sqlx::query_scalar("SELECT config_revision FROM providers WHERE provider_id = ?")
                .bind(PROVIDER_ID)
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(revision, 3);
    }

    #[tokio::test]
    async fn delete_rejects_role_referenced_by_capability() {
        let db = init_database_memory().await.unwrap();
        seed(db.pool()).await;
        let repo = SqliteProviderConnectionRepository::new(db.pool().clone());
        repo.upsert(PROVIDER_ID, 0, &connection("one"))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE provider_model_capabilities SET connection_role='voice' \
             WHERE provider_id=? AND model='gpt-5' AND task='chat'",
        )
        .bind(PROVIDER_ID)
        .execute(db.pool())
        .await
        .unwrap();
        let error = repo.delete(PROVIDER_ID, "voice").await.unwrap_err();
        assert!(matches!(error, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn default_role_is_not_a_named_connection() {
        let db = init_database_memory().await.unwrap();
        seed(db.pool()).await;
        let repo = SqliteProviderConnectionRepository::new(db.pool().clone());
        let invalid = UpsertProviderConnectionParams {
            role: "default",
            ..connection("one")
        };
        assert!(matches!(
            repo.upsert(PROVIDER_ID, 0, &invalid).await.unwrap_err(),
            DbError::Conflict(_)
        ));
        assert!(matches!(
            repo.delete(PROVIDER_ID, "default").await.unwrap_err(),
            DbError::Conflict(_)
        ));
    }
}

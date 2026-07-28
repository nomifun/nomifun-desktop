use nomifun_common::now_ms;
use nomifun_common::ProviderId;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{ProviderConnectionRow, UpsertProviderConnectionParams};
use crate::repository::provider_connection::IProviderConnectionRepository;

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
    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderConnectionRow>, DbError> {
        let rows = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? ORDER BY role ASC",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get(&self, provider_id: &str, role: &str) -> Result<Option<ProviderConnectionRow>, DbError> {
        let row = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id)
        .bind(role)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn upsert(&self, provider_id: &str, params: &UpsertProviderConnectionParams<'_>) -> Result<ProviderConnectionRow, DbError> {
        let now = now_ms();
        let mut tx = self.pool.begin().await?;
        let provider_id = ProviderId::parse(provider_id).map_err(|error| {
            DbError::Conflict(format!(
                "Provider connection provider_id '{provider_id}' is not a canonical UUIDv7: {error}"
            ))
        })?;
        // Parent-existence write lock: a concurrent provider delete cannot
        // race this child write within the transaction.
        let parent = sqlx::query("UPDATE providers SET updated_at = updated_at WHERE provider_id = ?")
            .bind(provider_id.as_str())
            .execute(&mut *tx)
            .await?;
        if parent.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Provider connection provider '{provider_id}' does not exist"
            )));
        }

        // connection_id is a bare UUIDv7 minted only on first insert; the
        // conflict arm deliberately leaves connection_id and created_at as-is.
        let connection_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO provider_connections \
                (connection_id, provider_id, role, label, base_url, auth_scheme, \
                 credentials_encrypted, is_full_url, extra, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(provider_id, role) DO UPDATE SET \
                label = excluded.label, \
                base_url = excluded.base_url, \
                auth_scheme = excluded.auth_scheme, \
                credentials_encrypted = excluded.credentials_encrypted, \
                is_full_url = excluded.is_full_url, \
                extra = excluded.extra, \
                updated_at = excluded.updated_at",
        )
        .bind(&connection_id)
        .bind(provider_id.as_str())
        .bind(params.role)
        .bind(params.label)
        .bind(params.base_url)
        .bind(params.auth_scheme)
        .bind(params.credentials_encrypted)
        .bind(params.is_full_url)
        .bind(params.extra)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let stored = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id.as_str())
        .bind(params.role)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(stored)
    }

    async fn delete(&self, provider_id: &str, role: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM provider_connections WHERE provider_id = ? AND role = ?")
            .bind(provider_id)
            .bind(role)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;
    use crate::repository::provider::{CreateProviderParams, IProviderRepository};
    use crate::repository::sqlite_provider::SqliteProviderRepository;

    const PROVIDER_1: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const PROVIDER_2: &str = "0190f5fe-7c00-7a00-8abc-012345678902";

    async fn seed_provider(pool: &SqlitePool, provider_id: &str) {
        SqliteProviderRepository::new(pool.clone())
            .create(CreateProviderParams {
                provider_id: Some(provider_id),
                platform: "openai",
                name: provider_id,
                base_url: "https://x.test/v1",
                api_key_encrypted: "enc",
                models: "[]",
                enabled: true,
                capabilities: "[]",
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
    }

    fn voice_params<'a>() -> UpsertProviderConnectionParams<'a> {
        UpsertProviderConnectionParams {
            role: "voice",
            label: Some("Voice endpoint"),
            base_url: "https://voice.x.test/v1",
            auth_scheme: "bearer",
            credentials_encrypted: "enc-voice",
            is_full_url: false,
            extra: "{}",
        }
    }

    #[tokio::test]
    async fn upsert_same_role_keeps_connection_id_and_updates_fields() {
        let db = init_database_memory().await.unwrap();
        seed_provider(db.pool(), PROVIDER_1).await;
        let r = SqliteProviderConnectionRepository::new(db.pool().clone());

        let first = r.upsert(PROVIDER_1, &voice_params()).await.unwrap();
        assert_eq!(first.provider_id, PROVIDER_1);
        assert_eq!(first.role, "voice");
        assert_eq!(first.label.as_deref(), Some("Voice endpoint"));
        nomifun_common::validate_uuidv7(&first.connection_id).unwrap();

        let second = r
            .upsert(
                PROVIDER_1,
                &UpsertProviderConnectionParams {
                    role: "voice",
                    label: None,
                    base_url: "https://voice2.x.test",
                    auth_scheme: "api_key",
                    credentials_encrypted: "enc-voice-2",
                    is_full_url: true,
                    extra: r#"{"region":"eu"}"#,
                },
            )
            .await
            .unwrap();
        assert_eq!(second.connection_id, first.connection_id, "upsert must not remint connection_id");
        assert_eq!(second.id, first.id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.label, None);
        assert_eq!(second.base_url, "https://voice2.x.test");
        assert_eq!(second.auth_scheme, "api_key");
        assert_eq!(second.credentials_encrypted, "enc-voice-2");
        assert!(second.is_full_url);
        assert_eq!(second.extra, r#"{"region":"eu"}"#);

        // Still exactly one row for the (provider, role) pair.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_connections")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn list_get_delete_roundtrip() {
        let db = init_database_memory().await.unwrap();
        seed_provider(db.pool(), PROVIDER_1).await;
        seed_provider(db.pool(), PROVIDER_2).await;
        let r = SqliteProviderConnectionRepository::new(db.pool().clone());

        r.upsert(PROVIDER_1, &voice_params()).await.unwrap();
        r.upsert(
            PROVIDER_1,
            &UpsertProviderConnectionParams {
                role: "image",
                label: None,
                base_url: "https://img.x.test/v1",
                auth_scheme: "bearer",
                credentials_encrypted: "enc-img",
                is_full_url: false,
                extra: "{}",
            },
        )
        .await
        .unwrap();
        r.upsert(PROVIDER_2, &voice_params()).await.unwrap();

        let scoped = r.list_for_provider(PROVIDER_1).await.unwrap();
        assert_eq!(
            scoped.iter().map(|c| c.role.as_str()).collect::<Vec<_>>(),
            ["image", "voice"],
        );

        let got = r.get(PROVIDER_1, "voice").await.unwrap().unwrap();
        assert_eq!(got.credentials_encrypted, "enc-voice");
        assert!(r.get(PROVIDER_1, "missing").await.unwrap().is_none());

        assert!(r.delete(PROVIDER_1, "voice").await.unwrap());
        assert!(!r.delete(PROVIDER_1, "voice").await.unwrap(), "delete is idempotent");
        assert!(r.get(PROVIDER_1, "voice").await.unwrap().is_none());
        assert_eq!(r.list_for_provider(PROVIDER_2).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unknown_provider_is_conflict() {
        let db = init_database_memory().await.unwrap();
        let r = SqliteProviderConnectionRepository::new(db.pool().clone());
        let err = r.upsert(PROVIDER_1, &voice_params()).await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_connections")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

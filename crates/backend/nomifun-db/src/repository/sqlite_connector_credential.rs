use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::ConnectorCredentialRow;
use crate::repository::IConnectorCredentialRepository;

/// SQLite-backed [`IConnectorCredentialRepository`].
#[derive(Clone, Debug)]
pub struct SqliteConnectorCredentialRepository {
    pool: SqlitePool,
}

impl SqliteConnectorCredentialRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IConnectorCredentialRepository for SqliteConnectorCredentialRepository {
    async fn list(&self) -> Result<Vec<ConnectorCredentialRow>, DbError> {
        let rows = sqlx::query_as::<_, ConnectorCredentialRow>(
            "SELECT * FROM connector_credentials ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn get(&self, id: &str) -> Result<Option<ConnectorCredentialRow>, DbError> {
        let row = sqlx::query_as::<_, ConnectorCredentialRow>("SELECT * FROM connector_credentials WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn create(&self, kind: &str, name: &str, payload_encrypted: &str) -> Result<ConnectorCredentialRow, DbError> {
        let id = nomifun_common::generate_prefixed_id("conn");
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO connector_credentials (id, kind, name, payload_encrypted, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kind)
        .bind(name)
        .bind(payload_encrypted)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(ConnectorCredentialRow {
            id,
            kind: kind.to_owned(),
            name: name.to_owned(),
            payload_encrypted: payload_encrypted.to_owned(),
            created_at: now,
            updated_at: now,
        })
    }

    async fn delete(&self, id: &str) -> Result<(), DbError> {
        let res = sqlx::query("DELETE FROM connector_credentials WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Err(DbError::NotFound(id.to_owned()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    #[tokio::test]
    async fn connector_credential_crud_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteConnectorCredentialRepository::new(db.pool().clone());

        let row = repo.create("feishu", "我的飞书", "ENC(payload)").await.unwrap();
        assert!(row.id.starts_with("conn"), "id prefixed: {}", row.id);

        let got = repo.get(&row.id).await.unwrap().unwrap();
        assert_eq!(got.kind, "feishu");
        assert_eq!(got.name, "我的飞书");
        assert_eq!(got.payload_encrypted, "ENC(payload)");

        // A second credential of the same kind is allowed (different tenant).
        repo.create("feishu", "另一个飞书", "ENC(other)").await.unwrap();
        assert_eq!(repo.list().await.unwrap().len(), 2);

        repo.delete(&row.id).await.unwrap();
        assert!(repo.get(&row.id).await.unwrap().is_none());
        assert!(matches!(repo.delete(&row.id).await, Err(DbError::NotFound(_))), "second delete errors");
    }
}

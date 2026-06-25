use nomifun_common::TimestampMs;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::RemoteAgentRow;
use crate::repository::remote_agent::{CreateRemoteAgentParams, IRemoteAgentRepository, UpdateRemoteAgentParams};

/// SQLite-backed implementation of [`IRemoteAgentRepository`].
#[derive(Clone, Debug)]
pub struct SqliteRemoteAgentRepository {
    pool: SqlitePool,
}

impl SqliteRemoteAgentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IRemoteAgentRepository for SqliteRemoteAgentRepository {
    async fn list(&self) -> Result<Vec<RemoteAgentRow>, DbError> {
        let rows = sqlx::query_as::<_, RemoteAgentRow>("SELECT * FROM remote_agents ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    async fn find_by_id(&self, id: i64) -> Result<Option<RemoteAgentRow>, DbError> {
        let row = sqlx::query_as::<_, RemoteAgentRow>("SELECT * FROM remote_agents WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row)
    }

    async fn create(&self, params: CreateRemoteAgentParams<'_>) -> Result<RemoteAgentRow, DbError> {
        let now = nomifun_common::now_ms();
        let status = "unknown";

        let result = sqlx::query(
            "INSERT INTO remote_agents \
                (name, protocol, url, auth_type, auth_token, allow_insecure, \
                 avatar, description, device_id, device_public_key, device_private_key, \
                 device_token, status, last_connected_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(params.name)
        .bind(params.protocol)
        .bind(params.url)
        .bind(params.auth_type)
        .bind(params.auth_token)
        .bind(params.allow_insecure)
        .bind(params.avatar)
        .bind(params.description)
        .bind(params.device_id)
        .bind(params.device_public_key)
        .bind(params.device_private_key)
        .bind(params.device_token)
        .bind(status)
        .bind(Option::<TimestampMs>::None)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let id = result.last_insert_rowid();

        Ok(RemoteAgentRow {
            id,
            name: params.name.to_string(),
            protocol: params.protocol.to_string(),
            url: params.url.to_string(),
            auth_type: params.auth_type.to_string(),
            auth_token: params.auth_token.map(String::from),
            allow_insecure: params.allow_insecure,
            avatar: params.avatar.map(String::from),
            description: params.description.map(String::from),
            device_id: params.device_id.map(String::from),
            device_public_key: params.device_public_key.map(String::from),
            device_private_key: params.device_private_key.map(String::from),
            device_token: params.device_token.map(String::from),
            status: status.to_string(),
            last_connected_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    async fn update(&self, id: i64, params: UpdateRemoteAgentParams<'_>) -> Result<RemoteAgentRow, DbError> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Remote agent '{id}' not found")))?;

        let merged = merge_update(existing, params);

        sqlx::query(
            "UPDATE remote_agents SET \
                name = ?, protocol = ?, url = ?, auth_type = ?, auth_token = ?, \
                allow_insecure = ?, avatar = ?, description = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&merged.name)
        .bind(&merged.protocol)
        .bind(&merged.url)
        .bind(&merged.auth_type)
        .bind(&merged.auth_token)
        .bind(merged.allow_insecure)
        .bind(&merged.avatar)
        .bind(&merged.description)
        .bind(merged.updated_at)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(merged)
    }

    async fn delete(&self, id: i64) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM remote_agents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Remote agent '{id}' not found")));
        }

        Ok(())
    }

    async fn update_status(
        &self,
        id: i64,
        status: &str,
        last_connected_at: Option<TimestampMs>,
    ) -> Result<(), DbError> {
        let now = nomifun_common::now_ms();

        let result = sqlx::query(
            "UPDATE remote_agents SET status = ?, \
             last_connected_at = COALESCE(?, last_connected_at), \
             updated_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(last_connected_at)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Remote agent '{id}' not found")));
        }

        Ok(())
    }
}

/// Merge partial update params into an existing row, returning a new instance.
fn merge_update(existing: RemoteAgentRow, params: UpdateRemoteAgentParams<'_>) -> RemoteAgentRow {
    let now = nomifun_common::now_ms();
    RemoteAgentRow {
        id: existing.id,
        name: params.name.unwrap_or(&existing.name).to_string(),
        protocol: params.protocol.unwrap_or(&existing.protocol).to_string(),
        url: params.url.unwrap_or(&existing.url).to_string(),
        auth_type: params.auth_type.unwrap_or(&existing.auth_type).to_string(),
        auth_token: params.auth_token.map_or(existing.auth_token, |v| v.map(String::from)),
        allow_insecure: params.allow_insecure.unwrap_or(existing.allow_insecure),
        avatar: params.avatar.map_or(existing.avatar, |v| v.map(String::from)),
        description: params.description.map_or(existing.description, |v| v.map(String::from)),
        // Device fields are not updated via UpdateRemoteAgentParams
        device_id: existing.device_id,
        device_public_key: existing.device_public_key,
        device_private_key: existing.device_private_key,
        device_token: existing.device_token,
        status: existing.status,
        last_connected_at: existing.last_connected_at,
        created_at: existing.created_at,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (SqliteRemoteAgentRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteRemoteAgentRepository::new(db.pool().clone());
        (repo, db)
    }

    fn sample_params() -> CreateRemoteAgentParams<'static> {
        CreateRemoteAgentParams {
            name: "Test Agent",
            protocol: "acp",
            url: "wss://remote.example.com",
            auth_type: "bearer",
            auth_token: Some("encrypted_token"),
            allow_insecure: false,
            avatar: None,
            description: Some("A test remote agent"),
            device_id: None,
            device_public_key: None,
            device_private_key: None,
            device_token: None,
        }
    }

    #[tokio::test]
    async fn list_empty() {
        let (repo, _db) = setup().await;
        let agents = repo.list().await.unwrap();
        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn create_returns_populated_fields() {
        let (repo, _db) = setup().await;
        let agent = repo.create(sample_params()).await.unwrap();

        assert!(agent.id > 0);
        assert_eq!(agent.name, "Test Agent");
        assert_eq!(agent.protocol, "acp");
        assert_eq!(agent.url, "wss://remote.example.com");
        assert_eq!(agent.auth_type, "bearer");
        assert_eq!(agent.auth_token.as_deref(), Some("encrypted_token"));
        assert!(!agent.allow_insecure);
        assert!(agent.avatar.is_none());
        assert_eq!(agent.description.as_deref(), Some("A test remote agent"));
        assert_eq!(agent.status, "unknown");
        assert!(agent.last_connected_at.is_none());
        assert!(agent.created_at > 0);
        assert_eq!(agent.created_at, agent.updated_at);
    }

    #[tokio::test]
    async fn create_with_device_fields() {
        let (repo, _db) = setup().await;
        let agent = repo
            .create(CreateRemoteAgentParams {
                protocol: "openClaw",
                device_id: Some("dev-123"),
                device_public_key: Some("enc_pub_key"),
                device_private_key: Some("enc_priv_key"),
                device_token: Some("enc_dev_token"),
                ..sample_params()
            })
            .await
            .unwrap();

        assert_eq!(agent.protocol, "openClaw");
        assert_eq!(agent.device_id.as_deref(), Some("dev-123"));
        assert_eq!(agent.device_public_key.as_deref(), Some("enc_pub_key"));
        assert_eq!(agent.device_private_key.as_deref(), Some("enc_priv_key"));
        assert_eq!(agent.device_token.as_deref(), Some("enc_dev_token"));
    }

    #[tokio::test]
    async fn create_then_find_by_id() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        let found = repo.find_by_id(created.id).await.unwrap().unwrap();
        assert_eq!(found.id, created.id);
        assert_eq!(found.name, "Test Agent");
        assert_eq!(found.protocol, "acp");
        assert_eq!(found.auth_token.as_deref(), Some("encrypted_token"));
    }

    #[tokio::test]
    async fn find_by_id_nonexistent() {
        let (repo, _db) = setup().await;
        assert!(repo.find_by_id(999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_returns_all_ordered_by_created_at() {
        let (repo, _db) = setup().await;
        let a1 = repo.create(sample_params()).await.unwrap();
        let a2 = repo
            .create(CreateRemoteAgentParams {
                name: "Second Agent",
                ..sample_params()
            })
            .await
            .unwrap();

        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, a1.id);
        assert_eq!(all[1].id, a2.id);
    }

    #[tokio::test]
    async fn update_partial_fields() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        let updated = repo
            .update(
                created.id,
                UpdateRemoteAgentParams {
                    name: Some("Updated Agent"),
                    allow_insecure: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "Updated Agent");
        assert!(updated.allow_insecure);
        // Unchanged fields preserved
        assert_eq!(updated.protocol, "acp");
        assert_eq!(updated.url, "wss://remote.example.com");
        assert_eq!(updated.auth_token.as_deref(), Some("encrypted_token"));
        assert!(updated.updated_at >= created.updated_at);
    }

    #[tokio::test]
    async fn update_clear_optional_fields() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();
        assert!(created.description.is_some());

        let updated = repo
            .update(
                created.id,
                UpdateRemoteAgentParams {
                    description: Some(None),
                    auth_token: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(updated.description.is_none());
        assert!(updated.auth_token.is_none());
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo
            .update(999, UpdateRemoteAgentParams::default())
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_existing() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        repo.delete(created.id).await.unwrap();
        assert!(repo.find_by_id(created.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.delete(999).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_status_sets_status_and_last_connected() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();
        assert_eq!(created.status, "unknown");
        assert!(created.last_connected_at.is_none());

        let connect_time = nomifun_common::now_ms();
        repo.update_status(created.id, "connected", Some(connect_time))
            .await
            .unwrap();

        let found = repo.find_by_id(created.id).await.unwrap().unwrap();
        assert_eq!(found.status, "connected");
        assert_eq!(found.last_connected_at, Some(connect_time));
        assert!(found.updated_at >= created.updated_at);
    }

    #[tokio::test]
    async fn update_status_without_last_connected_preserves_existing() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        // First set a connected timestamp
        let connect_time = nomifun_common::now_ms();
        repo.update_status(created.id, "connected", Some(connect_time))
            .await
            .unwrap();

        // Now update status to error without providing last_connected_at
        repo.update_status(created.id, "error", None).await.unwrap();

        let found = repo.find_by_id(created.id).await.unwrap().unwrap();
        assert_eq!(found.status, "error");
        // COALESCE preserves the existing last_connected_at
        assert_eq!(found.last_connected_at, Some(connect_time));
    }

    #[tokio::test]
    async fn update_status_none_on_null_field_stays_null() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();
        assert!(created.last_connected_at.is_none());

        repo.update_status(created.id, "error", None).await.unwrap();

        let found = repo.find_by_id(created.id).await.unwrap().unwrap();
        assert_eq!(found.status, "error");
        assert!(found.last_connected_at.is_none());
    }

    #[tokio::test]
    async fn update_status_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.update_status(999, "connected", None).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_then_list_excludes_deleted() {
        let (repo, _db) = setup().await;
        let a1 = repo.create(sample_params()).await.unwrap();
        let a2 = repo
            .create(CreateRemoteAgentParams {
                name: "Other Agent",
                ..sample_params()
            })
            .await
            .unwrap();

        repo.delete(a1.id).await.unwrap();

        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, a2.id);
    }
}

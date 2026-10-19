use sqlx::SqlitePool;

use crate::{DbError, models::{McpOAuthClientRow, McpOAuthGrantRow}};
use super::mcp_oauth::{IMcpOAuthRepository, McpOAuthClientParams, McpOAuthGrantParams, McpOAuthGrantRefreshParams};

#[derive(Clone, Debug)]
pub struct SqliteMcpOAuthRepository { pool: SqlitePool }

impl SqliteMcpOAuthRepository {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }

    /// Keep each profile's newest cache entry and every registration still
    /// named by a current grant. A replacement grant can release a registration
    /// in another profile, so evaluate all of this owner's profile groups.
    async fn prune_superseded_clients(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        owner_id: &str,
    ) -> Result<(), DbError> {
        sqlx::query(
            "DELETE FROM mcp_oauth_clients WHERE owner_id=? \
             AND client_ref<>(SELECT latest.client_ref FROM mcp_oauth_clients latest \
                 WHERE latest.owner_id=mcp_oauth_clients.owner_id AND latest.issuer=mcp_oauth_clients.issuer \
                 AND latest.profile_key=mcp_oauth_clients.profile_key ORDER BY latest.created_at DESC,latest.client_ref DESC LIMIT 1) \
             AND NOT EXISTS(SELECT 1 FROM mcp_oauth_grants WHERE mcp_oauth_grants.client_ref=mcp_oauth_clients.client_ref)",
        ).bind(owner_id).execute(&mut **transaction).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl IMcpOAuthRepository for SqliteMcpOAuthRepository {
    async fn get_client(&self, owner_id: &str, issuer: &str, profile_key: &str) -> Result<Option<McpOAuthClientRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM mcp_oauth_clients WHERE owner_id=? AND issuer=? AND profile_key=? ORDER BY created_at DESC, client_ref DESC LIMIT 1")
            .bind(owner_id).bind(issuer).bind(profile_key).fetch_optional(&self.pool).await?)
    }

    async fn get_client_by_ref(&self, owner_id: &str, client_ref: &str) -> Result<Option<McpOAuthClientRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM mcp_oauth_clients WHERE owner_id=? AND client_ref=?")
            .bind(owner_id).bind(client_ref).fetch_optional(&self.pool).await?)
    }

    async fn upsert_client(&self, p: McpOAuthClientParams) -> Result<McpOAuthClientRow, DbError> {
        let now = nomifun_common::now_ms();
        let mut transaction = self.pool.begin().await?;
        // Allocate cache order inside this write: wall-clock rollback must not
        // make the new active registration look superseded to the same GC.
        // The guarded addition refuses exhausted i64 clocks instead of letting
        // SQLite promote overflowing integer arithmetic to floating point.
        let row = sqlx::query_as::<_, McpOAuthClientRow>(
            "INSERT INTO mcp_oauth_clients (client_ref,owner_id,issuer,profile_key,registration_kind,client_id,token_endpoint_auth_method,redirect_uris_json,client_secret_encrypted,client_secret_expires_at,created_at,updated_at) \
             SELECT ?,?,?,?,?,?,?,?,?,?,clock.created_at,clock.created_at FROM (\
                 SELECT CASE WHEN previous.latest IS NULL OR previous.latest<? THEN ? ELSE previous.latest+1 END AS created_at \
                 FROM (SELECT MAX(created_at) AS latest FROM mcp_oauth_clients WHERE owner_id=? AND issuer=? AND profile_key=?) previous \
                 WHERE previous.latest IS NULL OR previous.latest<9223372036854775807\
             ) clock WHERE 1 ON CONFLICT(client_ref) DO UPDATE SET \
             client_secret_encrypted=excluded.client_secret_encrypted,client_secret_expires_at=excluded.client_secret_expires_at,updated_at=MAX(mcp_oauth_clients.updated_at+1,excluded.updated_at) \
             WHERE mcp_oauth_clients.owner_id=excluded.owner_id AND mcp_oauth_clients.issuer=excluded.issuer AND mcp_oauth_clients.profile_key=excluded.profile_key \
             AND mcp_oauth_clients.registration_kind=excluded.registration_kind AND mcp_oauth_clients.client_id=excluded.client_id \
             AND mcp_oauth_clients.token_endpoint_auth_method=excluded.token_endpoint_auth_method AND mcp_oauth_clients.redirect_uris_json=excluded.redirect_uris_json RETURNING *")
            .bind(&p.client_ref).bind(&p.owner_id).bind(&p.issuer).bind(&p.profile_key).bind(&p.registration_kind).bind(&p.client_id)
            .bind(&p.token_endpoint_auth_method).bind(&p.redirect_uris_json).bind(&p.client_secret_encrypted).bind(p.client_secret_expires_at)
            .bind(now).bind(now).bind(&p.owner_id).bind(&p.issuer).bind(&p.profile_key)
            .fetch_optional(&mut *transaction).await?
            .ok_or_else(|| DbError::Conflict("OAuth client registration identity changed or its clock is exhausted".into()))?;
        Self::prune_superseded_clients(&mut transaction, &p.owner_id).await?;
        transaction.commit().await?;
        Ok(row)
    }

    async fn get_grant(&self, owner_id: &str, server_id: &str) -> Result<Option<McpOAuthGrantRow>, DbError> {
        Ok(sqlx::query_as("SELECT * FROM mcp_oauth_grants WHERE owner_id=? AND server_id=?")
            .bind(owner_id).bind(server_id).fetch_optional(&self.pool).await?)
    }

    async fn replace_grant(&self, p: McpOAuthGrantParams) -> Result<McpOAuthGrantRow, DbError> {
        let now = nomifun_common::now_ms();
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, McpOAuthGrantRow>(
            "INSERT INTO mcp_oauth_grants (grant_id,owner_id,server_id,connection_config_ref,issuer,resource,client_ref,scopes_json,token_encrypted,expires_at,obtained_at,revision,created_at,updated_at) \
             SELECT ?,?,?,?,?,?,?,?,?,?,?,1,?,? WHERE EXISTS (SELECT 1 FROM mcp_servers WHERE mcp_server_id=? AND deleted_at IS NULL) \
             ON CONFLICT(owner_id,server_id) DO UPDATE SET grant_id=excluded.grant_id,connection_config_ref=excluded.connection_config_ref,issuer=excluded.issuer,resource=excluded.resource,client_ref=excluded.client_ref,scopes_json=excluded.scopes_json,token_encrypted=excluded.token_encrypted,expires_at=excluded.expires_at,obtained_at=excluded.obtained_at,revision=1,created_at=excluded.created_at,updated_at=excluded.updated_at \
             WHERE mcp_oauth_grants.grant_id<>excluded.grant_id RETURNING *")
            .bind(&p.grant_id).bind(&p.owner_id).bind(&p.server_id).bind(&p.connection_config_ref).bind(&p.issuer).bind(&p.resource)
            .bind(&p.client_ref).bind(&p.scopes_json).bind(&p.token_encrypted).bind(p.expires_at).bind(p.obtained_at)
            .bind(now).bind(now).bind(&p.server_id).fetch_optional(&mut *transaction).await?
            .ok_or_else(|| DbError::Conflict("OAuth grant requires a live server and a fresh grant identity".into()))?;
        Self::prune_superseded_clients(&mut transaction, &p.owner_id).await?;
        transaction.commit().await?;
        Ok(row)
    }

    async fn compare_and_swap_grant(&self, p: McpOAuthGrantRefreshParams) -> Result<Option<McpOAuthGrantRow>, DbError> {
        Ok(sqlx::query_as("UPDATE mcp_oauth_grants SET scopes_json=?,token_encrypted=?,expires_at=?,obtained_at=?,revision=revision+1,updated_at=MAX(updated_at+1,?) \
            WHERE owner_id=? AND grant_id=? AND revision=? AND revision<9223372036854775807 \
            AND EXISTS (SELECT 1 FROM mcp_servers WHERE mcp_server_id=mcp_oauth_grants.server_id AND deleted_at IS NULL) RETURNING *")
            .bind(&p.scopes_json).bind(&p.token_encrypted).bind(p.expires_at).bind(p.obtained_at).bind(nomifun_common::now_ms())
            .bind(&p.owner_id).bind(&p.grant_id).bind(p.expected_revision).fetch_optional(&self.pool).await?)
    }

    async fn delete_grant(&self, owner_id: &str, server_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM mcp_oauth_grants WHERE owner_id=? AND server_id=?")
            .bind(owner_id).bind(server_id).execute(&self.pool).await?;
        Ok(())
    }
}

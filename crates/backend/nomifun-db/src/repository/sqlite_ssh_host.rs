use nomifun_common::{SshHostId, TimestampMs};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::SshHostRow;
use crate::repository::ssh_host::{
    CreateSshHostParams, ISshHostRepository, UpdateSshHostParams,
};

/// SQLite-backed [`ISshHostRepository`]. All queries are owner-scoped: the
/// `user_id` predicate makes a cross-owner id return `None`/`NotFound`.
#[derive(Clone, Debug)]
pub struct SqliteSshHostRepository {
    pool: SqlitePool,
}

impl SqliteSshHostRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ISshHostRepository for SqliteSshHostRepository {
    async fn list(&self, user_id: &str) -> Result<Vec<SshHostRow>, DbError> {
        let rows = sqlx::query_as::<_, SshHostRow>(
            "SELECT * FROM ssh_hosts WHERE user_id = ? ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn find(&self, user_id: &str, id: &SshHostId) -> Result<Option<SshHostRow>, DbError> {
        let row = sqlx::query_as::<_, SshHostRow>(
            "SELECT * FROM ssh_hosts WHERE user_id = ? AND ssh_host_id = ?",
        )
        .bind(user_id)
        .bind(id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn create(
        &self,
        user_id: &str,
        params: CreateSshHostParams<'_>,
    ) -> Result<SshHostRow, DbError> {
        let now = nomifun_common::now_ms();
        let ssh_host_id = SshHostId::new();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO ssh_hosts \
                (ssh_host_id, user_id, name, host, port, username, auth_type, \
                 password_encrypted, private_key_encrypted, passphrase_encrypted, \
                 certificate_encrypted, sudo_password_encrypted, host_fingerprint, \
                 status, last_connected_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, 'unknown', NULL, ?, ?) \
             RETURNING id",
        )
        .bind(ssh_host_id.as_str())
        .bind(user_id)
        .bind(params.name)
        .bind(params.host)
        .bind(params.port)
        .bind(params.username)
        .bind(params.auth_type)
        .bind(params.password_encrypted)
        .bind(params.private_key_encrypted)
        .bind(params.passphrase_encrypted)
        .bind(params.certificate_encrypted)
        .bind(params.sudo_password_encrypted)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(SshHostRow {
            id,
            ssh_host_id: ssh_host_id.as_str().to_string(),
            user_id: user_id.to_string(),
            name: params.name.to_string(),
            host: params.host.to_string(),
            port: params.port,
            username: params.username.to_string(),
            auth_type: params.auth_type.to_string(),
            password_encrypted: params.password_encrypted.map(str::to_string),
            private_key_encrypted: params.private_key_encrypted.map(str::to_string),
            passphrase_encrypted: params.passphrase_encrypted.map(str::to_string),
            certificate_encrypted: params.certificate_encrypted.map(str::to_string),
            sudo_password_encrypted: params.sudo_password_encrypted.map(str::to_string),
            host_fingerprint: None,
            status: "unknown".to_string(),
            last_connected_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    async fn update(
        &self,
        user_id: &str,
        id: &SshHostId,
        params: UpdateSshHostParams<'_>,
    ) -> Result<SshHostRow, DbError> {
        // Read-modify-write: SSH-host edits are rare and this keeps the credential
        // "clear vs leave" semantics (Option<Option<_>>) unambiguous.
        let mut row = self
            .find(user_id, id)
            .await?
            .ok_or(DbError::NotFound("ssh_host".into()))?;

        if let Some(v) = params.name {
            row.name = v.to_string();
        }
        if let Some(v) = params.host {
            row.host = v.to_string();
        }
        if let Some(v) = params.port {
            row.port = v;
        }
        if let Some(v) = params.username {
            row.username = v.to_string();
        }
        if let Some(v) = params.auth_type {
            row.auth_type = v.to_string();
        }
        if let Some(v) = params.password_encrypted {
            row.password_encrypted = v.map(str::to_string);
        }
        if let Some(v) = params.private_key_encrypted {
            row.private_key_encrypted = v.map(str::to_string);
        }
        if let Some(v) = params.passphrase_encrypted {
            row.passphrase_encrypted = v.map(str::to_string);
        }
        if let Some(v) = params.certificate_encrypted {
            row.certificate_encrypted = v.map(str::to_string);
        }
        if let Some(v) = params.sudo_password_encrypted {
            row.sudo_password_encrypted = v.map(str::to_string);
        }
        row.updated_at = nomifun_common::now_ms();

        let affected = sqlx::query(
            "UPDATE ssh_hosts SET \
                name = ?, host = ?, port = ?, username = ?, auth_type = ?, \
                password_encrypted = ?, private_key_encrypted = ?, passphrase_encrypted = ?, \
                certificate_encrypted = ?, sudo_password_encrypted = ?, updated_at = ? \
             WHERE user_id = ? AND ssh_host_id = ?",
        )
        .bind(&row.name)
        .bind(&row.host)
        .bind(row.port)
        .bind(&row.username)
        .bind(&row.auth_type)
        .bind(&row.password_encrypted)
        .bind(&row.private_key_encrypted)
        .bind(&row.passphrase_encrypted)
        .bind(&row.certificate_encrypted)
        .bind(&row.sudo_password_encrypted)
        .bind(row.updated_at)
        .bind(user_id)
        .bind(id.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();

        if affected == 0 {
            return Err(DbError::NotFound("ssh_host".into()));
        }
        Ok(row)
    }

    async fn delete(&self, user_id: &str, id: &SshHostId) -> Result<(), DbError> {
        let affected = sqlx::query("DELETE FROM ssh_hosts WHERE user_id = ? AND ssh_host_id = ?")
            .bind(user_id)
            .bind(id.as_str())
            .execute(&self.pool)
            .await?
            .rows_affected();
        if affected == 0 {
            return Err(DbError::NotFound("ssh_host".into()));
        }
        Ok(())
    }

    async fn update_status(
        &self,
        user_id: &str,
        id: &SshHostId,
        status: &str,
        last_connected_at: Option<TimestampMs>,
        host_fingerprint: Option<&str>,
    ) -> Result<(), DbError> {
        let affected = sqlx::query(
            "UPDATE ssh_hosts SET status = ?, last_connected_at = ?, \
                host_fingerprint = COALESCE(?, host_fingerprint), updated_at = ? \
             WHERE user_id = ? AND ssh_host_id = ?",
        )
        .bind(status)
        .bind(last_connected_at)
        .bind(host_fingerprint)
        .bind(nomifun_common::now_ms())
        .bind(user_id)
        .bind(id.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();
        if affected == 0 {
            return Err(DbError::NotFound("ssh_host".into()));
        }
        Ok(())
    }
}

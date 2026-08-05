//! `SshHostService`: owner-scoped host-book CRUD with encryption at the service
//! boundary. The repository stores/returns ciphertext; this layer encrypts on
//! write (AES-256-GCM via `nomifun_common`) and decrypts only when a connection
//! actually needs the credential. Response DTOs are masked — plaintext never
//! leaves this process except into an SSH connection.
use std::sync::Arc;

use nomifun_common::SshHostId;
use nomifun_db::{
    CreateSshHostParams, ISshHostRepository, UpdateSshHostParams,
};
use zeroize::Zeroizing;

use crate::dto::{
    CreateSshHostRequest, SshHostResponse, UpdateSshHostRequest, SECRET_MASK,
};

/// Errors surfaced by the host-book service.
#[derive(Debug, thiserror::Error)]
pub enum SshServiceError {
    #[error("ssh host not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// Decrypted credential bundle handed to the transport layer (never serialized).
pub struct DecryptedCredential {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: String,
    pub password: Option<Zeroizing<String>>,
    pub private_key: Option<Zeroizing<String>>,
    pub passphrase: Option<Zeroizing<String>>,
    pub certificate: Option<Zeroizing<String>>,
    pub sudo_password: Option<Zeroizing<String>>,
}

/// Host-book service. Cheap to clone (`Arc` internals).
#[derive(Clone)]
pub struct SshHostService {
    repo: Arc<dyn ISshHostRepository>,
    encryption_key: [u8; 32],
}

impl SshHostService {
    pub fn new(repo: Arc<dyn ISshHostRepository>, encryption_key: [u8; 32]) -> Self {
        Self { repo, encryption_key }
    }

    fn encrypt(&self, plaintext: Option<&str>) -> Result<Option<String>, SshServiceError> {
        match plaintext {
            Some(p) if !p.is_empty() => nomifun_common::encrypt_string(p, &self.encryption_key)
                .map(Some)
                .map_err(|e| SshServiceError::Internal(format!("encrypt: {e}"))),
            _ => Ok(None),
        }
    }

    /// List the owner's hosts as masked responses.
    pub async fn list(&self, user_id: &str) -> Result<Vec<SshHostResponse>, SshServiceError> {
        let rows = self
            .repo
            .list(user_id)
            .await
            .map_err(|e| SshServiceError::Internal(e.to_string()))?;
        Ok(rows.into_iter().map(SshHostResponse::from).collect())
    }

    /// Fetch one owned host as a masked response.
    pub async fn get(
        &self,
        user_id: &str,
        id: &SshHostId,
    ) -> Result<SshHostResponse, SshServiceError> {
        self.repo
            .find(user_id, id)
            .await
            .map_err(|e| SshServiceError::Internal(e.to_string()))?
            .map(SshHostResponse::from)
            .ok_or(SshServiceError::NotFound)
    }

    /// Create a host, encrypting every supplied credential.
    pub async fn create(
        &self,
        user_id: &str,
        req: CreateSshHostRequest,
    ) -> Result<SshHostResponse, SshServiceError> {
        validate_auth_type(&req.auth_type)?;
        let password = self.encrypt(req.password.as_deref())?;
        let private_key = self.encrypt(req.private_key.as_deref())?;
        let passphrase = self.encrypt(req.passphrase.as_deref())?;
        let certificate = self.encrypt(req.certificate.as_deref())?;
        let sudo_password = self.encrypt(req.sudo_password.as_deref())?;

        let row = self
            .repo
            .create(
                user_id,
                CreateSshHostParams {
                    name: &req.name,
                    host: &req.host,
                    port: req.port,
                    username: &req.username,
                    auth_type: &req.auth_type,
                    password_encrypted: password.as_deref(),
                    private_key_encrypted: private_key.as_deref(),
                    passphrase_encrypted: passphrase.as_deref(),
                    certificate_encrypted: certificate.as_deref(),
                    sudo_password_encrypted: sudo_password.as_deref(),
                },
            )
            .await
            .map_err(|e| SshServiceError::Internal(e.to_string()))?;
        Ok(SshHostResponse::from(row))
    }

    /// Update a host. A credential field equal to the mask is left unchanged; an
    /// empty string clears it; any other value is re-encrypted.
    pub async fn update(
        &self,
        user_id: &str,
        id: &SshHostId,
        req: UpdateSshHostRequest,
    ) -> Result<SshHostResponse, SshServiceError> {
        if let Some(at) = &req.auth_type {
            validate_auth_type(at)?;
        }
        // For each credential: None => leave; Some(mask) => leave; Some("") =>
        // clear; Some(other) => re-encrypt.
        let cred = |v: &Option<String>| -> Result<Option<Option<String>>, SshServiceError> {
            match v {
                None => Ok(None),
                Some(s) if s == SECRET_MASK => Ok(None),
                Some(s) if s.is_empty() => Ok(Some(None)),
                Some(s) => Ok(Some(self.encrypt(Some(s))?)),
            }
        };
        let password = cred(&req.password)?;
        let private_key = cred(&req.private_key)?;
        let passphrase = cred(&req.passphrase)?;
        let certificate = cred(&req.certificate)?;
        let sudo_password = cred(&req.sudo_password)?;

        let params = UpdateSshHostParams {
            name: req.name.as_deref(),
            host: req.host.as_deref(),
            port: req.port,
            username: req.username.as_deref(),
            auth_type: req.auth_type.as_deref(),
            password_encrypted: password.as_ref().map(|o| o.as_deref()),
            private_key_encrypted: private_key.as_ref().map(|o| o.as_deref()),
            passphrase_encrypted: passphrase.as_ref().map(|o| o.as_deref()),
            certificate_encrypted: certificate.as_ref().map(|o| o.as_deref()),
            sudo_password_encrypted: sudo_password.as_ref().map(|o| o.as_deref()),
        };
        let row = self
            .repo
            .update(user_id, id, params)
            .await
            .map_err(map_not_found)?;
        Ok(SshHostResponse::from(row))
    }

    /// Delete an owned host.
    pub async fn delete(&self, user_id: &str, id: &SshHostId) -> Result<(), SshServiceError> {
        self.repo.delete(user_id, id).await.map_err(map_not_found)
    }

    /// Decrypt an owned host's credentials for the transport layer. Never
    /// serialized; the returned secrets are `Zeroizing`.
    pub async fn decrypt_credential(
        &self,
        user_id: &str,
        id: &SshHostId,
    ) -> Result<DecryptedCredential, SshServiceError> {
        let row = self
            .repo
            .find(user_id, id)
            .await
            .map_err(|e| SshServiceError::Internal(e.to_string()))?
            .ok_or(SshServiceError::NotFound)?;

        let dec = |c: &Option<String>| -> Result<Option<Zeroizing<String>>, SshServiceError> {
            match c {
                Some(ct) => nomifun_common::decrypt_string(ct, &self.encryption_key)
                    .map(|p| Some(Zeroizing::new(p)))
                    .map_err(|e| SshServiceError::Internal(format!("decrypt: {e}"))),
                None => Ok(None),
            }
        };

        Ok(DecryptedCredential {
            host: row.host,
            port: u16::try_from(row.port).unwrap_or(22),
            username: row.username,
            auth_type: row.auth_type,
            password: dec(&row.password_encrypted)?,
            private_key: dec(&row.private_key_encrypted)?,
            passphrase: dec(&row.passphrase_encrypted)?,
            certificate: dec(&row.certificate_encrypted)?,
            sudo_password: dec(&row.sudo_password_encrypted)?,
        })
    }
}

fn validate_auth_type(at: &str) -> Result<(), SshServiceError> {
    match at {
        "password" | "key" | "certificate" | "agent" => Ok(()),
        other => Err(SshServiceError::BadRequest(format!(
            "unknown auth_type {other:?}"
        ))),
    }
}

fn map_not_found(e: nomifun_db::DbError) -> SshServiceError {
    match e {
        nomifun_db::DbError::NotFound(_) => SshServiceError::NotFound,
        other => SshServiceError::Internal(other.to_string()),
    }
}

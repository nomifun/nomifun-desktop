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
    CreateSshHostRequest, ImportedSshHost, SshHostResponse, SshImportResult, SshImportSkipReason,
    SkippedSshHost, UpdateSshHostRequest, SECRET_MASK,
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

    /// Add the confirmed `~/.ssh/config` candidates to the book.
    ///
    /// Goes through [`Self::create`] host by host, so imported rows are encrypted
    /// and validated by exactly the code path the form uses — there is no second
    /// way into this table.
    ///
    /// `available` is the server's own scan of its ssh config (never anything the
    /// client supplied), and `requested` selects from it by alias. Anything that
    /// does not become a host is reported rather than dropped: a duplicate is
    /// skipped, an alias the config no longer has is named, and a host imported
    /// without a usable credential is flagged.
    pub async fn import_hosts(
        &self,
        user_id: &str,
        requested: &[String],
        available: &[crate::ssh_config::SshConfigHost],
    ) -> Result<SshImportResult, SshServiceError> {
        // The existing book, read once. Masked responses are enough: duplicate
        // detection compares names and endpoints, never secrets.
        let existing = self.list(user_id).await?;
        let mut names: std::collections::HashSet<String> =
            existing.iter().map(|h| h.name.clone()).collect();
        let mut endpoints: std::collections::HashSet<(String, i64, String)> = existing
            .iter()
            .map(|h| (h.host.clone(), h.port, h.username.clone()))
            .collect();

        let mut result = SshImportResult::default();
        for alias in requested {
            let Some(candidate) = available.iter().find(|c| &c.alias == alias) else {
                result.skipped.push(SkippedSshHost {
                    alias: alias.clone(),
                    reason: SshImportSkipReason::NotInConfig,
                });
                continue;
            };
            if names.contains(&candidate.alias) {
                result.skipped.push(SkippedSshHost {
                    alias: candidate.alias.clone(),
                    reason: SshImportSkipReason::DuplicateName,
                });
                continue;
            }
            let username = candidate.username.clone().unwrap_or_default();
            let endpoint = (candidate.host.clone(), candidate.port, username.clone());
            if endpoints.contains(&endpoint) {
                result.skipped.push(SkippedSshHost {
                    alias: candidate.alias.clone(),
                    reason: SshImportSkipReason::DuplicateEndpoint,
                });
                continue;
            }

            // Read the key the config points at and store it, so an import
            // produces hosts that actually connect instead of a list of stubs
            // waiting for the paste the import was supposed to save.
            let private_key = candidate
                .identity_file
                .as_deref()
                .and_then(|path| crate::ssh_config::read_identity_file(std::path::Path::new(path)));
            // A host that names an identity file authenticates by key even when
            // we could not read it — that is the truth about the host, and it
            // opens the edit form on the right field. One that names none is left
            // on the password default.
            let auth_type = if candidate.identity_file.is_some() {
                "key"
            } else {
                "password"
            };
            let created = self
                .create(
                    user_id,
                    CreateSshHostRequest {
                        name: candidate.alias.clone(),
                        host: candidate.host.clone(),
                        port: candidate.port,
                        username,
                        auth_type: auth_type.to_string(),
                        password: None,
                        private_key: private_key.as_deref().cloned(),
                        passphrase: None,
                        certificate: None,
                        sudo_password: None,
                    },
                )
                .await?;
            names.insert(candidate.alias.clone());
            endpoints.insert(endpoint);
            let needs_username = created.username.is_empty();
            result.imported.push(ImportedSshHost {
                alias: candidate.alias.clone(),
                ssh_host_id: created.ssh_host_id,
                needs_credential: private_key.is_none(),
                needs_username,
            });
        }
        Ok(result)
    }

    /// Stamp a host as connected now, recording its fingerprint (best-effort;
    /// called after a successful dial).
    pub async fn mark_connected(
        &self,
        user_id: &str,
        id: &SshHostId,
        fingerprint: Option<&str>,
    ) -> Result<(), SshServiceError> {
        self.repo
            .update_status(user_id, id, "connected", Some(nomifun_common::now_ms()), fingerprint)
            .await
            .map_err(map_not_found)
    }

    /// Walk a host's status back to `disconnected` after a dial or a live link
    /// failed. Until this existed `mark_connected` was the only writer of the
    /// column, so a host read `connected` forever after its first successful dial.
    ///
    /// The column is per-HOST while links are per-CONVERSATION, so treat it as a
    /// last-known hint for the host book — the live truth is the pool's `watch`
    /// per link. `detail` is logged rather than stored: the column holds a bare
    /// status word, and a diagnostic string persisted next to a credential is a
    /// leak waiting to happen.
    pub async fn mark_unreachable(
        &self,
        user_id: &str,
        id: &SshHostId,
        detail: &str,
    ) -> Result<(), SshServiceError> {
        tracing::debug!(ssh_host_id = %id, detail = %detail, "ssh host marked unreachable");
        // `update_status` assigns `last_connected_at` unconditionally (only the
        // fingerprint is COALESCEd), so passing `None` would erase the very hint
        // this column exists to provide. Read it back and hand it straight in.
        let last_connected_at = self
            .repo
            .find(user_id, id)
            .await
            .map_err(|e| SshServiceError::Internal(e.to_string()))?
            .ok_or(SshServiceError::NotFound)?
            .last_connected_at;
        self.repo
            .update_status(user_id, id, "disconnected", last_connected_at, None)
            .await
            .map_err(map_not_found)
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

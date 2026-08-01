use std::sync::Arc;

use nomifun_api_types::{ProviderConnectionResponse, UpsertProviderConnectionRequest};
use nomifun_common::{AppError, ProviderId, encrypt_string};
use nomifun_db::{
    IProviderConnectionRepository, IProviderRepository, ProviderConnectionRow,
    UpsertProviderConnectionParams,
};

use crate::provider::validate_base_url;

/// The provider's own `base_url`/`api_key` pair is the implicit default
/// connection, so the per-role table must never shadow it.
const RESERVED_DEFAULT_ROLE_MESSAGE: &str =
    "role 'default' is reserved: the provider's own base_url/api_key is the default connection";

/// Business logic for non-default per-role provider connection profiles.
///
/// Credentials are write-only on the wire: requests carry a structured JSON
/// object that is serialized and encrypted at rest; responses only expose
/// `has_credentials`.
#[derive(Clone)]
pub struct ProviderConnectionService {
    repo: Arc<dyn IProviderConnectionRepository>,
    provider_repo: Arc<dyn IProviderRepository>,
    encryption_key: [u8; 32],
}

impl ProviderConnectionService {
    pub fn new(
        repo: Arc<dyn IProviderConnectionRepository>,
        provider_repo: Arc<dyn IProviderRepository>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            repo,
            provider_repo,
            encryption_key,
        }
    }

    /// Crate-internal handle to the underlying repository, for flows that
    /// copy connection rows verbatim (ciphertext included) and so cannot go
    /// through the write-only-credentials wire API — e.g. provider clone.
    pub(crate) fn repository(&self) -> Arc<dyn IProviderConnectionRepository> {
        self.repo.clone()
    }

    /// List all connection profiles for one provider, ordered by role.
    pub async fn list(&self, provider_id: &str) -> Result<Vec<ProviderConnectionResponse>, AppError> {
        self.require_provider(provider_id).await?;
        let rows = self.repo.list_for_provider(provider_id).await?;
        rows.into_iter().map(row_to_response).collect()
    }

    /// Insert or update the connection for `(provider_id, req.role)`.
    ///
    /// Creating requires a non-empty credentials object; updating with
    /// `credentials: None` keeps the stored ciphertext unchanged.
    pub async fn upsert(
        &self,
        provider_id: &str,
        req: UpsertProviderConnectionRequest,
    ) -> Result<ProviderConnectionResponse, AppError> {
        self.require_provider(provider_id).await?;
        validate_role(&req.role)?;
        validate_base_url(&req.base_url)?;
        let auth_scheme = req.auth_scheme.trim().to_ascii_lowercase();
        if auth_scheme.is_empty() {
            return Err(AppError::BadRequest("auth_scheme is required".into()));
        }

        let existing = self.repo.get(provider_id, &req.role).await?;
        let credentials_encrypted = match (&req.credentials, &existing) {
            // Update without credentials: keep the stored ciphertext.
            (None, Some(row)) => row.credentials_encrypted.clone(),
            (None, None) => {
                return Err(AppError::BadRequest(
                    "credentials are required when creating a connection".into(),
                ));
            }
            (Some(credentials), _) => {
                validate_credentials_object(credentials)?;
                let plaintext = serde_json::to_string(credentials)
                    .map_err(|e| AppError::Internal(format!("Failed to serialize credentials: {e}")))?;
                encrypt_string(&plaintext, &self.encryption_key)?
            }
        };

        let extra = req.extra.unwrap_or_else(|| serde_json::json!({}));
        let extra_json = serde_json::to_string(&extra)
            .map_err(|e| AppError::Internal(format!("Failed to serialize extra: {e}")))?;

        let row = self
            .repo
            .upsert(
                provider_id,
                &UpsertProviderConnectionParams {
                    role: &req.role,
                    label: req.label.as_deref(),
                    base_url: &req.base_url,
                    auth_scheme: &auth_scheme,
                    credentials_encrypted: &credentials_encrypted,
                    is_full_url: req.is_full_url,
                    extra: &extra_json,
                },
            )
            .await?;
        row_to_response(row)
    }

    /// Delete one connection profile; returns whether a row was removed.
    pub async fn delete(&self, provider_id: &str, role: &str) -> Result<bool, AppError> {
        self.require_provider(provider_id).await?;
        Ok(self.repo.delete(provider_id, role).await?)
    }

    /// Validate the canonical id and require the provider row to exist.
    async fn require_provider(&self, provider_id: &str) -> Result<(), AppError> {
        ProviderId::parse(provider_id)
            .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))?;
        if self.provider_repo.find_by_id(provider_id).await?.is_none() {
            return Err(AppError::NotFound(format!("Provider {provider_id} not found")));
        }
        Ok(())
    }
}

/// Enforce `^[a-z][a-z0-9_-]{0,31}$` and reserve `default` for the providers
/// row itself. Shared with `ProviderModelService`, so a stored
/// `provider_models.connection_role` can always resolve to a connection role.
pub(crate) fn validate_role(role: &str) -> Result<(), AppError> {
    let mut bytes = role.bytes();
    let starts_lowercase = bytes.next().is_some_and(|b| b.is_ascii_lowercase());
    let tail_ok = bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !starts_lowercase || !tail_ok || role.len() > 32 {
        return Err(AppError::BadRequest(
            "role must match ^[a-z][a-z0-9_-]{0,31}$".into(),
        ));
    }
    if role == "default" {
        return Err(AppError::BadRequest(RESERVED_DEFAULT_ROLE_MESSAGE.into()));
    }
    Ok(())
}

fn validate_credentials_object(credentials: &serde_json::Value) -> Result<(), AppError> {
    match credentials.as_object() {
        Some(map) if !map.is_empty() => Ok(()),
        _ => Err(AppError::BadRequest(
            "credentials must be a non-empty JSON object".into(),
        )),
    }
}

fn row_to_response(row: ProviderConnectionRow) -> Result<ProviderConnectionResponse, AppError> {
    let extra = serde_json::from_str(&row.extra)
        .map_err(|e| AppError::Internal(format!("Failed to parse connection extra JSON: {e}")))?;
    Ok(ProviderConnectionResponse {
        connection_id: row.connection_id,
        provider_id: row.provider_id,
        role: row.role,
        label: row.label,
        base_url: row.base_url,
        auth_scheme: row.auth_scheme,
        has_credentials: !row.credentials_encrypted.is_empty(),
        is_full_url: row.is_full_url,
        extra,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::CreateProviderRequest;
    use nomifun_db::{
        SqliteProviderConnectionRepository, SqliteProviderModelRepository, SqliteProviderRepository,
        init_database_memory,
    };
    use serde_json::json;

    const TEST_KEY: [u8; 32] = [0x42; 32];

    async fn setup() -> (ProviderConnectionService, nomifun_db::SqlitePool) {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        std::mem::forget(db);
        let service = ProviderConnectionService::new(
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            TEST_KEY,
        );
        (service, pool)
    }

    async fn seed_provider(pool: &nomifun_db::SqlitePool) -> String {
        let provider_service = crate::provider::ProviderService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            TEST_KEY,
        );
        provider_service
            .create(CreateProviderRequest {
                provider_id: None,
                platform: "openai".into(),
                name: "Primary".into(),
                base_url: "https://api.example.com/v1".into(),
                api_key: "sk-primary".into(),
                models: vec![],
                enabled: true,
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
            .unwrap()
            .provider_id
    }

    fn voice_request() -> UpsertProviderConnectionRequest {
        UpsertProviderConnectionRequest {
            role: "voice".into(),
            label: Some("Voice endpoint".into()),
            base_url: "https://voice.example.com/v1".into(),
            auth_scheme: "bearer".into(),
            credentials: Some(json!({ "api_key": "sk-voice-plaintext-secret" })),
            is_full_url: false,
            extra: None,
        }
    }

    async fn stored_ciphertext(pool: &nomifun_db::SqlitePool, provider_id: &str, role: &str) -> String {
        nomifun_db::sqlx::query_scalar(
            "SELECT credentials_encrypted FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id)
        .bind(role)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn upsert_then_list_signals_credentials_without_echoing_plaintext() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;

        let created = svc.upsert(&provider_id, voice_request()).await.unwrap();
        assert_eq!(created.role, "voice");
        assert!(created.has_credentials);
        nomifun_common::validate_uuidv7(&created.connection_id).unwrap();
        assert_eq!(created.extra, json!({}), "extra defaults to an empty object");

        let listed = svc.list(&provider_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].has_credentials);

        // Neither the plaintext secret nor a credentials field may appear
        // anywhere in the serialized response.
        for response in [&created, &listed[0]] {
            let wire = serde_json::to_string(response).unwrap();
            assert!(!wire.contains("sk-voice-plaintext-secret"), "plaintext leaked: {wire}");
            assert!(!wire.contains("credentials_encrypted"), "ciphertext leaked: {wire}");
            assert!(!wire.contains(r#""credentials""#), "credentials echoed: {wire}");
        }

        // At rest the ciphertext is not the plaintext.
        let ciphertext = stored_ciphertext(&pool, &provider_id, "voice").await;
        assert!(!ciphertext.is_empty());
        assert!(!ciphertext.contains("sk-voice-plaintext-secret"));
    }

    #[tokio::test]
    async fn upsert_without_credentials_keeps_stored_ciphertext_and_updates_base_url() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;

        let first = svc.upsert(&provider_id, voice_request()).await.unwrap();
        let original_ciphertext = stored_ciphertext(&pool, &provider_id, "voice").await;

        let updated = svc
            .upsert(
                &provider_id,
                UpsertProviderConnectionRequest {
                    credentials: None,
                    base_url: "https://voice2.example.com/v1".into(),
                    ..voice_request()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.connection_id, first.connection_id);
        assert_eq!(updated.base_url, "https://voice2.example.com/v1");
        assert!(updated.has_credentials);

        let unchanged_ciphertext = stored_ciphertext(&pool, &provider_id, "voice").await;
        assert_eq!(
            unchanged_ciphertext, original_ciphertext,
            "update without credentials must keep the stored ciphertext byte-for-byte"
        );

        // Supplying credentials re-encrypts (fresh nonce => new ciphertext).
        svc.upsert(
            &provider_id,
            UpsertProviderConnectionRequest {
                credentials: Some(json!({ "api_key": "sk-voice-rotated" })),
                ..voice_request()
            },
        )
        .await
        .unwrap();
        let rotated_ciphertext = stored_ciphertext(&pool, &provider_id, "voice").await;
        assert_ne!(rotated_ciphertext, original_ciphertext);
    }

    #[tokio::test]
    async fn create_requires_non_empty_credentials_object() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;

        for credentials in [None, Some(json!({})), Some(json!("sk-raw")), Some(json!(["k"]))] {
            let err = svc
                .upsert(
                    &provider_id,
                    UpsertProviderConnectionRequest {
                        credentials,
                        ..voice_request()
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
        }
        assert!(svc.list(&provider_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn role_default_is_rejected_with_reserved_message() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;

        let err = svc
            .upsert(
                &provider_id,
                UpsertProviderConnectionRequest {
                    role: "default".into(),
                    ..voice_request()
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AppError::BadRequest(ref message)
                    if message
                        == "role 'default' is reserved: the provider's own base_url/api_key is the default connection"
            ),
            "unexpected error: {err:?}"
        );
    }

    #[tokio::test]
    async fn role_regex_violations_are_rejected() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;

        for role in [
            "",
            "Voice",
            "9voice",
            "_voice",
            "voice role",
            "voice.role",
            "语音",
            &"a".repeat(33),
        ] {
            let err = svc
                .upsert(
                    &provider_id,
                    UpsertProviderConnectionRequest {
                        role: role.into(),
                        ..voice_request()
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(
                err.status_code(),
                axum::http::StatusCode::BAD_REQUEST,
                "role {role:?} must be rejected"
            );
        }

        // Boundary: 32 chars total (1 + 31) is allowed.
        let max_role = format!("a{}", "b".repeat(31));
        svc.upsert(
            &provider_id,
            UpsertProviderConnectionRequest {
                role: max_role,
                ..voice_request()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn invalid_base_url_and_blank_auth_scheme_are_rejected() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;

        let err = svc
            .upsert(
                &provider_id,
                UpsertProviderConnectionRequest {
                    base_url: "ftp://voice.example.com".into(),
                    ..voice_request()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);

        let err = svc
            .upsert(
                &provider_id,
                UpsertProviderConnectionRequest {
                    auth_scheme: "   ".into(),
                    ..voice_request()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);

        // auth_scheme is normalized to trimmed lowercase.
        let created = svc
            .upsert(
                &provider_id,
                UpsertProviderConnectionRequest {
                    auth_scheme: " Api_Key ".into(),
                    ..voice_request()
                },
            )
            .await
            .unwrap();
        assert_eq!(created.auth_scheme, "api_key");
    }

    #[tokio::test]
    async fn unknown_provider_is_not_found_for_all_operations() {
        let (svc, _pool) = setup().await;
        let missing = "0190f5fe-7c00-7a00-8000-000000000099";

        for err in [
            svc.list(missing).await.unwrap_err(),
            svc.upsert(missing, voice_request()).await.unwrap_err(),
            svc.delete(missing, "voice").await.unwrap_err(),
        ] {
            assert_eq!(err.status_code(), axum::http::StatusCode::NOT_FOUND);
        }

        let err = svc.list("not-a-provider-id").await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;
        svc.upsert(&provider_id, voice_request()).await.unwrap();

        assert!(svc.delete(&provider_id, "voice").await.unwrap());
        assert!(!svc.delete(&provider_id, "voice").await.unwrap());
        assert!(svc.list(&provider_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn provider_delete_cascades_connections() {
        let (svc, pool) = setup().await;
        let provider_id = seed_provider(&pool).await;
        svc.upsert(&provider_id, voice_request()).await.unwrap();

        let provider_repo = SqliteProviderRepository::new(pool.clone());
        nomifun_db::IProviderRepository::delete(&provider_repo, &provider_id)
            .await
            .unwrap();

        let count: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM provider_connections WHERE provider_id = ?",
        )
        .bind(&provider_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "provider delete must cascade to provider_connections");
    }
}

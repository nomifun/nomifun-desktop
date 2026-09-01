//! Fresh-v4 system, provider, and installation-token route composition.
//!
//! This module is deliberately independent from `AppServices`. Every
//! repository used here is backed by the caller-provided Fresh-v4 pool.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use async_trait::async_trait;
use serde::Serialize;
use sqlx::SqlitePool;

use nomifun_common::{AppError, ProviderUsage};
use nomifun_db::{
    DbError, IInstanceTokenRepository,
    SqliteClientPreferenceRepository, SqliteProviderConnectionRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository, SqliteSettingsRepository,
};
use nomifun_system::provider_deletion::ProviderDeletionCoordinator;
use nomifun_system::{
    ClientPrefService, ModelFetchService, ProviderConnectionService, ProviderModelService,
    ProviderService, SettingsService, SystemRouterState, VersionCheckService, system_routes,
};

/// Build the Fresh-v4 system/provider router from one canonical SQLite pool.
///
/// The router intentionally exposes the existing `nomifun-system` service
/// semantics while ensuring that every repository is constructed from the
/// supplied pool. No legacy `AppServices`, database path, or table is opened.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    pool: SqlitePool,
    encryption_key: [u8; 32],
    data_dir: PathBuf,
    work_dir: PathBuf,
    work_dir_is_cli_override: bool,
) -> Result<Router> {
    let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let provider_model_repo = Arc::new(SqliteProviderModelRepository::new(pool.clone()));
    let capability_repo = Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
    let connection_repo = Arc::new(SqliteProviderConnectionRepository::new(pool.clone()));
    let deletion_coordinator = Arc::new(FreshV4ProviderDeletionCoordinator);

    let state = SystemRouterState {
        settings_service: SettingsService::new(Arc::new(SqliteSettingsRepository::new(
            pool.clone(),
        ))),
        client_pref_service: ClientPrefService::new(Arc::new(
            SqliteClientPreferenceRepository::new(pool.clone()),
        )),
        provider_service: ProviderService::new(
            provider_repo.clone(),
            provider_model_repo.clone(),
            capability_repo.clone(),
            connection_repo.clone(),
            encryption_key,
        )
        .with_deletion_coordinator(deletion_coordinator.clone()),
        provider_connection_service: ProviderConnectionService::new(
            connection_repo.clone(),
            provider_repo.clone(),
            capability_repo.clone(),
            encryption_key,
        ),
        model_fetch_service: ModelFetchService::new_dynamic(
            provider_repo.clone(),
            encryption_key,
        ),
        provider_model_service: ProviderModelService::new(
            provider_model_repo,
            capability_repo,
            provider_repo,
            connection_repo,
            deletion_coordinator,
        ),
        managed_model_service: None,
        version_check_service: VersionCheckService::new_dynamic(env!("CARGO_PKG_VERSION").into()),
        data_dir,
        work_dir,
        work_dir_is_cli_override,
    };

    Ok(system_routes(state).layer(middleware::from_fn(
        reject_fresh_v4_legacy_scanning_operations,
    )))
}

/// Minimal deletion coordination for a standalone Fresh-v4 host.
///
/// The full app coordinator scans companion, conversation, workshop, and
/// execution side stores. Those stores are deliberately outside a Fresh-v4
/// system router, so this coordinator only reports no external usages and
/// leaves the v4 SQLite repository to perform its own referential cleanup.
#[derive(Debug, Default)]
pub(crate) struct FreshV4ProviderDeletionCoordinator;

#[async_trait]
impl ProviderDeletionCoordinator for FreshV4ProviderDeletionCoordinator {
    async fn usages(&self, _provider_id: &str) -> Result<Vec<ProviderUsage>, AppError> {
        Ok(Vec::new())
    }

    async fn prepare_soft_model_cleanup(
        &self,
        _provider_id: &str,
        _model: &str,
    ) -> Result<nomifun_db::ProviderModelCleanupPlan, AppError> {
        Ok(nomifun_db::ProviderModelCleanupPlan::default())
    }
}

#[derive(Debug, Serialize)]
struct FreshV4UnsupportedResponse {
    error_code: &'static str,
    message: &'static str,
}

/// Some mutation implementations still contain legacy-domain reference scans.
/// Keep those routes closed on a Fresh-v4 host until their references have a
/// v4-native owner; ordinary provider/model/connection writes remain available.
async fn reject_fresh_v4_legacy_scanning_operations(
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let rejected = (request.method() == Method::DELETE
        && (is_provider_delete_path(path) || path == "/api/provider-models"))
        || (request.method() == Method::POST
            && matches!(
                path,
                "/api/system/factory-reset" | "/api/system/work-dir"
            ));
    if rejected {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(FreshV4UnsupportedResponse {
                error_code: "FRESH_V4_PROVIDER_DELETE_UNSUPPORTED",
                message:
                    "this operation is unavailable on the Fresh-v4 host until all reference scans are v4-native",
            }),
        )
            .into_response();
    }

    next.run(request).await
}

fn is_provider_delete_path(path: &str) -> bool {
    let mut segments = path.trim_matches('/').split('/');
    matches!(
        (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next()
        ),
        (Some("api"), Some("providers"), Some(_), None)
    )
}

/// Fresh-v4 installation token storage.
///
/// Unlike `SqliteInstanceTokenRepository`, this implementation never touches
/// the legacy `instance_access_token` table. The installation row is created
/// by Fresh-v4 bootstrap because its owner identity is required by the schema.
#[derive(Clone, Debug)]
pub struct FreshV4InstallationTokenRepository {
    pool: SqlitePool,
}

impl FreshV4InstallationTokenRepository {
    /// Construct a repository backed by the supplied Fresh-v4 pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn update(
        &self,
        current_verifier_hash: Option<&str>,
        status: &str,
    ) -> Result<(), DbError> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE installation_auth \
             SET current_verifier_hash = ?1, status = ?2, \
                 auth_revision = auth_revision + 1, updated_at = ?3 \
             WHERE singleton_key = 'installation'",
        )
        .bind(current_verifier_hash)
        .bind(status)
        .bind(nomifun_common::now_ms())
        .execute(&mut *transaction)
        .await?;

        if result.rows_affected() != 1 {
            return Err(DbError::NotFound(
                "Fresh-v4 installation auth row not found".to_owned(),
            ));
        }
        transaction.commit().await?;
        Ok(())
    }
}

#[async_trait]
impl IInstanceTokenRepository for FreshV4InstallationTokenRepository {
    async fn get(&self) -> Result<Option<String>, DbError> {
        let row: Option<(Option<String>, String)> = sqlx::query_as(
            "SELECT current_verifier_hash, status \
             FROM installation_auth WHERE singleton_key = 'installation'",
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some((Some(hash), status)) if status == "active" => {
                if is_lower_sha256_hex(&hash) {
                    Ok(Some(hash))
                } else {
                    Err(DbError::Init(
                        "Fresh-v4 installation auth hash is not lowercase SHA-256 hex".to_owned(),
                    ))
                }
            }
            Some((None, status)) if status == "revoked" => Ok(None),
            Some((_hash, status)) => Err(DbError::Init(format!(
                "invalid Fresh-v4 installation auth status/hash pair: {status}"
            ))),
            None => Ok(None),
        }
    }

    async fn set(&self, token_hash: &str) -> Result<(), DbError> {
        if !is_lower_sha256_hex(token_hash) {
            return Err(DbError::Conflict(
                "Fresh-v4 installation token hash must be lowercase SHA-256 hex".to_owned(),
            ));
        }
        self.update(Some(token_hash), "active").await
    }

    async fn clear(&self) -> Result<(), DbError> {
        self.update(None, "revoked").await
    }
}

fn is_lower_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE installation_auth (
                singleton_key TEXT PRIMARY KEY,
                owner_user_id TEXT NOT NULL,
                current_verifier_hash TEXT,
                auth_revision INTEGER NOT NULL,
                status TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO installation_auth
             (singleton_key, owner_user_id, current_verifier_hash,
              auth_revision, status, updated_at)
             VALUES ('installation', 'owner', NULL, 1, 'revoked', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn installation_token_uses_v4_row_and_increments_revision() {
        let pool = test_pool().await;
        let repository = FreshV4InstallationTokenRepository::new(pool.clone());

        assert_eq!(repository.get().await.unwrap(), None);
        let hash_a = "a".repeat(64);
        repository.set(&hash_a).await.unwrap();
        assert_eq!(repository.get().await.unwrap().as_deref(), Some(hash_a.as_str()));

        let revision: i64 =
            sqlx::query_scalar("SELECT auth_revision FROM installation_auth")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(revision, 2);

        repository.clear().await.unwrap();
        assert_eq!(repository.get().await.unwrap(), None);
        let (status, revision): (String, i64) =
            sqlx::query_as("SELECT status, auth_revision FROM installation_auth")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "revoked");
        assert_eq!(revision, 3);

        let legacy_table: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name = 'instance_access_token'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert_eq!(legacy_table, None);
    }

    #[tokio::test]
    async fn missing_installation_row_is_not_created_from_legacy_storage() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE installation_auth (
                singleton_key TEXT PRIMARY KEY,
                owner_user_id TEXT NOT NULL,
                current_verifier_hash TEXT,
                auth_revision INTEGER NOT NULL,
                status TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        let repository = FreshV4InstallationTokenRepository::new(pool);
        assert_eq!(repository.get().await.unwrap(), None);
        assert!(matches!(
            repository.set(&"a".repeat(64)).await,
            Err(DbError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn installation_token_rejects_noncanonical_hashes() {
        let pool = test_pool().await;
        let repository = FreshV4InstallationTokenRepository::new(pool);
        for hash in ["", &"A".repeat(64), &"z".repeat(64), &"a".repeat(63)] {
            assert!(matches!(
                repository.set(hash).await,
                Err(DbError::Conflict(_))
            ));
        }
    }

    #[test]
    fn provider_delete_guard_only_matches_provider_resource_routes() {
        assert!(is_provider_delete_path("/api/providers/provider-id"));
        assert!(is_provider_delete_path("api/providers/provider-id/"));
        assert!(!is_provider_delete_path("/api/providers"));
        assert!(!is_provider_delete_path("/api/providers/provider-id/models"));
        assert!(!is_provider_delete_path("/api/provider-models"));
    }
}

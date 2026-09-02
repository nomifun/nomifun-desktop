use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use async_trait::async_trait;
use axum::extract::DefaultBodyLimit;
use axum::http::Method;
use axum::middleware::{self, from_fn_with_state};
use axum::routing::get;
use axum::{Json, Router};
use tower_http::cors::{Any, CorsLayer};

use nomifun_agent_contracts::PluginMountId;
use nomifun_agent_domain_wave5::REMOTE_INGRESS_MOUNT_ID;
use nomifun_agent_platform::{
    AgentPlatform, agent_session_command_service_key,
    agent_session_query_service_key,
};
use nomifun_auth::{
    AuthPolicy, AuthRouterState, AuthState, CookieConfig, JwtService, QrTokenStore, TrustState,
    auth_middleware, auth_routes, csrf_middleware, require_instance_owner_middleware,
    security_headers_middleware,
    trust_resolve_middleware,
};
use nomifun_common::UserId;
use nomifun_db::models::User;
use nomifun_db::{DbError, IInstanceTokenRepository, IUserRepository};
use nomifun_v4_root::FreshV4BootstrapOutcome;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::router::{
    agent_platform_host, create_agent_platform_router, fresh_v4_system,
    instance_token_routes, remote_rest, remote_runtime::RemoteRuntimeCoordinator,
};
#[cfg(test)]
use super::APPLICATION_BUILD_IDENTITY;

const V4_AUTH_USERNAME: &str = "admin";
const V4_AUTH_METADATA_FILE: &str = ".nomifun-v4-auth-metadata.json";
static V4_AUTH_METADATA_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The only canonical application host selected by a production bootstrap.
#[derive(Clone, Debug)]
pub enum CanonicalHost {
    FreshV4(FreshV4Host),
}

/// A fully composed Fresh-v4 application.
///
/// This type intentionally does not contain `AppServices`. The v4 pool,
/// installation authentication, Agent platform, and HTTP surface are owned by
/// one small composition root.
#[derive(Clone)]
pub struct FreshV4Application {
    pool: SqlitePool,
    platform: Arc<AgentPlatform>,
    remote_runtime: Arc<RemoteRuntimeCoordinator>,
    user_repo: Arc<dyn IUserRepository>,
    jwt_service: Arc<JwtService>,
    owner_id: Arc<str>,
    auth_policy: AuthPolicy,
    local_trust_secret: Option<Arc<str>>,
    router: Router,
}

impl std::fmt::Debug for FreshV4Application {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FreshV4Application")
            .field("owner_id", &self.owner_id)
            .field("auth_policy", &self.auth_policy)
            .field(
                "local_trust_secret",
                &self.local_trust_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish_non_exhaustive()
    }
}

impl FreshV4Application {
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    pub fn platform(&self) -> &Arc<AgentPlatform> {
        &self.platform
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn user_repo(&self) -> Arc<dyn IUserRepository> {
        self.user_repo.clone()
    }

    pub fn jwt_service(&self) -> &Arc<JwtService> {
        &self.jwt_service
    }

    pub fn auth_policy(&self) -> AuthPolicy {
        self.auth_policy
    }

    pub fn local_trust_secret(&self) -> Option<&str> {
        self.local_trust_secret.as_deref()
    }

    /// Pre-seed the v4 installation owner for an authenticated host.
    ///
    /// Returns `true` when the installation still needs interactive setup.
    pub async fn ensure_admin_credentials(
        &self,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<bool> {
        if !self.auth_policy.requires_admin_provisioning() {
            return Ok(false);
        }
        if self
            .user_repo
            .has_users()
            .await
            .map_err(|error| anyhow::anyhow!("v4 admin bootstrap query failed: {error}"))?
        {
            return Ok(false);
        }
        let Some(password) = password else {
            return Ok(true);
        };
        let username = username.unwrap_or(V4_AUTH_USERNAME);
        nomifun_auth::validate_username(username)
            .map_err(|error| anyhow::anyhow!("invalid v4 admin username: {error}"))?;
        nomifun_auth::validate_password(password)
            .map_err(|error| anyhow::anyhow!("invalid v4 admin password: {error}"))?;

        let password = password.to_owned();
        let hash = tokio::task::spawn_blocking(move || nomifun_auth::hash_password(&password))
            .await
            .context("v4 admin password hash task failed")?
            .map_err(|error| anyhow::anyhow!("v4 admin password hashing failed: {error}"))?;
        let provisioned = self
            .user_repo
            .set_system_user_credentials_if_uninitialized(username, &hash)
            .await
            .map_err(|error| anyhow::anyhow!("v4 admin credential write failed: {error}"))?;
        if provisioned {
            Ok(false)
        } else {
            Ok(!self.user_repo.has_users().await.unwrap_or(false))
        }
    }

    pub async fn close(self) -> Result<()> {
        self.remote_runtime.shutdown().await;
        let shutdown = self
            .platform
            .shutdown()
            .await
            .context("shut down Fresh-v4 runtime bindings");
        self.platform.pool().close().await;
        shutdown
    }
}

/// App-level handle for a root that has passed the Fresh-v4 coordinator.
#[derive(Clone, Debug)]
pub struct FreshV4Host {
    outcome: FreshV4BootstrapOutcome,
}

impl CanonicalHost {
    pub(crate) fn from_bootstrap(outcome: FreshV4BootstrapOutcome) -> Self {
        Self::FreshV4(FreshV4Host { outcome })
    }

    pub fn fresh_v4(&self) -> &FreshV4Host {
        match self {
            Self::FreshV4(host) => host,
        }
    }

    pub async fn compose(&self, config: &crate::AppConfig) -> Result<FreshV4Application> {
        self.fresh_v4().compose(config).await
    }

    /// Retained for callers that need a typed fail-closed error without
    /// composing a host. Production startup should call [`Self::compose`].
    pub fn unavailable_error(&self, operation: &str) -> anyhow::Error {
        self.fresh_v4().unavailable_error(operation)
    }
}

impl FreshV4Host {
    pub fn bootstrap_outcome(&self) -> &FreshV4BootstrapOutcome {
        &self.outcome
    }

    pub fn canonical_root(&self) -> &Path {
        &self.outcome.canonical_root
    }

    pub fn canonical_root_path(&self) -> PathBuf {
        self.outcome.canonical_root.clone()
    }

    pub async fn compose(&self, config: &crate::AppConfig) -> Result<FreshV4Application> {
        if !nomifun_common::paths::paths_equivalent(
            &config.data_dir,
            &self.outcome.canonical_root,
        ) {
            anyhow::bail!(
                "Fresh-v4 host config root {} differs from coordinator root {}",
                config.data_dir.display(),
                self.canonical_root().display()
            );
        }
        if matches!(config.auth_policy, AuthPolicy::TrustLocalToken)
            && config.local_trust_secret.is_none()
        {
            anyhow::bail!("TrustLocalToken requires a per-boot local trust secret");
        }
        super::environment::install_fresh_v4_storage_generation_environment(config)
            .context("initialize Fresh-v4 storage generation")?;

        let database_path = self
            .canonical_root()
            .join(nomifun_v4_root::FRESH_V4_DATABASE_FILE);
        let pool = agent_platform_host::open_validated_pool(&database_path).await?;
        let (user_repo, owner_id, jwt_secret) =
            match FreshV4UserRepository::open(pool.clone(), self.canonical_root()).await {
                Ok(value) => value,
                Err(error) => {
                    pool.close().await;
                    return Err(error);
                }
            };
        let encryption_key = match crate::config::load_or_create_data_encryption_key(
            self.canonical_root(),
            &jwt_secret,
        )
        .context("load Fresh-v4 provider encryption key")
        {
            Ok(key) => key,
            Err(error) => {
                pool.close().await;
                return Err(error);
            }
        };
        let platform = match agent_platform_host::build_from_open_pool(
            pool.clone(),
            self.canonical_root()
                .join(nomifun_v4_root::FRESH_V4_READY_MARKER_FILE),
            self.outcome.ready_marker.clone(),
            self.outcome
                .ready_marker
                .canonical_schema_manifest_digest
                .clone(),
            Some(pool.clone()),
            encryption_key,
            config.work_dir.clone(),
        )
        .await
        {
            Ok(platform) => platform,
            Err(error) => {
                pool.close().await;
                return Err(error);
            }
        };

        let jwt_service = Arc::new(JwtService::new(jwt_secret));
        let cookie_config = Arc::new(CookieConfig::from_env());
        let auth_state = AuthRouterState {
            jwt_service: jwt_service.clone(),
            user_repo: user_repo.clone(),
            cookie_config: cookie_config.clone(),
            qr_token_store: Arc::new(QrTokenStore::new()),
        };
        let auth_middleware_state = AuthState {
            jwt_service: jwt_service.clone(),
            user_repo: user_repo.clone(),
            cookie_config: cookie_config.clone(),
        };
        let trust_state = TrustState {
            policy: config.auth_policy,
            local_trust_secret: config.local_trust_secret.clone(),
            authoritative_user_id: Arc::from(owner_id.as_ref()),
        };
        let owner_state = nomifun_auth::InstanceOwnerState::new(Arc::from(owner_id.as_ref()));
        let system_router = match fresh_v4_system::build(
            pool.clone(),
            encryption_key,
            self.canonical_root().to_path_buf(),
            config.work_dir.clone(),
            config.work_dir_is_cli_override,
        )
        .context("build Fresh-v4 system and provider routes")
        {
            Ok(router) => router,
            Err(error) => {
                let _ = platform.shutdown().await;
                pool.close().await;
                return Err(error);
            }
        }
        .route_layer(from_fn_with_state(
            owner_state.clone(),
            require_instance_owner_middleware,
        ))
        .route_layer(from_fn_with_state(
            auth_middleware_state.clone(),
            auth_middleware,
        ));
        let token_repository = Arc::new(
            fresh_v4_system::FreshV4InstallationTokenRepository::new(pool.clone()),
        );
        let remote_auth_admission =
            Arc::new(nomifun_auth::RemoteAuthAdmissionFence::new());
        let initial_token = match token_repository
            .get()
            .await
            .context("read Fresh-v4 installation token")
        {
            Ok(token) => token,
            Err(error) => {
                let _ = platform.shutdown().await;
                pool.close().await;
                return Err(error);
            }
        };
        let token_validator = Arc::new(nomifun_auth::InstanceTokenValidator::new(initial_token));
        if let Err(error) = seed_installation_token(
            token_repository.as_ref(),
            token_validator.as_ref(),
            std::env::var("NOMIFUN_ACCESS_TOKEN").ok().as_deref(),
        )
        .await
        {
            let _ = platform.shutdown().await;
            pool.close().await;
            return Err(error.context("seed Fresh-v4 installation token"));
        }
        let token_state = instance_token_routes::InstanceTokenRouterState {
            provider_repo: Arc::new(nomifun_db::SqliteProviderRepository::new(pool.clone())),
            token_repo: token_repository,
            token_validator,
            admission: remote_auth_admission.clone(),
        };
        let remote_token_validator = token_state.token_validator.clone();
        let remote_runtime = Arc::new(RemoteRuntimeCoordinator::new(
            platform.clone(),
            self.canonical_root().to_path_buf(),
        ));
        if let Err(error) = remote_runtime.reconcile_opening_sessions().await {
            let _ = remote_runtime.shutdown().await;
            let _ = platform.shutdown().await;
            pool.close().await;
            return Err(anyhow::anyhow!(
                "recover opening Fresh-v4 Remote Sessions: {error}"
            ));
        }
        let owner_user_id = UserId::parse(owner_id.as_ref())
            .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 owner id: {error}"))?;
        let remote_services = platform
            .kernel_registry()
            .declared_service_view(&PluginMountId::from(REMOTE_INGRESS_MOUNT_ID))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Remote ingress mount has no declared AgentSession service view"
                )
            })?;
        let remote_session_command =
            remote_services.require(&agent_session_command_service_key())?;
        let remote_session_query =
            remote_services.require(&agent_session_query_service_key())?;
        let remote_router = remote_rest::build(
            platform.clone(),
            remote_session_command.clone(),
            remote_session_query.clone(),
            remote_token_validator,
            owner_user_id.clone(),
            remote_runtime.clone(),
        );
        let remote_mcp_router = nomifun_public::canonical_remote_mcp_router(
            platform.clone(),
            remote_session_command,
            remote_session_query,
            token_state.token_validator.clone(),
            owner_user_id,
            remote_runtime.clone(),
        );
        let token_state = instance_token_routes::InstanceTokenRouterState {
            admission: remote_auth_admission,
            ..token_state
        };
        let agent_router = create_agent_platform_router(platform.clone())
            .route_layer(from_fn_with_state(
                auth_middleware_state.clone(),
                auth_middleware,
            ));

        let token_router = if matches!(config.auth_policy, AuthPolicy::Required) {
            instance_token_routes::instance_token_routes_authenticated(
                token_state,
                auth_middleware_state.clone(),
                owner_state,
            )
        } else {
            instance_token_routes::instance_token_routes(token_state)
        };
        let mut router = Router::new()
            .merge(auth_routes(auth_state))
            .merge(token_router)
            .merge(system_router)
            .merge(agent_router)
            .merge(remote_router)
            .nest("/mcp", remote_mcp_router)
            .route("/health", get(v4_health))
            .layer(middleware::from_fn(security_headers_middleware))
            .layer(from_fn_with_state(cookie_config, csrf_middleware))
            .layer(from_fn_with_state(trust_state, trust_resolve_middleware))
            .layer(DefaultBodyLimit::max(nomifun_common::constants::BODY_LIMIT));

        if config.auth_policy.allows_local_webview() {
            router = router.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods([
                        Method::GET,
                        Method::POST,
                        Method::PUT,
                        Method::PATCH,
                        Method::DELETE,
                        Method::OPTIONS,
                    ])
                    .allow_headers(Any),
            );
        }

        Ok(FreshV4Application {
            pool,
            platform,
            remote_runtime,
            user_repo,
            jwt_service,
            owner_id: Arc::from(owner_id.as_ref()),
            auth_policy: config.auth_policy,
            local_trust_secret: config.local_trust_secret.clone(),
            router,
        })
    }

    pub fn unavailable_error(&self, operation: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "{operation} selected the ready Fresh-v4 root at {}; \
             canonical host composition failed closed; \
             legacy v3 data-layer initialization was not attempted",
            self.canonical_root().display()
        )
    }
}

async fn seed_installation_token(
    repository: &dyn IInstanceTokenRepository,
    validator: &nomifun_auth::InstanceTokenValidator,
    seed: Option<&str>,
) -> Result<bool> {
    let Some(seed) = seed.map(str::trim).filter(|seed| !seed.is_empty()) else {
        return Ok(false);
    };
    if validator.validate(seed) {
        return Ok(false);
    }
    let hash = nomifun_auth::token_sha256_hex(seed);
    repository
        .set(&hash)
        .await
        .map_err(|error| anyhow::anyhow!("persist NOMIFUN_ACCESS_TOKEN seed: {error}"))?;
    validator.set_token(hash);
    tracing::info!(
        "Remote access token seeded from NOMIFUN_ACCESS_TOKEN for this Fresh-v4 installation"
    );
    Ok(true)
}


#[derive(Clone, Debug, Serialize)]
struct V4HealthResponse {
    status: &'static str,
    version: &'static str,
    data_generation: u32,
}

async fn v4_health() -> Json<V4HealthResponse> {
    Json(V4HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        data_generation: nomifun_agent_contracts::FRESH_V4_DATA_GENERATION,
    })
}

struct FreshV4UserRepository {
    pool: SqlitePool,
    owner_id: UserId,
    metadata: tokio::sync::Mutex<FreshV4AuthMetadata>,
    metadata_path: PathBuf,
}

impl FreshV4UserRepository {
    async fn open(pool: SqlitePool, root: &Path) -> Result<(Arc<Self>, UserId, String)> {
        let metadata_path = root.join(V4_AUTH_METADATA_FILE);
        let metadata = load_or_create_auth_metadata(&metadata_path)?;
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT owner_user_id, current_verifier_hash, status \
             FROM installation_auth WHERE singleton_key = 'installation'",
        )
        .fetch_optional(&pool)
        .await
        .context("read Fresh-v4 installation auth")?;
        let owner_id = match row {
            Some((owner, verifier, status)) => {
                validate_installation_auth_row(&verifier, &status)?;
                UserId::parse(&owner)
                    .map_err(|error| anyhow::anyhow!("invalid v4 installation owner: {error}"))?
            }
            None => {
                let owner_id = UserId::new();
                sqlx::query(
                    "INSERT INTO installation_auth \
                     (singleton_key, owner_user_id, current_verifier_hash, auth_revision, status, updated_at) \
                     VALUES ('installation', ?, NULL, 1, 'revoked', ?)",
                )
                .bind(owner_id.as_ref())
                .bind(nomifun_common::now_ms())
                .execute(&pool)
                .await
                .context("create Fresh-v4 installation auth")?;
                owner_id
            }
        };
        let jwt_secret = metadata.jwt_secret.clone();
        Ok((
            Arc::new(Self {
                pool,
                owner_id: owner_id.clone(),
                metadata: tokio::sync::Mutex::new(metadata),
                metadata_path,
            }),
            owner_id,
            jwt_secret,
        ))
    }

    async fn owner_exists(&self) -> Result<bool, DbError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM installation_auth \
             WHERE singleton_key = 'installation' AND owner_user_id = ?",
        )
        .bind(self.owner_id.as_ref())
        .fetch_one(&self.pool)
        .await?;
        Ok(count == 1)
    }

    async fn load_user(&self) -> Result<Option<User>, DbError> {
        if !self.owner_exists().await? {
            return Ok(None);
        }
        let metadata = self.metadata.lock().await.clone();
        Ok(Some(User {
            id: 0,
            user_id: self.owner_id.clone(),
            username: metadata.username,
            email: None,
            password_hash: metadata.password_hash,
            avatar_path: None,
            jwt_secret: Some(metadata.jwt_secret),
            created_at: 0,
            updated_at: 0,
            last_login: None,
        }))
    }

    async fn update_metadata(
        &self,
        username: Option<&str>,
        password_hash: Option<&str>,
        jwt_secret: Option<&str>,
        only_if_password_empty: bool,
    ) -> Result<bool, DbError> {
        if !self.owner_exists().await? {
            return Err(DbError::NotFound(
                "Fresh-v4 installation owner not found".to_owned(),
            ));
        }
        let mut metadata = self.metadata.lock().await;
        if only_if_password_empty && !metadata.password_hash.trim().is_empty() {
            return Ok(false);
        }
        let mut next_metadata = metadata.clone();
        if let Some(username) = username {
            next_metadata.username = username.to_owned();
        }
        if let Some(password_hash) = password_hash {
            next_metadata.password_hash = password_hash.to_owned();
        }
        if let Some(jwt_secret) = jwt_secret {
            next_metadata.jwt_secret = jwt_secret.to_owned();
        }
        persist_auth_metadata(&self.metadata_path, &next_metadata)
            .map_err(|error| DbError::Init(error.to_string()))?;
        *metadata = next_metadata;
        Ok(true)
    }
}

#[async_trait]
impl IUserRepository for FreshV4UserRepository {
    async fn has_users(&self) -> Result<bool, DbError> {
        Ok(self
            .load_user()
            .await?
            .is_some_and(|user| !user.password_hash.trim().is_empty()))
    }

    async fn get_system_user(&self) -> Result<Option<User>, DbError> {
        self.load_user().await
    }

    async fn get_primary_webui_user(&self) -> Result<Option<User>, DbError> {
        self.load_user().await
    }

    async fn set_system_user_credentials(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<(), DbError> {
        self.update_metadata(Some(username), Some(password_hash), None, false)
            .await?;
        Ok(())
    }

    async fn set_system_user_credentials_if_uninitialized(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<bool, DbError> {
        self.update_metadata(Some(username), Some(password_hash), None, true)
            .await
    }

    async fn set_system_user_password_if_uninitialized(
        &self,
        password_hash: &str,
    ) -> Result<bool, DbError> {
        self.update_metadata(None, Some(password_hash), None, true)
            .await
    }

    async fn create_user(&self, _username: &str, _password_hash: &str) -> Result<User, DbError> {
        Err(DbError::Conflict(
            "Fresh-v4 installation auth has one owner identity".to_owned(),
        ))
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DbError> {
        Ok(self
            .load_user()
            .await?
            .filter(|user| user.username == username))
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<User>, DbError> {
        if id != self.owner_id.as_ref() {
            return Ok(None);
        }
        self.load_user().await
    }

    async fn list_users(&self) -> Result<Vec<User>, DbError> {
        Ok(self.load_user().await?.into_iter().collect())
    }

    async fn count_users(&self) -> Result<i64, DbError> {
        Ok(i64::from(self.load_user().await?.is_some()))
    }

    async fn update_password(&self, user_id: &str, password_hash: &str) -> Result<(), DbError> {
        if user_id != self.owner_id.as_ref() {
            return Err(DbError::NotFound("Fresh-v4 user not found".to_owned()));
        }
        self.update_metadata(None, Some(password_hash), None, false)
            .await?;
        Ok(())
    }

    async fn update_username(&self, user_id: &str, username: &str) -> Result<(), DbError> {
        if user_id != self.owner_id.as_ref() {
            return Err(DbError::NotFound("Fresh-v4 user not found".to_owned()));
        }
        self.update_metadata(Some(username), None, None, false)
            .await?;
        Ok(())
    }

    async fn update_last_login(&self, user_id: &str) -> Result<(), DbError> {
        if user_id != self.owner_id.as_ref() || !self.owner_exists().await? {
            return Err(DbError::NotFound("Fresh-v4 user not found".to_owned()));
        }
        Ok(())
    }

    async fn update_jwt_secret(&self, user_id: &str, jwt_secret: &str) -> Result<(), DbError> {
        if user_id != self.owner_id.as_ref() {
            return Err(DbError::NotFound("Fresh-v4 user not found".to_owned()));
        }
        self.update_metadata(None, None, Some(jwt_secret), false)
            .await?;
        Ok(())
    }
}

fn validate_installation_auth_row(raw: &Option<String>, status: &str) -> Result<()> {
    if !matches!(status, "active" | "revoked") {
        anyhow::bail!("Fresh-v4 installation auth has an invalid status");
    }
    if (status == "active" && raw.is_none()) || (status == "revoked" && raw.is_some()) {
        anyhow::bail!("Fresh-v4 installation auth has an invalid status/hash pair");
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FreshV4AuthMetadata {
    username: String,
    password_hash: String,
    jwt_secret: String,
}

fn load_or_create_auth_metadata(path: &Path) -> Result<FreshV4AuthMetadata> {
    let metadata = match fs::read(path) {
        Ok(bytes) => {
            let metadata: FreshV4AuthMetadata = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse Fresh-v4 auth metadata {}", path.display()))?;
            if bytes != nomifun_agent_contracts::canonical_json_bytes(&metadata)? {
                anyhow::bail!("Fresh-v4 auth metadata is not canonical JSON");
            }
            metadata
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let metadata = FreshV4AuthMetadata {
                username: V4_AUTH_USERNAME.to_owned(),
                password_hash: String::new(),
                jwt_secret: nomifun_auth::generate_random_secret_string(),
            };
            persist_auth_metadata(path, &metadata)?;
            metadata
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read Fresh-v4 auth metadata {}", path.display()));
        }
    };
    nomifun_auth::validate_username(&metadata.username)
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 auth username: {error}"))?;
    if metadata.jwt_secret.trim().is_empty() {
        anyhow::bail!("Fresh-v4 auth JWT secret is empty");
    }
    Ok(metadata)
}

fn persist_auth_metadata(path: &Path, metadata: &FreshV4AuthMetadata) -> Result<()> {
    nomifun_auth::validate_username(&metadata.username)
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 auth username: {error}"))?;
    if metadata.jwt_secret.trim().is_empty() {
        anyhow::bail!("Fresh-v4 auth JWT secret is empty");
    }
    let bytes = nomifun_agent_contracts::canonical_json_bytes(metadata)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("Fresh-v4 auth metadata has no parent directory")?;
    fs::create_dir_all(parent)?;
    let (temporary_path, mut temporary_file) = create_auth_metadata_temp_file(path)?;
    let result = (|| -> Result<()> {
        use std::io::Write as _;

        temporary_file.write_all(&bytes)?;
        temporary_file.sync_all()?;
        drop(temporary_file);
        replace_auth_metadata_file(&temporary_path, path)?;
        sync_auth_metadata_parent(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_auth_metadata_temp_file(path: &Path) -> Result<(PathBuf, fs::File)> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("Fresh-v4 auth metadata has no parent directory")?;
    let file_name = path
        .file_name()
        .context("Fresh-v4 auth metadata has no file name")?;

    for _ in 0..128 {
        let sequence = V4_AUTH_METADATA_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = file_name.to_os_string();
        temporary_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
        let temporary_path = parent.join(temporary_name);
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "create Fresh-v4 auth metadata temporary file {}",
                        temporary_path.display()
                    )
                });
            }
        }
    }

    anyhow::bail!(
        "could not reserve a Fresh-v4 auth metadata temporary file beside {}",
        path.display()
    )
}

#[cfg(windows)]
fn replace_auth_metadata_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both buffers are NUL-terminated UTF-16 paths and remain valid
    // for the duration of the synchronous MoveFileExW call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_auth_metadata_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_auth_metadata_parent(parent: &Path) -> io::Result<()> {
    fs::File::open(parent)?.sync_all()
}

#[cfg(windows)]
fn sync_auth_metadata_parent(_parent: &Path) -> io::Result<()> {
    // MoveFileExW with MOVEFILE_WRITE_THROUGH is the durable publication
    // boundary available here; opening directories for sync is not portable
    // across supported Windows filesystems.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_auth_metadata_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use clap::Parser;
    use nomifun_agent_contracts::FRESH_V4_DATA_GENERATION;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn assert_no_auth_metadata_temporary_files(parent: &Path) {
        let temporary_prefix = format!("{V4_AUTH_METADATA_FILE}.tmp-");
        for entry in fs::read_dir(parent).unwrap() {
            let entry = entry.unwrap();
            assert!(
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&temporary_prefix),
                "temporary auth metadata file remained: {}",
                entry.path().display()
            );
        }
    }

    #[derive(Default)]
    struct RecordingTokenRepository {
        hash: tokio::sync::Mutex<Option<String>>,
        set_calls: AtomicUsize,
    }

    #[async_trait]
    impl IInstanceTokenRepository for RecordingTokenRepository {
        async fn get(&self) -> std::result::Result<Option<String>, DbError> {
            Ok(self.hash.lock().await.clone())
        }

        async fn set(&self, token_hash: &str) -> std::result::Result<(), DbError> {
            self.set_calls.fetch_add(1, AtomicOrdering::AcqRel);
            *self.hash.lock().await = Some(token_hash.to_owned());
            Ok(())
        }

        async fn clear(&self) -> std::result::Result<(), DbError> {
            *self.hash.lock().await = None;
            Ok(())
        }
    }

    #[tokio::test]
    async fn headless_installation_token_seed_is_persisted_once_and_activates_validator() {
        let repository = RecordingTokenRepository::default();
        let validator = nomifun_auth::InstanceTokenValidator::new(None);

        assert!(
            seed_installation_token(&repository, &validator, Some("headless-secret"))
                .await
                .unwrap()
        );
        let expected_hash = nomifun_auth::token_sha256_hex("headless-secret");
        assert_eq!(repository.set_calls.load(AtomicOrdering::Acquire), 1);
        assert_eq!(
            repository.get().await.unwrap().as_deref(),
            Some(expected_hash.as_str())
        );
        assert!(validator.validate("headless-secret"));

        assert!(
            !seed_installation_token(&repository, &validator, Some("headless-secret"))
                .await
                .unwrap()
        );
        assert_eq!(repository.set_calls.load(AtomicOrdering::Acquire), 1);
        assert!(
            !seed_installation_token(&repository, &validator, Some("  "))
                .await
                .unwrap()
        );
    }

    #[test]
    fn auth_metadata_update_roundtrips_without_leaving_a_temp_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(V4_AUTH_METADATA_FILE);
        let metadata = FreshV4AuthMetadata {
            username: "admin".to_owned(),
            password_hash: "$2b$12$initial".to_owned(),
            jwt_secret: "initial-secret".to_owned(),
        };
        persist_auth_metadata(&path, &metadata).unwrap();

        let updated = FreshV4AuthMetadata {
            username: "new-admin".to_owned(),
            password_hash: "$2b$12$updated".to_owned(),
            jwt_secret: "updated-secret".to_owned(),
        };
        persist_auth_metadata(&path, &updated).unwrap();

        assert_eq!(load_or_create_auth_metadata(&path).unwrap().username, "new-admin");
        assert_eq!(
            fs::read(&path).unwrap(),
            nomifun_agent_contracts::canonical_json_bytes(&updated).unwrap()
        );
        assert_no_auth_metadata_temporary_files(directory.path());
    }

    #[tokio::test]
    async fn auth_metadata_persist_failure_does_not_change_repository_memory() {
        let directory = tempfile::tempdir().unwrap();
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let database_path = outcome
            .canonical_root
            .join(nomifun_v4_root::FRESH_V4_DATABASE_FILE);
        let pool = agent_platform_host::open_validated_pool(&database_path)
            .await
            .unwrap();
        let (mut repository, owner_id, _) =
            FreshV4UserRepository::open(pool.clone(), &outcome.canonical_root)
                .await
                .unwrap();
        let original = repository.load_user().await.unwrap().unwrap();

        let conflicting_path = outcome.canonical_root.join("auth-metadata-conflict");
        fs::create_dir(&conflicting_path).unwrap();
        Arc::get_mut(&mut repository).unwrap().metadata_path = conflicting_path;

        let error = repository
            .update_metadata(Some("changed-admin"), None, None, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Is a directory") || !error.to_string().is_empty());

        let current = repository.load_user().await.unwrap().unwrap();
        assert_eq!(current.username, original.username);
        assert_eq!(current.password_hash, original.password_hash);
        assert_eq!(current.jwt_secret, original.jwt_secret);
        assert_no_auth_metadata_temporary_files(&outcome.canonical_root);
        assert!(outcome.canonical_root.join("auth-metadata-conflict").is_dir());
        assert_eq!(owner_id, current.user_id);
        pool.close().await;
    }

    #[tokio::test]
    async fn canonical_host_composes_without_opening_the_legacy_database() {
        let directory = tempfile::tempdir().unwrap();
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let host = CanonicalHost::from_bootstrap(outcome);
        let config = crate::AppConfig {
            data_dir: directory.path().to_path_buf(),
            work_dir: directory.path().to_path_buf(),
            auth_policy: AuthPolicy::NoAuth,
            ..crate::AppConfig::default()
        };

        let application = host.compose(&config).await.unwrap();
        assert_eq!(
            application
                .platform()
                .materialized_registry()
                .unwrap()
                .capabilities
                .len(),
            137
        );
        assert!(
            directory
                .path()
                .join(nomifun_v4_root::FRESH_V4_DATABASE_FILE)
                .is_file()
        );
        assert!(!config.database_path().exists());
        let storage_generation =
            fs::read_to_string(directory.path().join("storage-generation")).unwrap();
        assert!(nomifun_common::validate_uuidv7(&storage_generation).is_ok());
        assert_eq!(
            nomifun_system::sysinfo::get_system_info().storage_generation,
            storage_generation
        );
        assert_eq!(FRESH_V4_DATA_GENERATION, 4);
        application.close().await.unwrap();
    }

    #[tokio::test]
    async fn canonical_host_can_restart_after_platform_materialization() {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::AppConfig {
            data_dir: directory.path().to_path_buf(),
            work_dir: directory.path().to_path_buf(),
            auth_policy: AuthPolicy::NoAuth,
            ..crate::AppConfig::default()
        };
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let host = CanonicalHost::from_bootstrap(outcome);
        let application = host.compose(&config).await.unwrap();
        application.close().await.unwrap();

        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .expect("a composed Fresh-v4 root must remain restartable");
        let host = CanonicalHost::from_bootstrap(outcome);
        let application = host.compose(&config).await.unwrap();
        application.close().await.unwrap();
    }

    #[tokio::test]
    async fn canonical_host_retains_coordinator_identity_and_fails_closed_explicitly() {
        let directory = tempfile::tempdir().unwrap();
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let expected_root = outcome.canonical_root.clone();
        let host = CanonicalHost::from_bootstrap(outcome);

        assert_eq!(host.fresh_v4().canonical_root(), expected_root);
        let error = host.unavailable_error("test startup");
        assert!(error.to_string().contains("canonical host composition"));
        assert!(
            error
                .to_string()
                .contains("legacy v3 data-layer initialization was not attempted")
        );
    }

    #[test]
    fn default_cli_can_still_be_used_by_canonical_hosts() {
        let cli = crate::cli::Cli::parse_from(["canonical-host-test"]);
        assert!(cli.command.is_none());
    }

    #[tokio::test]
    async fn canonical_auth_setup_and_login_use_the_v4_installation_row() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let directory = tempfile::tempdir().unwrap();
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let host = CanonicalHost::from_bootstrap(outcome);
        let config = crate::AppConfig {
            data_dir: directory.path().to_path_buf(),
            work_dir: directory.path().to_path_buf(),
            auth_policy: AuthPolicy::Required,
            ..crate::AppConfig::default()
        };
        let application = host.compose(&config).await.unwrap();
        let router = application.router();

        let status = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let status_body = axum::body::to_bytes(status.into_body(), usize::MAX)
            .await
            .unwrap();
        let status_json: serde_json::Value = serde_json::from_slice(&status_body).unwrap();
        assert_eq!(status_json["needs_setup"], true);

        let setup = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/setup")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username": "admin",
                            "password": "correct horse battery staple"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(setup.status(), StatusCode::OK);
        let setup_body = axum::body::to_bytes(setup.into_body(), usize::MAX)
            .await
            .unwrap();
        let setup_json: serde_json::Value = serde_json::from_slice(&setup_body).unwrap();
        let token = setup_json["token"].as_str().unwrap();

        let user = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/user")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(user.status(), StatusCode::OK);
        let user_body = axum::body::to_bytes(user.into_body(), usize::MAX)
            .await
            .unwrap();
        let user_json: serde_json::Value = serde_json::from_slice(&user_body).unwrap();
        assert_eq!(user_json["user"]["username"], "admin");

        let stored_verifier: Option<String> = sqlx::query_scalar(
            "SELECT current_verifier_hash FROM installation_auth \
             WHERE singleton_key = 'installation'",
        )
        .fetch_one(application.pool())
        .await
        .unwrap();
        assert!(
            stored_verifier.is_none(),
            "WebUI password must not be stored in the installation-token verifier"
        );
        let metadata = std::fs::read_to_string(
            directory
                .path()
                .join(V4_AUTH_METADATA_FILE),
        )
        .unwrap();
        let metadata: FreshV4AuthMetadata = serde_json::from_str(&metadata).unwrap();
        assert!(metadata.password_hash.starts_with("$2"));
        application.close().await.unwrap();
    }

    #[tokio::test]
    async fn canonical_system_and_installation_token_routes_use_the_v4_pool() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let directory = tempfile::tempdir().unwrap();
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let host = CanonicalHost::from_bootstrap(outcome);
        let config = crate::AppConfig {
            data_dir: directory.path().to_path_buf(),
            work_dir: directory.path().to_path_buf(),
            auth_policy: AuthPolicy::NoAuth,
            ..crate::AppConfig::default()
        };
        let application = host.compose(&config).await.unwrap();
        let router = application.router();

        let initial_preferences = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/settings/client")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(initial_preferences.status(), StatusCode::OK);

        let update_preferences = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings/client")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "webui.desktop.enabled": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_preferences.status(), StatusCode::OK);
        let stored_preference: String = sqlx::query_scalar(
            "SELECT value FROM client_preferences WHERE key = 'webui.desktop.enabled'",
        )
        .fetch_one(application.pool())
        .await
        .unwrap();
        assert_eq!(stored_preference, "true");

        let providers = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(providers.status(), StatusCode::OK);

        let token_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webui/access-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let token_body = axum::body::to_bytes(token_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let token_json: serde_json::Value = serde_json::from_slice(&token_body).unwrap();
        let token = token_json["data"]["token"].as_str().unwrap();
        assert!(!token.is_empty());

        let (status, hash): (String, Option<String>) = sqlx::query_as(
            "SELECT status, current_verifier_hash FROM installation_auth \
             WHERE singleton_key = 'installation'",
        )
        .fetch_one(application.pool())
        .await
        .unwrap();
        assert_eq!(status, "active");
        assert!(hash.is_some());

        let revoke = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/webui/access-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::OK);
        let (status, hash): (String, Option<String>) = sqlx::query_as(
            "SELECT status, current_verifier_hash FROM installation_auth \
             WHERE singleton_key = 'installation'",
        )
        .fetch_one(application.pool())
        .await
        .unwrap();
        assert_eq!(status, "revoked");
        assert!(hash.is_none());

        let legacy_table: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name = 'instance_access_token'",
        )
        .fetch_optional(application.pool())
        .await
        .unwrap();
        assert_eq!(legacy_table, None);
        application.close().await.unwrap();
    }

    #[tokio::test]
    async fn canonical_remote_rest_freezes_binding_and_auth_fence() {
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            canonical_remote_rest_freezes_binding_and_auth_fence_body(),
        )
        .await
        .expect("canonical Remote REST integration exceeded its 60 second test deadline");
    }

    async fn canonical_remote_rest_freezes_binding_and_auth_fence_body() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use nomifun_agent_contracts::UserId as ContractUserId;
        use nomifun_agent_control_plane::ControlPlaneStore;
        use nomifun_api_types::{
            CreateAgentPresetFromTemplateRequest, CreateRemoteBindingRequest,
            RemoteObserveResponseDto, RemoteOpenResponseDto,
        };
        use tower::ServiceExt;

        let directory = tempfile::tempdir().unwrap();
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(
                directory.path(),
                APPLICATION_BUILD_IDENTITY,
                &[],
            )
            .await
            .unwrap();
        let host = CanonicalHost::from_bootstrap(outcome);
        let config = crate::AppConfig {
            data_dir: directory.path().to_path_buf(),
            work_dir: directory.path().to_path_buf(),
            auth_policy: AuthPolicy::NoAuth,
            ..crate::AppConfig::default()
        };
        let application = host.compose(&config).await.unwrap();
        let owner = ContractUserId::from(application.owner_id().to_owned());

        let preset = application
            .platform()
            .control_plane()
            .create_from_template(
                &owner,
                "chat.minimal",
                CreateAgentPresetFromTemplateRequest {
                    display_name: "Remote smoke preset".to_owned(),
                    description: None,
                    resource_bindings: Vec::new(),
                    model_route_refs: std::collections::BTreeMap::new(),
                    chat_route_records: std::collections::BTreeMap::new(),
                },
            )
            .await
            .unwrap();
        let revision = preset.revision.unwrap();
        let revision_ref = nomifun_agent_contracts::PresetRevisionRef {
            preset_id: nomifun_agent_contracts::AgentPresetId::from(
                preset.preset.preset_id.clone(),
            ),
            revision: revision.reference.revision,
            revision_digest: nomifun_agent_contracts::DigestHex::from(
                revision.reference.revision_digest.clone(),
            ),
        };
        let snapshot = application
            .platform()
            .control_store()
            .get_snapshot(&revision_ref)
            .await
            .unwrap()
            .expect("template snapshot");
        let agent_binding = nomifun_api_types::AgentBindingValueDto {
            preset_revision_ref: revision.reference.clone(),
            resolved_snapshot_ref: serde_json::from_value(
                serde_json::to_value(snapshot.snapshot_ref).unwrap(),
            )
            .unwrap(),
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        };
        let remote_binding = application
            .platform()
            .control_plane()
            .create_remote_binding(
                &owner,
                CreateRemoteBindingRequest {
                    name: "Remote smoke binding".to_owned(),
                    agent_binding,
                },
            )
            .await
            .unwrap();

        let router = application.router();
        let token_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/webui/access-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(token_response.status(), StatusCode::OK);
        let token_body = axum::body::to_bytes(token_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let token_json: serde_json::Value = serde_json::from_slice(&token_body).unwrap();
        let token = token_json["data"]["token"]
            .as_str()
            .expect("installation token")
            .to_owned();

        let open_body = serde_json::json!({
            "binding_id": remote_binding.remote_binding_id.clone(),
            "idempotency_key": "remote-smoke-open"
        });
        let open_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/remote/open")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(open_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(open_response.status(), StatusCode::OK);
        let open_body = axum::body::to_bytes(open_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let open: RemoteOpenResponseDto = serde_json::from_slice(&open_body).unwrap();
        assert_eq!(
            open.open_state,
            nomifun_api_types::RemoteOpenStateViewDto::Opening
        );

        let observe_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/remote/observe?agent_session_id={}&after_seq=0&limit=100",
                        open.agent_session_id
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(observe_response.status(), StatusCode::OK);
        let observe_body = axum::body::to_bytes(observe_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let observe: RemoteObserveResponseDto = serde_json::from_slice(&observe_body).unwrap();
        assert!(observe
            .events
            .iter()
            .any(|event| event["kind"] == "session/opening"));

        application
            .platform()
            .control_plane()
            .delete_remote_binding(&owner, &remote_binding.remote_binding_id)
            .await
            .unwrap();
        let observe_after_delete = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/remote/observe?agent_session_id={}&after_seq=0&limit=100",
                        open.agent_session_id
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(observe_after_delete.status(), StatusCode::OK);

        let revoke = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/webui/access-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::OK);
        let rejected = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/remote/observe?agent_session_id={}&after_seq=0&limit=100",
                        open.agent_session_id
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        let rejected_body = axum::body::to_bytes(rejected.into_body(), usize::MAX)
            .await
            .unwrap();
        let rejected_json: serde_json::Value =
            serde_json::from_slice(&rejected_body).unwrap();
        assert_eq!(rejected_json["code"], "REMOTE_AUTH_REQUIRED");

        application.close().await.unwrap();
    }
}

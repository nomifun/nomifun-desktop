use std::collections::HashMap;
use std::sync::Arc;

use nomifun_api_types::{
    CreateProviderRequest, ModelHealthStatus, ProviderModelResponse, ProviderResponse,
    UpdateProviderRequest,
};
use nomifun_common::{
    AppError, ProviderId, ProviderInUseDetails, decrypt_string, encrypt_string,
};
use nomifun_db::{
    CreateProviderParams, IProviderConnectionRepository, IProviderModelRepository,
    IProviderRepository, ProviderModelRow, ProviderModelUpdate, UpdateProviderParams,
    UpsertProviderConnectionParams, models::Provider,
};
use serde::de::DeserializeOwned;

use crate::managed_model::is_managed_provider_platform;
use crate::provider_deletion::SharedProviderDeletionCoordinator;
use crate::provider_model::row_to_model_response;

/// Business logic for model provider CRUD with API key encryption/masking.
///
/// Reads project the per-model surface (`models` + the per-model maps +
/// `models_detail`) from the authoritative `provider_models` rows; the legacy
/// JSON map columns on `providers` are still dual-written but no longer read.
#[derive(Clone)]
pub struct ProviderService {
    repo: Arc<dyn IProviderRepository>,
    provider_model_repo: Arc<dyn IProviderModelRepository>,
    encryption_key: [u8; 32],
    coordinator: Option<SharedProviderDeletionCoordinator>,
}

impl ProviderService {
    pub fn new(
        repo: Arc<dyn IProviderRepository>,
        provider_model_repo: Arc<dyn IProviderModelRepository>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            repo,
            provider_model_repo,
            encryption_key,
            coordinator: None,
        }
    }

    /// Inject a deletion coordinator so `delete` returns friendly labeled
    /// conflicts before the repository transaction enforces the same logical
    /// bindings atomically.
    pub fn with_deletion_coordinator(mut self, coordinator: SharedProviderDeletionCoordinator) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    /// List all providers with masked API keys.
    pub async fn list(&self) -> Result<Vec<ProviderResponse>, AppError> {
        let rows = self.repo.list().await?;
        let model_rows = self.provider_model_repo.list().await?;
        let mut grouped: HashMap<String, Vec<ProviderModelRow>> = HashMap::new();
        for model_row in model_rows {
            grouped
                .entry(model_row.provider_id.clone())
                .or_default()
                .push(model_row);
        }
        rows.into_iter()
            .map(|row| {
                let models = grouped.remove(&row.provider_id).unwrap_or_default();
                self.row_to_response(row, models)
            })
            .collect()
    }

    /// Create a new provider. The API key is encrypted before storage.
    ///
    /// If `req.provider_id` is `Some`, the caller-supplied canonical UUIDv7 is
    /// used after strict validation; otherwise the repository generates one.
    pub async fn create(&self, req: CreateProviderRequest) -> Result<ProviderResponse, AppError> {
        reject_managed_create(&req)?;
        validate_create_request(&req)?;

        let encrypted_key = encrypt_string(&req.api_key, &self.encryption_key)?;
        let models_json = serialize_json(&req.models, "models")?;
        let capabilities_json = serialize_json(&req.capabilities, "capabilities")?;
        let model_protocols_json = serialize_opt(&req.model_protocols, "model_protocols")?;
        let model_context_limits_json = serialize_opt(&req.model_context_limits, "model_context_limits")?;
        let model_descriptions_json = serialize_opt(&req.model_descriptions, "model_descriptions")?;
        let model_enabled_json = serialize_opt(&req.model_enabled, "model_enabled")?;
        let model_health_json = serialize_opt(&req.model_health, "model_health")?;
        let bedrock_json = serialize_opt(&req.bedrock_config, "bedrock_config")?;
        let params = CreateProviderParams {
            provider_id: req.provider_id.as_deref(),
            platform: &req.platform,
            name: &req.name,
            base_url: &req.base_url,
            api_key_encrypted: &encrypted_key,
            models: &models_json,
            enabled: req.enabled,
            capabilities: &capabilities_json,
            model_context_limits: model_context_limits_json.as_deref(),
            model_protocols: model_protocols_json.as_deref(),
            model_descriptions: model_descriptions_json.as_deref(),
            model_enabled: model_enabled_json.as_deref(),
            model_health: model_health_json.as_deref(),
            bedrock_config: bedrock_json.as_deref(),
            is_full_url: req.is_full_url,
            sort_order: req.sort_order,
        };

        let row = self.repo.create(params).await?;
        let model_rows = self.provider_model_repo.list_for_provider(&row.provider_id).await?;
        self.row_to_response(row, model_rows)
    }

    /// Update an existing provider. Only provided fields are changed.
    pub async fn update(&self, id: &str, req: UpdateProviderRequest) -> Result<ProviderResponse, AppError> {
        validate_id(id)?;
        self.reject_persisted_managed_provider(id).await?;
        reject_managed_update(&req)?;
        validate_update_request(&req)?;

        let encrypted_key = req
            .api_key
            .as_deref()
            .map(|k| encrypt_string(k, &self.encryption_key))
            .transpose()?;
        let models_json = serialize_opt(&req.models, "models")?;
        let capabilities_json = serialize_opt(&req.capabilities, "capabilities")?;
        let model_protocols_json = serialize_opt(&req.model_protocols, "model_protocols")?;
        let model_context_limits_json = serialize_opt(&req.model_context_limits, "model_context_limits")?;
        let model_descriptions_json = serialize_opt(&req.model_descriptions, "model_descriptions")?;
        let model_enabled_json = serialize_opt(&req.model_enabled, "model_enabled")?;
        let model_health_json = serialize_opt(&req.model_health, "model_health")?;
        let bedrock_json = serialize_opt(&req.bedrock_config, "bedrock_config")?;

        let params = UpdateProviderParams {
            platform: req.platform.as_deref(),
            name: req.name.as_deref(),
            base_url: req.base_url.as_deref(),
            api_key_encrypted: encrypted_key.as_deref(),
            models: models_json.as_deref(),
            enabled: req.enabled,
            capabilities: capabilities_json.as_deref(),
            model_context_limits: model_context_limits_json.as_ref().map(|s| Some(s.as_str())),
            model_protocols: model_protocols_json.as_ref().map(|s| Some(s.as_str())),
            model_descriptions: model_descriptions_json.as_ref().map(|s| Some(s.as_str())),
            model_enabled: model_enabled_json.as_ref().map(|s| Some(s.as_str())),
            model_health: model_health_json.as_ref().map(|s| Some(s.as_str())),
            bedrock_config: bedrock_json.as_ref().map(|s| Some(s.as_str())),
            is_full_url: req.is_full_url,
            sort_order: req.sort_order,
        };

        let row = self.repo.update(id, params).await?;
        let model_rows = self.provider_model_repo.list_for_provider(&row.provider_id).await?;
        self.row_to_response(row, model_rows)
    }

    /// Delete a provider by ID.
    ///
    /// When a deletion coordinator is configured, deletion is refused with
    /// `AppError::ProviderInUse` if any feature still holds a hard binding to
    /// the provider. SQLite references are handled in the repository DELETE
    /// transaction; side stores are cleaned under the lifecycle write guard.
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        validate_id(id)?;
        self.reject_persisted_managed_provider(id).await?;
        let lifecycle_barrier = self
            .coordinator
            .as_ref()
            .and_then(|coord| coord.provider_lifecycle_barrier());
        let _lifecycle_guard = if let Some(barrier) = lifecycle_barrier.as_ref() {
            Some(barrier.write().await)
        } else {
            None
        };
        if let Some(coord) = &self.coordinator {
            let usages = coord.usages(id).await?;
            if !usages.is_empty() {
                return Err(AppError::ProviderInUse(ProviderInUseDetails { usages }));
            }
            coord.cleanup_soft_references(id).await?;
        }
        self.repo.delete(id).await?;
        Ok(())
    }

    /// Server-side provider clone that preserves the full per-model profile
    /// surface and connection profiles — unlike the legacy frontend clone,
    /// which copies only the provider-level fields (per-model rows keyed by
    /// the old provider_id were silently lost).
    ///
    /// - The new provider row copies platform/base_url/api_key ciphertext
    ///   (same encryption key — the source bytes are reused verbatim, no
    ///   decrypt/re-encrypt), models JSON, capabilities, legacy maps,
    ///   bedrock_config, is_full_url and enabled; `name` gets a " copy"
    ///   suffix; sort_order appends after the current max.
    /// - `provider_models` rows are copied field-for-field from the source
    ///   rows (tasks/traits/params/source/protocol/connection_role/
    ///   context_limit/description/enabled/sort_order). `health` /
    ///   `health_checked_at` are intentionally NOT copied: health is
    ///   per-deployment probe state, not configuration.
    /// - `provider_connections` rows are copied with the ciphertext as-is;
    ///   the upsert mints a fresh `connection_id` per row.
    pub async fn clone_provider(
        &self,
        id: &str,
        connection_repo: &Arc<dyn IProviderConnectionRepository>,
    ) -> Result<ProviderResponse, AppError> {
        validate_id(id)?;
        let source = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provider {id} not found")))?;
        if is_managed_provider_platform(&source.platform) {
            return Err(AppError::Forbidden(
                "Managed model providers must be changed through their dedicated model-service API"
                    .into(),
            ));
        }

        let source_model_rows = self.provider_model_repo.list_for_provider(id).await?;
        let source_connections = connection_repo.list_for_provider(id).await?;

        let clone_name = format!("{} copy", source.name.trim_end());
        let created = self
            .repo
            .create(CreateProviderParams {
                provider_id: None,
                platform: &source.platform,
                name: &clone_name,
                base_url: &source.base_url,
                // Ciphertext copy: same encryption key, so re-encrypting would
                // only mint a fresh nonce for identical plaintext.
                api_key_encrypted: &source.api_key_encrypted,
                models: &source.models,
                enabled: source.enabled,
                capabilities: &source.capabilities,
                model_context_limits: source.model_context_limits.as_deref(),
                model_protocols: source.model_protocols.as_deref(),
                model_descriptions: source.model_descriptions.as_deref(),
                model_enabled: source.model_enabled.as_deref(),
                // Health is per-deployment state; keep it out of the clone's
                // legacy column AND the dual-written rows.
                model_health: None,
                bedrock_config: source.bedrock_config.as_deref(),
                is_full_url: source.is_full_url,
                sort_order: None,
            })
            .await?;
        let new_id = created.provider_id.clone();

        // The repo create's dual-write materialized rows for every model in
        // the legacy array — but with placeholder profiles (tasks/traits '[]',
        // params '{}', source 'inferred'). Overwrite each from the
        // authoritative source row; a source row absent from the legacy array
        // (created via /api/provider-models, which does not write the legacy
        // column back) is inserted instead.
        for row in &source_model_rows {
            let update = ProviderModelUpdate {
                enabled: Some(row.enabled),
                sort_order: Some(row.sort_order),
                tasks: Some(&row.tasks),
                traits: Some(&row.traits),
                protocol: Some(row.protocol.as_deref()),
                connection_role: Some(row.connection_role.as_deref()),
                params: Some(&row.params),
                context_limit: Some(row.context_limit),
                description: Some(row.description.as_deref()),
                source: Some(&row.source),
            };
            match self.provider_model_repo.update(&new_id, &row.model, &update).await {
                Ok(_) => {}
                Err(nomifun_db::DbError::NotFound(_)) => {
                    self.provider_model_repo
                        .create(
                            &new_id,
                            &nomifun_db::NewProviderModel {
                                model: &row.model,
                                enabled: row.enabled,
                                sort_order: row.sort_order,
                                tasks: &row.tasks,
                                traits: &row.traits,
                                protocol: row.protocol.as_deref(),
                                params: &row.params,
                                context_limit: row.context_limit,
                                description: row.description.as_deref(),
                                source: &row.source,
                                health: None,
                            },
                        )
                        .await?;
                    if row.connection_role.is_some() {
                        self.provider_model_repo
                            .update(
                                &new_id,
                                &row.model,
                                &ProviderModelUpdate {
                                    connection_role: Some(row.connection_role.as_deref()),
                                    ..Default::default()
                                },
                            )
                            .await?;
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }

        // Copy connection profiles; upsert mints a fresh connection_id per
        // row and the credentials ciphertext crosses unchanged.
        for connection in &source_connections {
            connection_repo
                .upsert(
                    &new_id,
                    &UpsertProviderConnectionParams {
                        role: &connection.role,
                        label: connection.label.as_deref(),
                        base_url: &connection.base_url,
                        auth_scheme: &connection.auth_scheme,
                        credentials_encrypted: &connection.credentials_encrypted,
                        is_full_url: connection.is_full_url,
                        extra: &connection.extra,
                    },
                )
                .await?;
        }

        let model_rows = self.provider_model_repo.list_for_provider(&new_id).await?;
        self.row_to_response(created, model_rows)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    async fn reject_persisted_managed_provider(&self, id: &str) -> Result<(), AppError> {
        if let Some(row) = self.repo.find_by_id(id).await?
            && is_managed_provider_platform(&row.platform)
        {
            return Err(AppError::Forbidden(
                "Managed model providers must be changed through their dedicated model-service API"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Convert a DB row into the v3 response DTO with the plaintext API key
    /// (decrypted) and deserialized JSON fields. The named `provider_id`
    /// business identity crosses the wire; the SQLite technical `id` does not.
    ///
    /// The per-model surface is projected from `provider_models` rows (the
    /// authoritative entity), NOT from the legacy JSON map columns on the
    /// providers row:
    /// - `models` = row `model` names ordered by `(sort_order, id)`;
    /// - `model_enabled` holds an entry only for disabled rows (absent =
    ///   enabled, matching the legacy readers' `!= false` semantics);
    /// - `model_protocols`/`model_context_limits`/`model_descriptions` hold an
    ///   entry when the row field is non-NULL;
    /// - `model_health` holds an entry when the row's health JSON parses;
    /// - every map is `None` when empty, preserving the legacy
    ///   `skip_serializing_if` wire shape;
    /// - `models_detail` = all rows, fully projected.
    fn row_to_response(
        &self,
        row: Provider,
        mut model_rows: Vec<ProviderModelRow>,
    ) -> Result<ProviderResponse, AppError> {
        ProviderId::parse(&row.provider_id).map_err(|error| {
            AppError::Internal(format!(
                "stored providers.provider_id '{}' is not canonical: {error}",
                row.provider_id
            ))
        })?;
        let managed = is_managed_provider_platform(&row.platform);
        let api_key = if managed {
            String::new()
        } else {
            decrypt_string(&row.api_key_encrypted, &self.encryption_key)?
        };

        let capabilities = serde_json::from_str(&row.capabilities)
            .map_err(|e| AppError::Internal(format!("Failed to parse capabilities JSON: {e}")))?;
        let bedrock_config = deserialize_opt(&row.bedrock_config, "bedrock_config")?;

        model_rows.sort_by(|a, b| (a.sort_order, a.id).cmp(&(b.sort_order, b.id)));

        let models: Vec<String> = model_rows.iter().map(|m| m.model.clone()).collect();
        let mut model_enabled: HashMap<String, bool> = HashMap::new();
        let mut model_protocols: HashMap<String, String> = HashMap::new();
        let mut model_context_limits: HashMap<String, i64> = HashMap::new();
        let mut model_descriptions: HashMap<String, String> = HashMap::new();
        let mut model_health: HashMap<String, ModelHealthStatus> = HashMap::new();
        for model_row in &model_rows {
            if !model_row.enabled {
                model_enabled.insert(model_row.model.clone(), false);
            }
            if let Some(protocol) = &model_row.protocol {
                model_protocols.insert(model_row.model.clone(), protocol.clone());
            }
            if let Some(limit) = model_row.context_limit {
                model_context_limits.insert(model_row.model.clone(), limit);
            }
            if let Some(description) = &model_row.description {
                model_descriptions.insert(model_row.model.clone(), description.clone());
            }
            if let Some(health_json) = &model_row.health {
                match serde_json::from_str::<ModelHealthStatus>(health_json) {
                    Ok(health) => {
                        model_health.insert(model_row.model.clone(), health);
                    }
                    Err(error) => {
                        tracing::warn!(
                            provider_id = %model_row.provider_id,
                            model = %model_row.model,
                            %error,
                            "invalid provider_models.health JSON; dropping model_health entry"
                        );
                    }
                }
            }
        }

        let models_detail: Vec<ProviderModelResponse> = model_rows
            .into_iter()
            .map(row_to_model_response)
            .collect::<Result<_, _>>()?;

        Ok(ProviderResponse {
            provider_id: row.provider_id,
            platform: row.platform,
            name: row.name,
            base_url: row.base_url,
            api_key,
            models,
            enabled: row.enabled,
            capabilities,
            model_context_limits: (!model_context_limits.is_empty()).then_some(model_context_limits),
            model_protocols: (!model_protocols.is_empty()).then_some(model_protocols),
            model_descriptions: (!model_descriptions.is_empty()).then_some(model_descriptions),
            model_enabled: (!model_enabled.is_empty()).then_some(model_enabled),
            model_health: (!model_health.is_empty()).then_some(model_health),
            bedrock_config,
            models_detail,
            is_full_url: row.is_full_url,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

// ---------------------------------------------------------------------------
// JSON helpers (M-1 / M-2 refactor)
// ---------------------------------------------------------------------------

/// Serialize an optional value to JSON string.
fn serialize_opt<T: serde::Serialize>(val: &Option<T>, field: &str) -> Result<Option<String>, AppError> {
    val.as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Internal(format!("Failed to serialize {field}: {e}")))
}

/// Serialize a value to JSON string.
fn serialize_json<T: serde::Serialize>(val: &T, field: &str) -> Result<String, AppError> {
    serde_json::to_string(val).map_err(|e| AppError::Internal(format!("Failed to serialize {field}: {e}")))
}

/// Deserialize an optional JSON string into a typed value.
pub(crate) fn deserialize_opt<T: DeserializeOwned>(json: &Option<String>, field: &str) -> Result<Option<T>, AppError> {
    json.as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| AppError::Internal(format!("Failed to parse {field} JSON: {e}")))
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_create_request(req: &CreateProviderRequest) -> Result<(), AppError> {
    if let Some(ref id) = req.provider_id {
        validate_id(id)?;
    }
    if req.platform.trim().is_empty() {
        return Err(AppError::BadRequest("platform is required".into()));
    }
    if req.name.trim().is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    validate_sort_order(req.sort_order)?;
    // Bedrock auths via bedrock_config (IAM profile / static keys) rather than
    // an HTTP endpoint + bearer key, so baseUrl and apiKey may be empty.
    if req.platform == "bedrock" {
        if req.bedrock_config.is_none() {
            return Err(AppError::BadRequest(
                "bedrockConfig is required for bedrock platform".into(),
            ));
        }
        if !req.base_url.trim().is_empty() {
            validate_base_url(&req.base_url)?;
        }
    } else {
        validate_base_url(&req.base_url)?;
        if req.api_key.trim().is_empty() {
            return Err(AppError::BadRequest("apiKey is required".into()));
        }
    }
    Ok(())
}

fn reject_managed_create(req: &CreateProviderRequest) -> Result<(), AppError> {
    if is_managed_provider_platform(req.platform.trim()) {
        return Err(AppError::Forbidden(
            "Managed model providers are created and configured by their dedicated model-service API"
                .into(),
        ));
    }
    Ok(())
}

fn reject_managed_update(req: &UpdateProviderRequest) -> Result<(), AppError> {
    if req
        .platform
        .as_deref()
        .map(str::trim)
        .is_some_and(is_managed_provider_platform)
    {
        return Err(AppError::Forbidden(
            "Managed model providers must be changed through their dedicated model-service API"
                .into(),
        ));
    }
    Ok(())
}

/// Validate a caller-supplied provider id.
///
fn validate_id(id: &str) -> Result<(), AppError> {
    ProviderId::parse(id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))
}

fn validate_update_request(req: &UpdateProviderRequest) -> Result<(), AppError> {
    validate_sort_order(req.sort_order)?;
    if let Some(ref platform) = req.platform
        && platform.trim().is_empty()
    {
        return Err(AppError::BadRequest("platform cannot be empty".into()));
    }
    if let Some(ref name) = req.name
        && name.trim().is_empty()
    {
        return Err(AppError::BadRequest("name cannot be empty".into()));
    }
    if let Some(ref url) = req.base_url
        && !url.trim().is_empty()
    {
        validate_base_url(url)?;
    }
    Ok(())
}

fn validate_sort_order(sort_order: Option<i64>) -> Result<(), AppError> {
    if sort_order.is_some_and(|value| value < 0) {
        return Err(AppError::BadRequest("sort_order must be non-negative".into()));
    }
    Ok(())
}

/// Shared base-url validation for provider-level and per-connection endpoints.
pub(crate) fn validate_base_url(url: &str) -> Result<(), AppError> {
    if url.trim().is_empty() {
        return Err(AppError::BadRequest("baseUrl is required".into()));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::BadRequest(
            "baseUrl must start with http:// or https://".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_db::{SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory};

    // A fixed 32-byte key for testing
    const TEST_KEY: [u8; 32] = [0x42; 32];

    fn service_for_pool(pool: &nomifun_db::SqlitePool) -> ProviderService {
        ProviderService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            TEST_KEY,
        )
    }

    async fn setup_with_pool() -> (ProviderService, nomifun_db::SqlitePool) {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        std::mem::forget(db);
        (service_for_pool(&pool), pool)
    }

    async fn setup() -> ProviderService {
        setup_with_pool().await.0
    }

    fn sample_create_request() -> CreateProviderRequest {
        CreateProviderRequest {
            provider_id: None,
            platform: "anthropic".into(),
            name: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            api_key: "sk-ant-api03-test1234".into(),
            models: vec!["claude-sonnet-4-20250514".into()],
            enabled: true,
            capabilities: vec![],
            model_context_limits: None,
            model_protocols: None,
            model_descriptions: None,
            model_enabled: None,
            model_health: None,
            bedrock_config: None,
            is_full_url: false,
            sort_order: None,
        }
    }

    // -- id validation tests --

    #[test]
    fn validate_id_accepts_canonical_provider_uuid_v7() {
        assert!(
            validate_id("0190f5fe-7c00-7a00-8abc-012345678901").is_ok()
        );
    }

    #[test]
    fn validate_id_rejects_uuid_v4() {
        assert!(validate_id("11111111-1111-4111-8111-111111111111").is_err());
    }

    #[test]
    fn validate_id_rejects_legacy_short_hex() {
        assert!(validate_id("a1b2c3d4").is_err());
    }

    #[test]
    fn validate_id_rejects_empty() {
        assert!(validate_id("").is_err());
        assert!(validate_id("   ").is_err());
    }

    #[test]
    fn validate_id_rejects_wrong_prefix_or_noncanonical_characters() {
        assert!(
            validate_id("provider_0190f5fe-7c00-7a00-8abc-012345678901").is_err()
        );
        assert!(validate_id("0190F5FE-7C00-7A00-8ABC-012345678901").is_err());
        assert!(validate_id("bad/slash").is_err());
    }

    // -- validation tests --

    #[test]
    fn validate_create_missing_platform() {
        let req = CreateProviderRequest {
            platform: "".into(),
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_err());
    }

    #[test]
    fn validate_create_missing_name() {
        let req = CreateProviderRequest {
            name: "  ".into(),
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_err());
    }

    #[test]
    fn validate_create_missing_base_url() {
        let req = CreateProviderRequest {
            base_url: "".into(),
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_err());
    }

    #[test]
    fn validate_create_invalid_url() {
        let req = CreateProviderRequest {
            base_url: "not-a-url".into(),
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_err());
    }

    #[test]
    fn validate_create_missing_api_key() {
        let req = CreateProviderRequest {
            api_key: "  ".into(),
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_err());
    }

    #[test]
    fn validate_create_valid() {
        assert!(validate_create_request(&sample_create_request()).is_ok());
    }

    #[test]
    fn generic_create_rejects_managed_platform() {
        let by_id = CreateProviderRequest {
            provider_id: Some(nomifun_common::ProviderId::new().into_string()),
            ..sample_create_request()
        };
        assert!(reject_managed_create(&by_id).is_ok());

        let by_platform = CreateProviderRequest {
            platform: crate::managed_model::FREE_MODEL_PLATFORM.into(),
            ..sample_create_request()
        };
        assert!(matches!(
            reject_managed_create(&by_platform),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn generic_update_rejects_managed_platform() {
        assert!(matches!(
            reject_managed_update(&UpdateProviderRequest {
                platform: Some(crate::managed_model::FREE_MODEL_PLATFORM.into()),
                ..Default::default()
            }),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn validate_create_bedrock_allows_empty_base_url_and_api_key() {
        let req = CreateProviderRequest {
            platform: "bedrock".into(),
            name: "AWS Bedrock".into(),
            base_url: "".into(),
            api_key: "".into(),
            bedrock_config: Some(nomifun_api_types::BedrockConfig {
                auth_method: nomifun_api_types::BedrockAuthMethod::Profile,
                region: "us-west-2".into(),
                profile: Some("ai".into()),
                access_key_id: None,
                secret_access_key: None,
            }),
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_ok());
    }

    #[test]
    fn validate_create_bedrock_requires_bedrock_config() {
        let req = CreateProviderRequest {
            platform: "bedrock".into(),
            name: "AWS Bedrock".into(),
            base_url: "".into(),
            api_key: "".into(),
            bedrock_config: None,
            ..sample_create_request()
        };
        assert!(validate_create_request(&req).is_err());
    }

    #[test]
    fn validate_update_empty_name_rejected() {
        let req = UpdateProviderRequest {
            name: Some("".into()),
            ..Default::default()
        };
        assert!(validate_update_request(&req).is_err());
    }

    #[test]
    fn validate_update_empty_request_ok() {
        assert!(validate_update_request(&UpdateProviderRequest::default()).is_ok());
    }

    #[test]
    fn validate_update_empty_base_url_ok() {
        let req = UpdateProviderRequest {
            base_url: Some("".into()),
            ..Default::default()
        };
        assert!(validate_update_request(&req).is_ok());
    }

    #[test]
    fn validate_update_invalid_base_url_rejected() {
        let req = UpdateProviderRequest {
            base_url: Some("not-a-url".into()),
            ..Default::default()
        };
        assert!(validate_update_request(&req).is_err());
    }

    #[test]
    fn validate_base_url_http() {
        assert!(validate_base_url("http://localhost:8080").is_ok());
    }

    #[test]
    fn validate_base_url_https() {
        assert!(validate_base_url("https://api.example.com").is_ok());
    }

    #[test]
    fn validate_base_url_ftp_rejected() {
        assert!(validate_base_url("ftp://files.example.com").is_err());
    }

    // -- service integration tests --

    #[tokio::test]
    async fn list_empty() {
        let svc = setup().await;
        let result = svc.list().await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn create_and_list() {
        let svc = setup().await;
        let created = svc.create(sample_create_request()).await.unwrap();

        assert!(ProviderId::parse(&created.provider_id).is_ok());
        assert_eq!(created.platform, "anthropic");
        assert_eq!(created.name, "Anthropic");
        assert_eq!(created.base_url, "https://api.anthropic.com");
        // API key is returned in plaintext (pre-launch; encrypted at rest).
        assert_eq!(created.api_key, "sk-ant-api03-test1234");
        assert_eq!(created.models, vec!["claude-sonnet-4-20250514"]);
        assert!(created.enabled);

        let all = svc.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].provider_id, created.provider_id);
        assert_eq!(all[0].api_key, "sk-ant-api03-test1234");
    }

    // -- row projection tests (reads come from provider_models rows) --

    #[tokio::test]
    async fn list_projects_models_and_maps_from_provider_model_rows() {
        use std::collections::HashMap;
        let (svc, _pool) = setup_with_pool().await;
        let req = CreateProviderRequest {
            models: vec!["m1".into(), "m2".into(), "m3".into()],
            model_enabled: Some(HashMap::from([("m2".into(), false)])),
            model_protocols: Some(HashMap::from([("m1".into(), "openai".into())])),
            model_context_limits: Some(HashMap::from([("m1".into(), 32_000)])),
            model_descriptions: Some(HashMap::from([("m3".into(), "描述".into())])),
            model_health: Some(HashMap::from([(
                "m1".into(),
                nomifun_api_types::ModelHealthStatus {
                    status: nomifun_api_types::HealthStatus::Healthy,
                    last_check: Some(11),
                    latency: Some(22),
                    error: None,
                },
            )])),
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();

        let all = svc.list().await.unwrap();
        assert_eq!(all.len(), 1);
        let provider = &all[0];

        // models keep the creation (sort_order) order.
        assert_eq!(provider.models, vec!["m1", "m2", "m3"]);

        // model_enabled contains only explicit-false entries (absent = enabled).
        assert_eq!(
            provider.model_enabled,
            Some(HashMap::from([("m2".to_string(), false)]))
        );
        assert_eq!(
            provider.model_protocols,
            Some(HashMap::from([("m1".to_string(), "openai".to_string())]))
        );
        assert_eq!(
            provider.model_context_limits,
            Some(HashMap::from([("m1".to_string(), 32_000)]))
        );
        assert_eq!(
            provider.model_descriptions,
            Some(HashMap::from([("m3".to_string(), "描述".to_string())]))
        );
        let health = provider.model_health.as_ref().unwrap();
        assert_eq!(health.len(), 1);
        assert_eq!(health["m1"].status, nomifun_api_types::HealthStatus::Healthy);
        assert_eq!(health["m1"].latency, Some(22));

        // models_detail mirrors all rows, in order, consistent with the maps.
        let detail = &provider.models_detail;
        assert_eq!(detail.len(), 3);
        assert_eq!(
            detail.iter().map(|d| d.model.as_str()).collect::<Vec<_>>(),
            vec!["m1", "m2", "m3"]
        );
        assert!(detail.iter().all(|d| d.provider_id == created.provider_id));
        assert!(detail[0].enabled);
        assert!(!detail[1].enabled);
        assert!(detail[2].enabled);
        assert_eq!(detail[0].protocol.as_deref(), Some("openai"));
        assert_eq!(detail[0].context_limit, Some(32_000));
        assert_eq!(detail[2].description.as_deref(), Some("描述"));
        assert_eq!(
            detail[0].health.as_ref().map(|h| h.status),
            Some(nomifun_api_types::HealthStatus::Healthy)
        );
        assert!(detail[1].health.is_none());
    }

    #[tokio::test]
    async fn list_reads_from_rows_not_legacy_map_columns() {
        use std::collections::HashMap;
        let (svc, pool) = setup_with_pool().await;
        let req = CreateProviderRequest {
            models: vec!["m1".into(), "m2".into()],
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();
        // Baseline: everything enabled, no legacy map entries.
        assert!(svc.list().await.unwrap()[0].model_enabled.is_none());

        // Flip one provider_models row directly, bypassing the repository's
        // dual-write, so the legacy providers.model_enabled column still says
        // "all enabled". A row-projected read must reflect the row.
        nomifun_db::sqlx::query(
            "UPDATE provider_models SET enabled = 0 WHERE provider_id = ? AND model = 'm2'",
        )
        .bind(&created.provider_id)
        .execute(&pool)
        .await
        .unwrap();

        let provider = svc.list().await.unwrap().remove(0);
        assert_eq!(
            provider.model_enabled,
            Some(HashMap::from([("m2".to_string(), false)])),
            "list() must read enabled from provider_models rows, not the legacy column"
        );
        assert_eq!(provider.models, vec!["m1", "m2"]);
        let m2 = provider
            .models_detail
            .iter()
            .find(|d| d.model == "m2")
            .unwrap();
        assert!(!m2.enabled);

        // Direct row reorder is reflected in models order too.
        nomifun_db::sqlx::query(
            "UPDATE provider_models SET sort_order = 99 WHERE provider_id = ? AND model = 'm1'",
        )
        .bind(&created.provider_id)
        .execute(&pool)
        .await
        .unwrap();
        let provider = svc.list().await.unwrap().remove(0);
        assert_eq!(provider.models, vec!["m2", "m1"]);
        assert_eq!(
            provider
                .models_detail
                .iter()
                .map(|d| d.model.as_str())
                .collect::<Vec<_>>(),
            vec!["m2", "m1"]
        );
    }

    #[tokio::test]
    async fn provider_without_model_rows_projects_empty_surface() {
        let (svc, _pool) = setup_with_pool().await;
        let req = CreateProviderRequest {
            models: vec![],
            ..sample_create_request()
        };
        svc.create(req).await.unwrap();
        let provider = svc.list().await.unwrap().remove(0);
        assert!(provider.models.is_empty());
        assert!(provider.models_detail.is_empty());
        assert!(provider.model_enabled.is_none());
        assert!(provider.model_protocols.is_none());
        assert!(provider.model_context_limits.is_none());
        assert!(provider.model_descriptions.is_none());
        assert!(provider.model_health.is_none());
    }

    #[tokio::test]
    async fn corrupt_health_row_degrades_without_killing_list() {
        let (svc, pool) = setup_with_pool().await;
        let created = svc.create(sample_create_request()).await.unwrap();
        nomifun_db::sqlx::query(
            "UPDATE provider_models SET health = 'not-json' WHERE provider_id = ?",
        )
        .bind(&created.provider_id)
        .execute(&pool)
        .await
        .unwrap();
        let provider = svc.list().await.unwrap().remove(0);
        assert!(provider.model_health.is_none());
        assert_eq!(provider.models_detail.len(), 1);
        assert!(provider.models_detail[0].health.is_none());
    }

    #[tokio::test]
    async fn create_with_provided_id() {
        let svc = setup().await;
        let provider_id = ProviderId::new().into_string();
        let req = CreateProviderRequest {
            provider_id: Some(provider_id.clone()),
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();
        assert_eq!(created.provider_id, provider_id);
    }

    #[tokio::test]
    async fn create_with_provided_id_rejects_invalid() {
        let svc = setup().await;
        let req = CreateProviderRequest {
            provider_id: Some("   ".into()),
            ..sample_create_request()
        };
        let err = svc.create(req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_with_duplicate_id_returns_conflict() {
        let svc = setup().await;
        let provider_id = ProviderId::new().into_string();
        let req1 = CreateProviderRequest {
            provider_id: Some(provider_id.clone()),
            ..sample_create_request()
        };
        svc.create(req1).await.unwrap();

        let req2 = CreateProviderRequest {
            provider_id: Some(provider_id),
            ..sample_create_request()
        };
        let err = svc.create(req2).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn create_persists_per_model_fields() {
        use std::collections::HashMap;
        let svc = setup().await;
        let req = CreateProviderRequest {
            models: vec!["gpt-4".into(), "gpt-3.5".into()],
            model_protocols: Some(HashMap::from([("gpt-4".into(), "openai".into())])),
            model_enabled: Some(HashMap::from([("gpt-4".into(), true), ("gpt-3.5".into(), false)])),
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();

        assert_eq!(
            created.model_protocols.as_ref().and_then(|m| m.get("gpt-4")),
            Some(&"openai".to_string())
        );
        // Row projection surfaces only explicit-false entries; an enabled
        // model is absent from the map (absent = enabled for all readers).
        assert_eq!(created.model_enabled.as_ref().and_then(|m| m.get("gpt-4")), None);
        assert_eq!(
            created.model_enabled.as_ref().and_then(|m| m.get("gpt-3.5")),
            Some(&false)
        );

        // And persist through a fresh read.
        let all = svc.list().await.unwrap();
        assert_eq!(all[0].model_enabled.as_ref().and_then(|m| m.get("gpt-3.5")), Some(&false));
        assert_eq!(all[0].model_enabled.as_ref().and_then(|m| m.get("gpt-4")), None);
    }

    #[tokio::test]
    async fn create_and_update_round_trips_model_descriptions() {
        use std::collections::HashMap;
        let svc = setup().await;

        // create with a model description map
        let req = CreateProviderRequest {
            models: vec!["m1".into()],
            model_descriptions: Some(HashMap::from([("m1".into(), "擅长前端".into())])),
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();
        assert_eq!(
            created.model_descriptions.as_ref().and_then(|m| m.get("m1")),
            Some(&"擅长前端".to_string())
        );

        // persists through a fresh read (row_to_response decode path)
        let all = svc.list().await.unwrap();
        assert_eq!(
            all[0].model_descriptions.as_ref().and_then(|m| m.get("m1")),
            Some(&"擅长前端".to_string())
        );

        // update changes the description
        let updated = svc
            .update(
                &created.provider_id,
                UpdateProviderRequest {
                    model_descriptions: Some(HashMap::from([("m1".into(), "擅长后端".into())])),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            updated.model_descriptions.as_ref().and_then(|m| m.get("m1")),
            Some(&"擅长后端".to_string())
        );
    }

    #[tokio::test]
    async fn create_and_update_round_trips_model_context_limits() {
        use std::collections::HashMap;
        let svc = setup().await;

        let req = CreateProviderRequest {
            models: vec!["m1".into(), "m2".into()],
            model_context_limits: Some(HashMap::from([("m1".into(), 32_000), ("m2".into(), 128_000)])),
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();
        assert_eq!(
            created.model_context_limits.as_ref().and_then(|m| m.get("m2")),
            Some(&128_000)
        );

        let all = svc.list().await.unwrap();
        assert_eq!(
            all[0].model_context_limits.as_ref().and_then(|m| m.get("m1")),
            Some(&32_000)
        );

        let updated = svc
            .update(
                &created.provider_id,
                UpdateProviderRequest {
                    model_context_limits: Some(HashMap::from([("m2".into(), 200_000)])),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            updated.model_context_limits.as_ref().and_then(|m| m.get("m2")),
            Some(&200_000)
        );
        assert!(updated.model_context_limits.as_ref().and_then(|m| m.get("m1")).is_none());

        let cleared = svc
            .update(
                &created.provider_id,
                UpdateProviderRequest {
                    model_context_limits: Some(HashMap::new()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(cleared.model_context_limits.is_none());
    }

    #[tokio::test]
    async fn provider_response_api_key_plaintext_matches_input() {
        // Replaces the masking test: api_key on the response is the
        // encrypted-then-decrypted plaintext (equal to the input).
        let svc = setup().await;
        let req = CreateProviderRequest {
            api_key: "sk-secret-original-value".into(),
            ..sample_create_request()
        };
        let created = svc.create(req).await.unwrap();
        assert_eq!(created.api_key, "sk-secret-original-value");
        assert!(!created.api_key.contains("***"));
    }

    #[tokio::test]
    async fn managed_provider_secret_is_redacted_from_generic_list() {
        let db = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let encrypted = encrypt_string("internal-loopback-token", &TEST_KEY).unwrap();
        let provider_id = nomifun_common::ProviderId::new().into_string();
        repo.create(CreateProviderParams {
            provider_id: Some(&provider_id),
            platform: crate::managed_model::FREE_MODEL_PLATFORM,
            name: "NomiFun Free Model",
            base_url: "http://127.0.0.1:12345/v1",
            api_key_encrypted: &encrypted,
            models: r#"["big-pickle"]"#,
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
        let svc = service_for_pool(db.pool());
        let providers = svc.list().await.unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].provider_id, provider_id);
        assert!(providers[0].api_key.is_empty());
    }

    #[tokio::test]
    async fn persisted_managed_platform_is_protected_by_update_and_delete() {
        let db = init_database_memory().await.unwrap();
        let repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
        let encrypted = encrypt_string("internal-loopback-token", &TEST_KEY).unwrap();
        let provider_id = nomifun_common::ProviderId::new().into_string();
        repo.create(CreateProviderParams {
            provider_id: Some(&provider_id),
            platform: crate::managed_model::FREE_MODEL_PLATFORM,
            name: "Managed provider",
            base_url: "http://127.0.0.1:12345/v1",
            api_key_encrypted: &encrypted,
            models: r#"["big-pickle"]"#,
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
        let svc = service_for_pool(db.pool());

        assert!(matches!(
            svc.update(
                &provider_id,
                UpdateProviderRequest {
                    name: Some("changed".into()),
                    ..Default::default()
                }
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            svc.delete(&provider_id).await,
            Err(AppError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn create_invalid_request_rejected() {
        let svc = setup().await;
        let req = CreateProviderRequest {
            platform: "".into(),
            ..sample_create_request()
        };
        let err = svc.create(req).await.unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_name() {
        let svc = setup().await;
        let created = svc.create(sample_create_request()).await.unwrap();

        let updated = svc
            .update(
                &created.provider_id,
                UpdateProviderRequest {
                    name: Some("New Name".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.platform, "anthropic");
    }

    #[tokio::test]
    async fn update_api_key_re_encrypts() {
        let svc = setup().await;
        let created = svc.create(sample_create_request()).await.unwrap();

        let updated = svc
            .update(
                &created.provider_id,
                UpdateProviderRequest {
                    api_key: Some("new-key-abcdefgh".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Response carries the new plaintext key (encrypted at rest).
        assert_eq!(updated.api_key, "new-key-abcdefgh");
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let svc = setup().await;
        let err = svc
            .update(
                "0190f5fe-7c00-7a00-8000-000000000099",
                UpdateProviderRequest::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_existing() {
        let svc = setup().await;
        let created = svc.create(sample_create_request()).await.unwrap();

        svc.delete(&created.provider_id).await.unwrap();
        let all = svc.list().await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let svc = setup().await;
        let err = svc
            .delete("0190f5fe-7c00-7a00-8000-000000000099")
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::NOT_FOUND);
    }

    // -- clone tests --

    use nomifun_db::{
        IProviderConnectionRepository, NewProviderModel, ProviderModelUpdate,
        SqliteProviderConnectionRepository, UpsertProviderConnectionParams,
    };

    fn model_repo_for_pool(pool: &nomifun_db::SqlitePool) -> Arc<dyn IProviderModelRepository> {
        Arc::new(SqliteProviderModelRepository::new(pool.clone()))
    }

    fn connection_repo_for_pool(
        pool: &nomifun_db::SqlitePool,
    ) -> Arc<dyn IProviderConnectionRepository> {
        Arc::new(SqliteProviderConnectionRepository::new(pool.clone()))
    }

    #[tokio::test]
    async fn clone_copies_model_rows_exactly_without_health() {
        use std::collections::HashMap;
        let (svc, pool) = setup_with_pool().await;
        let model_repo = model_repo_for_pool(&pool);
        let connection_repo = connection_repo_for_pool(&pool);

        let created = svc
            .create(CreateProviderRequest {
                models: vec!["m1".into(), "m2".into()],
                model_protocols: Some(HashMap::from([("m1".into(), "openai".into())])),
                model_context_limits: Some(HashMap::from([("m1".into(), 32_000)])),
                model_descriptions: Some(HashMap::from([("m2".into(), "描述".into())])),
                model_enabled: Some(HashMap::from([("m2".into(), false)])),
                ..sample_create_request()
            })
            .await
            .unwrap();

        // Enrich the authoritative rows with profile fields the legacy create
        // params cannot express — exactly what the frontend clone loses.
        model_repo
            .update(
                &created.provider_id,
                "m1",
                &ProviderModelUpdate {
                    tasks: Some(r#"["chat"]"#),
                    traits: Some(r#"["vision_input","function_calling"]"#),
                    params: Some(r#"{"temperature":0.2}"#),
                    source: Some("user"),
                    connection_role: Some(Some("voice")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Health present on the source must NOT be copied to the clone.
        model_repo
            .set_health(
                &created.provider_id,
                "m1",
                Some(r#"{"status":"healthy","latency":22}"#),
            )
            .await
            .unwrap();
        // A row that exists only in provider_models (never written back to the
        // legacy models column) must survive the clone too.
        model_repo
            .create(
                &created.provider_id,
                &NewProviderModel {
                    model: "m3-row-only",
                    enabled: true,
                    sort_order: 7,
                    tasks: r#"["embedding"]"#,
                    traits: "[]",
                    protocol: None,
                    params: "{}",
                    context_limit: Some(8_192),
                    description: Some("row only"),
                    source: "user",
                    health: None,
                },
            )
            .await
            .unwrap();

        let clone = svc
            .clone_provider(&created.provider_id, &connection_repo)
            .await
            .unwrap();
        assert_ne!(clone.provider_id, created.provider_id);
        assert_eq!(clone.name, "Anthropic copy");
        assert_eq!(clone.platform, "anthropic");
        assert_eq!(clone.api_key, "sk-ant-api03-test1234");
        assert_eq!(clone.models, vec!["m1", "m2", "m3-row-only"]);

        let mut source_rows = model_repo.list_for_provider(&created.provider_id).await.unwrap();
        let mut clone_rows = model_repo.list_for_provider(&clone.provider_id).await.unwrap();
        source_rows.sort_by(|a, b| a.model.cmp(&b.model));
        clone_rows.sort_by(|a, b| a.model.cmp(&b.model));
        assert_eq!(source_rows.len(), 3);
        assert_eq!(clone_rows.len(), 3);
        for (source, cloned) in source_rows.iter().zip(&clone_rows) {
            assert_eq!(cloned.provider_id, clone.provider_id);
            assert_eq!(cloned.model, source.model);
            assert_eq!(cloned.enabled, source.enabled);
            assert_eq!(cloned.sort_order, source.sort_order);
            assert_eq!(cloned.tasks, source.tasks);
            assert_eq!(cloned.traits, source.traits);
            assert_eq!(cloned.protocol, source.protocol);
            assert_eq!(cloned.connection_role, source.connection_role);
            assert_eq!(cloned.params, source.params);
            assert_eq!(cloned.context_limit, source.context_limit);
            assert_eq!(cloned.description, source.description);
            assert_eq!(cloned.source, source.source);
            assert!(
                cloned.health.is_none(),
                "health is per-deployment probe state and must not be cloned ({})",
                cloned.model
            );
            assert!(cloned.health_checked_at.is_none());
        }
        // Sanity: the source m1 row really carried health, so the None above
        // proves the clone dropped it rather than there being nothing to drop.
        let source_m1 = source_rows.iter().find(|r| r.model == "m1").unwrap();
        assert!(source_m1.health.is_some());
    }

    #[tokio::test]
    async fn clone_copies_connections_with_new_ids_and_same_ciphertext() {
        let (svc, pool) = setup_with_pool().await;
        let connection_repo = connection_repo_for_pool(&pool);
        let created = svc.create(sample_create_request()).await.unwrap();
        connection_repo
            .upsert(
                &created.provider_id,
                &UpsertProviderConnectionParams {
                    role: "voice",
                    label: Some("Voice endpoint"),
                    base_url: "https://voice.example.com/v1",
                    auth_scheme: "bearer",
                    credentials_encrypted: "opaque-ciphertext-blob",
                    is_full_url: true,
                    extra: r#"{"region":"ap"}"#,
                },
            )
            .await
            .unwrap();

        let clone = svc
            .clone_provider(&created.provider_id, &connection_repo)
            .await
            .unwrap();

        let source_connections = connection_repo
            .list_for_provider(&created.provider_id)
            .await
            .unwrap();
        let clone_connections = connection_repo
            .list_for_provider(&clone.provider_id)
            .await
            .unwrap();
        assert_eq!(source_connections.len(), 1);
        assert_eq!(clone_connections.len(), 1);
        let (source, cloned) = (&source_connections[0], &clone_connections[0]);
        assert_ne!(cloned.connection_id, source.connection_id);
        assert_eq!(cloned.role, source.role);
        assert_eq!(cloned.label, source.label);
        assert_eq!(cloned.base_url, source.base_url);
        assert_eq!(cloned.auth_scheme, source.auth_scheme);
        assert_eq!(cloned.is_full_url, source.is_full_url);
        assert_eq!(cloned.extra, source.extra);

        // Raw row assert: the credentials ciphertext is copied verbatim (same
        // encryption key — no decrypt/re-encrypt, which would change the nonce).
        let ciphertexts: Vec<String> = nomifun_db::sqlx::query_scalar(
            "SELECT credentials_encrypted FROM provider_connections ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(ciphertexts, vec!["opaque-ciphertext-blob"; 2]);
    }

    #[tokio::test]
    async fn clone_copies_api_key_ciphertext_verbatim() {
        let (svc, pool) = setup_with_pool().await;
        let connection_repo = connection_repo_for_pool(&pool);
        let created = svc.create(sample_create_request()).await.unwrap();
        let clone = svc
            .clone_provider(&created.provider_id, &connection_repo)
            .await
            .unwrap();

        let fetch = |provider_id: String| {
            let pool = pool.clone();
            async move {
                nomifun_db::sqlx::query_scalar::<_, String>(
                    "SELECT api_key_encrypted FROM providers WHERE provider_id = ?",
                )
                .bind(provider_id)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let source_ciphertext = fetch(created.provider_id.clone()).await;
        let clone_ciphertext = fetch(clone.provider_id.clone()).await;
        // Re-encrypting would mint a fresh nonce and change the ciphertext;
        // the clone must carry the source bytes unchanged.
        assert_eq!(clone_ciphertext, source_ciphertext);
        assert_eq!(clone.api_key, "sk-ant-api03-test1234");
    }

    #[tokio::test]
    async fn clone_appends_sort_order_and_keeps_enabled() {
        let (svc, pool) = setup_with_pool().await;
        let connection_repo = connection_repo_for_pool(&pool);
        let created = svc
            .create(CreateProviderRequest {
                enabled: false,
                sort_order: Some(3),
                ..sample_create_request()
            })
            .await
            .unwrap();
        let clone = svc
            .clone_provider(&created.provider_id, &connection_repo)
            .await
            .unwrap();
        assert!(!clone.enabled, "enabled state follows the source");
        assert!(clone.sort_order > created.sort_order, "clone appends after current max");
    }

    #[tokio::test]
    async fn clone_managed_platform_is_forbidden() {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        std::mem::forget(db);
        let repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
        let encrypted = encrypt_string("internal-loopback-token", &TEST_KEY).unwrap();
        let provider_id = nomifun_common::ProviderId::new().into_string();
        repo.create(CreateProviderParams {
            provider_id: Some(&provider_id),
            platform: crate::managed_model::FREE_MODEL_PLATFORM,
            name: "Managed provider",
            base_url: "http://127.0.0.1:12345/v1",
            api_key_encrypted: &encrypted,
            models: r#"["big-pickle"]"#,
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

        let svc = service_for_pool(&pool);
        let connection_repo = connection_repo_for_pool(&pool);
        assert!(matches!(
            svc.clone_provider(&provider_id, &connection_repo).await,
            Err(AppError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn clone_missing_source_is_not_found() {
        let (svc, pool) = setup_with_pool().await;
        let connection_repo = connection_repo_for_pool(&pool);
        let err = svc
            .clone_provider("0190f5fe-7c00-7a00-8000-000000000099", &connection_repo)
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::NOT_FOUND);

        let err = svc
            .clone_provider("not-a-provider-id", &connection_repo)
            .await
            .unwrap_err();
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
    }
}

#[cfg(test)]
mod delete_guard_tests {
    use super::*;
    use nomifun_common::{ProviderUsage, ProviderUsageFeature};
    use nomifun_db::{
        SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
        models::Provider,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000098";

    struct CountingRepo {
        deleted: AtomicBool,
    }
    #[async_trait::async_trait]
    impl IProviderRepository for CountingRepo {
        async fn list(&self) -> Result<Vec<Provider>, nomifun_db::DbError> {
            Ok(vec![])
        }
        async fn find_by_id(&self, _: &str) -> Result<Option<Provider>, nomifun_db::DbError> {
            Ok(None)
        }
        async fn create(&self, _: nomifun_db::CreateProviderParams<'_>) -> Result<Provider, nomifun_db::DbError> {
            unimplemented!()
        }
        async fn update(&self, _: &str, _: nomifun_db::UpdateProviderParams<'_>) -> Result<Provider, nomifun_db::DbError> {
            unimplemented!()
        }
        async fn delete(&self, _: &str) -> Result<(), nomifun_db::DbError> {
            self.deleted.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Empty stand-in for the delete path, which never reads model rows.
    struct NoopModelRepo;
    #[async_trait::async_trait]
    impl IProviderModelRepository for NoopModelRepo {
        async fn list(&self) -> Result<Vec<ProviderModelRow>, nomifun_db::DbError> {
            Ok(vec![])
        }
        async fn list_for_provider(&self, _: &str) -> Result<Vec<ProviderModelRow>, nomifun_db::DbError> {
            Ok(vec![])
        }
        async fn get(&self, _: &str, _: &str) -> Result<Option<ProviderModelRow>, nomifun_db::DbError> {
            Ok(None)
        }
        async fn create(
            &self,
            _: &str,
            _: &nomifun_db::NewProviderModel<'_>,
        ) -> Result<ProviderModelRow, nomifun_db::DbError> {
            unimplemented!()
        }
        async fn insert_if_absent(
            &self,
            _: &str,
            _: &nomifun_db::NewProviderModel<'_>,
        ) -> Result<bool, nomifun_db::DbError> {
            unimplemented!()
        }
        async fn update(
            &self,
            _: &str,
            _: &str,
            _: &nomifun_db::ProviderModelUpdate<'_>,
        ) -> Result<ProviderModelRow, nomifun_db::DbError> {
            unimplemented!()
        }
        async fn set_health(&self, _: &str, _: &str, _: Option<&str>) -> Result<bool, nomifun_db::DbError> {
            unimplemented!()
        }
        async fn delete(&self, _: &str, _: &str) -> Result<bool, nomifun_db::DbError> {
            unimplemented!()
        }
    }

    struct FakeCoord {
        usages: Vec<ProviderUsage>,
    }
    #[async_trait::async_trait]
    impl crate::provider_deletion::ProviderDeletionCoordinator for FakeCoord {
        async fn usages(&self, _: &str) -> Result<Vec<ProviderUsage>, AppError> {
            Ok(self.usages.clone())
        }
    }

    struct RacingCoord {
        pool: nomifun_db::sqlx::SqlitePool,
    }

    #[async_trait::async_trait]
    impl crate::provider_deletion::ProviderDeletionCoordinator for RacingCoord {
        async fn usages(&self, provider_id: &str) -> Result<Vec<ProviderUsage>, AppError> {
            // Simulate a hard binding committed immediately after the friendly
            // application scan observed no usages. The repository transaction
            // guard remains the authoritative race barrier.
            nomifun_db::sqlx::query(
                "INSERT INTO conversations (\
                    conversation_id, user_id, name, type, extra, model, status, pinned, created_at, updated_at\
                 ) VALUES (\
                    '0190f5fe-7c00-7a00-8000-000000000098', \
                    (SELECT owner_user_id FROM installation_identity WHERE singleton_key = 'installation'), \
                    'racing conversation', 'nomi', '{}', ?,\
                    'pending', 0, 1, 1\
                 )",
            )
            .bind(format!(
                r#"{{"provider_id":"{provider_id}","model":"model"}}"#
            ))
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Internal(format!("create racing provider binding: {error}")))?;
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn delete_blocked_when_in_use() {
        let repo = Arc::new(CountingRepo {
            deleted: AtomicBool::new(false),
        });
        let coord = Arc::new(FakeCoord {
            usages: vec![ProviderUsage {
                feature: ProviderUsageFeature::DesktopCompanion,
                label: "甲".into(),
                target_id: None,
            }],
        });
        let svc = ProviderService::new(repo.clone(), Arc::new(NoopModelRepo), [0u8; 32])
            .with_deletion_coordinator(coord);
        let err = svc.delete(PROVIDER_ID).await.unwrap_err();
        assert!(matches!(err, AppError::ProviderInUse(_)));
        assert!(!repo.deleted.load(Ordering::SeqCst), "must not delete when in use");
    }

    #[tokio::test]
    async fn delete_proceeds_when_unused() {
        let repo = Arc::new(CountingRepo {
            deleted: AtomicBool::new(false),
        });
        let coord = Arc::new(FakeCoord {
            usages: vec![],
        });
        let svc = ProviderService::new(repo.clone(), Arc::new(NoopModelRepo), [0u8; 32])
            .with_deletion_coordinator(coord);
        svc.delete(PROVIDER_ID).await.unwrap();
        assert!(repo.deleted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn delete_race_is_reported_as_conflict_instead_of_internal_error() {
        let database = init_database_memory().await.unwrap();
        nomifun_db::sqlx::query(
            "INSERT INTO providers (\
                provider_id, platform, name, base_url, api_key_encrypted, models, enabled,\
                capabilities, created_at, updated_at\
             ) VALUES (\
                '0190f5fe-7c00-7a00-8000-000000000097', 'openai', 'Race provider', 'https://example.invalid',\
                'encrypted', '[]', 1, '[]', 1, 1\
             )",
        )
        .execute(database.pool())
        .await
        .unwrap();

        let repo = Arc::new(SqliteProviderRepository::new(database.pool().clone()));
        let coordinator = Arc::new(RacingCoord {
            pool: database.pool().clone(),
        });
        let service = ProviderService::new(
            repo.clone(),
            Arc::new(SqliteProviderModelRepository::new(database.pool().clone())),
            [0u8; 32],
        )
        .with_deletion_coordinator(coordinator);

        let provider_id = "0190f5fe-7c00-7a00-8000-000000000097";
        let error = service.delete(provider_id).await.unwrap_err();
        assert!(
            matches!(
                error,
                AppError::Conflict(ref message)
                    if message == "provider is still referenced by an executable Agent binding"
            ),
            "the atomic delete guard must surface as a deterministic conflict; got {error:?}"
        );
        assert_eq!(error.status_code(), axum::http::StatusCode::CONFLICT);
        assert!(repo.find_by_id(provider_id).await.unwrap().is_some());
    }
}

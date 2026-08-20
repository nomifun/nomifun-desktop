use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nomifun_api_types::{
    BedrockConfig, CreateProviderRequest, ProviderModelCapabilityInput, ProviderModelResponse,
    ProviderResponse, UpdateProviderRequest,
};
use nomifun_common::{AppError, ProviderId, ProviderInUseDetails};
use nomifun_db::{
    CreateProviderParams, IProviderConnectionRepository,
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    NewProviderModel, UpdateProviderParams, models::Provider,
};
use nomifun_model_invoke::{AuthMaterial, AuthScheme};
use serde::de::DeserializeOwned;

use crate::bedrock_probe::service::validate_bedrock_auth;
use crate::managed_model::is_managed_provider_platform;
use crate::provider_connection::{
    PreparedProviderConnection, credentials_have_values, decrypt_credentials,
    encrypt_credentials, normalize_auth_scheme, prepare_new_connection,
};
use crate::provider_deletion::SharedProviderDeletionCoordinator;
use crate::provider_model::{
    capability_row_to_response, row_to_model_response, rows_to_model_responses,
    serialize_capabilities, validate_capability_auth_scheme, validate_capability_urls,
    validate_positive_token_limit, validate_protocol, validate_provider_params,
};

#[derive(Clone)]
pub struct ProviderService {
    repo: Arc<dyn IProviderRepository>,
    model_repo: Arc<dyn IProviderModelRepository>,
    capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
    connection_repo: Arc<dyn IProviderConnectionRepository>,
    encryption_key: [u8; 32],
    coordinator: Option<SharedProviderDeletionCoordinator>,
}

impl ProviderService {
    pub fn new(
        repo: Arc<dyn IProviderRepository>,
        model_repo: Arc<dyn IProviderModelRepository>,
        capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
        connection_repo: Arc<dyn IProviderConnectionRepository>,
        encryption_key: [u8; 32],
    ) -> Self {
        Self {
            repo,
            model_repo,
            capability_repo,
            connection_repo,
            encryption_key,
            coordinator: None,
        }
    }

    pub fn with_deletion_coordinator(
        mut self,
        coordinator: SharedProviderDeletionCoordinator,
    ) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    pub async fn list(&self) -> Result<Vec<ProviderResponse>, AppError> {
        let providers = self.repo.list().await?;
        let models = rows_to_model_responses(
            self.model_repo.list().await?,
            self.capability_repo.list().await?,
        )?;
        let mut grouped = HashMap::<String, Vec<ProviderModelResponse>>::new();
        for model in models {
            grouped
                .entry(model.provider_id.clone())
                .or_default()
                .push(model);
        }
        providers
            .into_iter()
            .map(|provider| {
                let models = grouped.remove(&provider.provider_id).unwrap_or_default();
                self.row_to_response(provider, models)
            })
            .collect()
    }

    /// The sole public create path: provider, first model/capabilities, and all
    /// named connections are validated before one repository transaction.
    pub async fn create(
        &self,
        req: CreateProviderRequest,
    ) -> Result<ProviderResponse, AppError> {
        reject_managed_platform(&req.platform)?;
        validate_provider_id(req.provider_id.as_deref())?;
        validate_required_text("platform", &req.platform)?;
        validate_required_text("name", &req.name)?;
        validate_sort_order(req.sort_order)?;
        let bedrock_config = normalize_bedrock_config(req.bedrock_config.as_ref());
        let auth_scheme = validate_provider_auth(
            &req.platform,
            &req.auth_scheme,
            &req.credentials,
            bedrock_config.as_ref(),
        )?;
        validate_provider_base_url(&req.platform, &req.base_url)?;

        let prepared_connections = req
            .connections
            .iter()
            .map(|connection| prepare_new_connection(connection, &self.encryption_key))
            .collect::<Result<Vec<_>, _>>()?;
        let connection_targets = unique_connection_targets(&prepared_connections)?;
        validate_capability_set(
            &req.platform,
            &req.base_url,
            &auth_scheme,
            &connection_targets,
            &req.initial_model.capabilities,
        )?;

        let model_sort_order = req.initial_model.sort_order.unwrap_or(0);
        validate_sort_order(Some(model_sort_order))?;
        let serialized_capabilities = serialize_capabilities(&req.initial_model.capabilities)?;
        let db_capabilities = serialized_capabilities
            .iter()
            .map(|capability| capability.as_db())
            .collect::<Vec<_>>();
        let initial_model = NewProviderModel {
            model: req.initial_model.model.trim(),
            enabled: req.initial_model.enabled,
            sort_order: model_sort_order,
            description: req.initial_model.description.as_deref(),
            capabilities: &db_capabilities,
        };
        let db_connections = prepared_connections
            .iter()
            .map(PreparedProviderConnection::as_db)
            .collect::<Vec<_>>();
        let encrypted_credentials = encrypt_credentials(&req.credentials, &self.encryption_key)?;
        let bedrock_config = serialize_opt(&bedrock_config, "bedrock_config")?;
        let params = CreateProviderParams {
            provider_id: req.provider_id.as_deref(),
            platform: req.platform.trim(),
            name: req.name.trim(),
            base_url: req.base_url.trim(),
            auth_scheme: &auth_scheme,
            credentials_encrypted: &encrypted_credentials,
            enabled: req.enabled,
            bedrock_config: bedrock_config.as_deref(),
            sort_order: req.sort_order,
        };
        let (provider, model) = self
            .repo
            .create(params, &initial_model, &db_connections)
            .await?;
        let capabilities = self
            .capability_repo
            .list_for_model(&provider.provider_id, &model.model)
            .await?;
        let model = row_to_model_response(model, capabilities)?;
        self.row_to_response(provider, vec![model])
    }

    pub async fn update(
        &self,
        provider_id: &str,
        req: UpdateProviderRequest,
    ) -> Result<ProviderResponse, AppError> {
        validate_provider_id(Some(provider_id))?;
        validate_sort_order(req.sort_order)?;
        let existing = self
            .repo
            .find_by_id(provider_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provider {provider_id} not found")))?;
        if is_managed_provider_platform(&existing.platform) {
            return Err(managed_mutation_error());
        }
        if let Some(name) = req.name.as_deref() {
            validate_required_text("name", name)?;
        }

        let next_base_url = req.base_url.as_deref().unwrap_or(&existing.base_url);
        let next_auth_raw = req
            .auth_scheme
            .as_deref()
            .unwrap_or(&existing.auth_scheme);
        let existing_credentials =
            decrypt_credentials(&existing.credentials_encrypted, &self.encryption_key)?;
        let next_credentials = req
            .credentials
            .as_ref()
            .unwrap_or(&existing_credentials)
            .clone();
        let next_bedrock = match req.bedrock_config.as_ref() {
            Some(config) => normalize_bedrock_config(Some(config)),
            None => normalize_bedrock_config(
                deserialize_opt::<BedrockConfig>(&existing.bedrock_config, "bedrock_config")?
                    .as_ref(),
            ),
        };
        let auth_scheme = validate_provider_auth(
            &existing.platform,
            next_auth_raw,
            &next_credentials,
            next_bedrock.as_ref(),
        )?;
        validate_provider_base_url(&existing.platform, next_base_url)?;
        self.validate_existing_capabilities(
            provider_id,
            &existing.platform,
            next_base_url,
            &auth_scheme,
        )
            .await?;

        // Encryption is randomized. Re-encrypting unchanged plaintext would
        // make the repository treat a no-op edit as an invocation change and
        // unnecessarily invalidate every default-role capability health row.
        let encrypted_credentials = match req.credentials.as_ref() {
            Some(credentials) if credentials != &existing_credentials => {
                Some(encrypt_credentials(credentials, &self.encryption_key)?)
            }
            _ => None,
        };
        let bedrock_json = req
            .bedrock_config
            .as_ref()
            .map(|config| serialize_json(config, "bedrock_config"))
            .transpose()?;
        let provider = self
            .repo
            .update(
                provider_id,
                existing.config_revision,
                UpdateProviderParams {
                    name: req.name.as_deref().map(str::trim),
                    base_url: req.base_url.as_deref().map(str::trim),
                    auth_scheme: req.auth_scheme.as_ref().map(|_| auth_scheme.as_str()),
                    credentials_encrypted: encrypted_credentials.as_deref(),
                    enabled: req.enabled,
                    bedrock_config: bedrock_json.as_ref().map(|json| Some(json.as_str())),
                    sort_order: req.sort_order,
                },
            )
            .await?;
        let models = rows_to_model_responses(
            self.model_repo.list_for_provider(provider_id).await?,
            self.capability_repo.list_for_provider(provider_id).await?,
        )?;
        self.row_to_response(provider, models)
    }

    pub async fn clone_provider(
        &self,
        provider_id: &str,
        name: Option<&str>,
    ) -> Result<ProviderResponse, AppError> {
        validate_provider_id(Some(provider_id))?;
        let source = self
            .repo
            .find_by_id(provider_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provider {provider_id} not found")))?;
        if is_managed_provider_platform(&source.platform) {
            return Err(managed_mutation_error());
        }
        let clone_name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} copy", source.name.trim_end()));
        let provider = self.repo.clone_graph(provider_id, &clone_name).await?;
        let models = rows_to_model_responses(
            self.model_repo
                .list_for_provider(&provider.provider_id)
                .await?,
            self.capability_repo
                .list_for_provider(&provider.provider_id)
                .await?,
        )?;
        self.row_to_response(provider, models)
    }

    pub async fn delete(&self, provider_id: &str) -> Result<(), AppError> {
        validate_provider_id(Some(provider_id))?;
        if let Some(provider) = self.repo.find_by_id(provider_id).await?
            && is_managed_provider_platform(&provider.platform)
        {
            return Err(managed_mutation_error());
        }
        let lifecycle_barrier = self
            .coordinator
            .as_ref()
            .and_then(|coordinator| coordinator.provider_lifecycle_barrier());
        let _lifecycle_guard = if let Some(barrier) = lifecycle_barrier.as_ref() {
            Some(barrier.write().await)
        } else {
            None
        };
        if let Some(coordinator) = &self.coordinator {
            let usages = coordinator.usages(provider_id).await?;
            if !usages.is_empty() {
                return Err(AppError::ProviderInUse(ProviderInUseDetails { usages }));
            }
            coordinator.cleanup_soft_references(provider_id).await?;
        }
        self.repo.delete(provider_id).await?;
        Ok(())
    }

    async fn validate_existing_capabilities(
        &self,
        provider_id: &str,
        platform: &str,
        default_base_url: &str,
        default_auth_scheme: &str,
    ) -> Result<(), AppError> {
        let connections = self.connection_repo.list_for_provider(provider_id).await?;
        let connection_targets = connections
            .into_iter()
            .map(|connection| {
                (
                    connection.role,
                    ConnectionTarget {
                        base_url: connection.base_url,
                        auth_scheme: connection.auth_scheme,
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let capabilities = self.capability_repo.list_for_provider(provider_id).await?;
        for row in capabilities {
            let response = capability_row_to_response(row)?;
            let capability = ProviderModelCapabilityInput {
                task: response.task,
                traits: response.traits,
                protocol: response.protocol,
                connection_role: response.connection_role,
                base_url_override: response.base_url_override,
                endpoint: response.endpoint,
                poll_endpoint: response.poll_endpoint,
                content_endpoint: response.content_endpoint,
                realtime_endpoint: response.realtime_endpoint,
                allow_cross_origin_credentials: response.allow_cross_origin_credentials,
                provider_params: response.provider_params,
                context_limit: response.context_limit,
                output_limit: response.output_limit,
            };
            validate_capability(
                platform,
                default_base_url,
                default_auth_scheme,
                &connection_targets,
                &capability,
            )?;
        }
        Ok(())
    }

    fn row_to_response(
        &self,
        row: Provider,
        models: Vec<ProviderModelResponse>,
    ) -> Result<ProviderResponse, AppError> {
        Ok(ProviderResponse {
            provider_id: row.provider_id,
            platform: row.platform,
            name: row.name,
            base_url: row.base_url,
            auth_scheme: row.auth_scheme,
            has_credentials: credentials_have_values(&decrypt_credentials(
                &row.credentials_encrypted,
                &self.encryption_key,
            )?),
            models,
            enabled: row.enabled,
            bedrock_config: deserialize_opt(&row.bedrock_config, "bedrock_config")?,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

struct ConnectionTarget {
    base_url: String,
    auth_scheme: String,
}

fn unique_connection_targets(
    connections: &[PreparedProviderConnection],
) -> Result<HashMap<String, ConnectionTarget>, AppError> {
    let mut targets = HashMap::with_capacity(connections.len());
    for connection in connections {
        if targets
            .insert(
                connection.role().to_owned(),
                ConnectionTarget {
                    base_url: connection.base_url().to_owned(),
                    auth_scheme: connection.auth_scheme().to_owned(),
                },
            )
            .is_some()
        {
            return Err(AppError::BadRequest(format!(
                "connection role {:?} is duplicated",
                connection.role()
            )));
        }
    }
    Ok(targets)
}

fn validate_capability_set(
    platform: &str,
    default_base_url: &str,
    default_auth_scheme: &str,
    connection_targets: &HashMap<String, ConnectionTarget>,
    capabilities: &[ProviderModelCapabilityInput],
) -> Result<(), AppError> {
    if capabilities.is_empty() {
        return Err(AppError::BadRequest(
            "initial_model must declare at least one capability".into(),
        ));
    }
    let mut tasks = HashSet::with_capacity(capabilities.len());
    for capability in capabilities {
        if !tasks.insert(capability.task) {
            return Err(AppError::BadRequest(
                "initial_model contains a duplicate capability task".into(),
            ));
        }
        validate_capability(
            platform,
            default_base_url,
            default_auth_scheme,
            connection_targets,
            capability,
        )?;
    }
    Ok(())
}

fn validate_capability(
    platform: &str,
    default_base_url: &str,
    default_auth_scheme: &str,
    connection_targets: &HashMap<String, ConnectionTarget>,
    capability: &ProviderModelCapabilityInput,
) -> Result<(), AppError> {
    validate_protocol(platform, capability)?;
    validate_positive_token_limit("context_limit", capability.context_limit)?;
    validate_positive_token_limit("output_limit", capability.output_limit)?;
    validate_provider_params(
        &capability.protocol,
        capability.task,
        &capability.provider_params,
    )?;
    let (base_url, auth_scheme) = if capability.connection_role == "default" {
        (default_base_url, default_auth_scheme)
    } else {
        let target = connection_targets
            .get(&capability.connection_role)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "capability connection_role {:?} is not present in provider connections",
                    capability.connection_role
                ))
            })?;
        (target.base_url.as_str(), target.auth_scheme.as_str())
    };
    validate_capability_auth_scheme(capability, auth_scheme)?;
    validate_capability_urls(capability, base_url)
}

pub(crate) fn validate_provider_auth(
    platform: &str,
    raw_scheme: &str,
    credentials: &serde_json::Value,
    bedrock_config: Option<&BedrockConfig>,
) -> Result<String, AppError> {
    let scheme = normalize_auth_scheme(raw_scheme)?;
    let is_bedrock_platform = platform.trim().eq_ignore_ascii_case("bedrock");
    let parsed = AuthScheme::parse(&scheme).map_err(|error| {
        AppError::BadRequest(format!("invalid provider auth_scheme {scheme:?}: {}", error.message))
    })?;
    if matches!(parsed, AuthScheme::Bedrock) != is_bedrock_platform {
        return Err(AppError::BadRequest(
            if is_bedrock_platform {
                "the bedrock platform requires explicit auth_scheme 'bedrock'"
            } else {
                "auth_scheme 'bedrock' is valid only for the bedrock platform"
            }
            .into(),
        ));
    }
    AuthMaterial {
        scheme: parsed,
        credentials: credentials.clone(),
    }
    .validate_credentials()
    .map_err(|error| {
        AppError::BadRequest(format!(
            "credentials do not match auth_scheme {scheme:?}: {}",
            error.message
        ))
    })?;
    if is_bedrock_platform {
        validate_bedrock_auth(
            bedrock_config.ok_or_else(|| {
                AppError::BadRequest(
                    "bedrock_config is required when auth_scheme is 'bedrock'".into(),
                )
            })?,
            credentials,
        )?;
    } else if bedrock_config.is_some() {
        return Err(AppError::BadRequest(
            "bedrock_config is valid only for the bedrock platform".into(),
        ));
    }
    Ok(scheme)
}

fn normalize_bedrock_config(config: Option<&BedrockConfig>) -> Option<BedrockConfig> {
    config.map(|config| BedrockConfig {
        auth_method: config.auth_method,
        region: config.region.trim().to_owned(),
        profile: config
            .profile
            .as_deref()
            .map(str::trim)
            .filter(|profile| !profile.is_empty())
            .map(str::to_owned),
    })
}

pub(crate) fn validate_provider_base_url(platform: &str, base_url: &str) -> Result<(), AppError> {
    if platform.trim().eq_ignore_ascii_case("bedrock") {
        return if base_url.trim().is_empty() {
            Ok(())
        } else {
            Err(AppError::BadRequest(
                "the bedrock SDK provider must not define an HTTP base_url".into(),
            ))
        };
    }
    validate_base_url(base_url)
}

pub(crate) fn validate_base_url(base_url: &str) -> Result<(), AppError> {
    crate::provider_model::validate_base_url(base_url)
}

fn validate_provider_id(provider_id: Option<&str>) -> Result<(), AppError> {
    let Some(provider_id) = provider_id else {
        return Ok(());
    };
    ProviderId::parse(provider_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))
}

fn validate_required_text(field: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    Ok(())
}

fn validate_sort_order(sort_order: Option<i64>) -> Result<(), AppError> {
    if sort_order.is_some_and(|value| value < 0) {
        return Err(AppError::BadRequest(
            "sort_order must be greater than or equal to zero".into(),
        ));
    }
    Ok(())
}

fn reject_managed_platform(platform: &str) -> Result<(), AppError> {
    if is_managed_provider_platform(platform.trim()) {
        return Err(managed_mutation_error());
    }
    Ok(())
}

fn managed_mutation_error() -> AppError {
    AppError::Forbidden(
        "Managed model providers must be changed through their dedicated model-service API".into(),
    )
}

fn serialize_json<T: serde::Serialize>(value: &T, field: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::Internal(format!("failed to serialize {field}: {error}")))
}

fn serialize_opt<T: serde::Serialize>(
    value: &Option<T>,
    field: &str,
) -> Result<Option<String>, AppError> {
    value
        .as_ref()
        .map(|value| serialize_json(value, field))
        .transpose()
}

pub(crate) fn deserialize_opt<T: DeserializeOwned>(
    json: &Option<String>,
    field: &str,
) -> Result<Option<T>, AppError> {
    json.as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| AppError::Internal(format!("failed to parse {field} JSON: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::BedrockAuthMethod;

    #[test]
    fn provider_auth_is_explicit_and_validated() {
        assert_eq!(
            validate_provider_auth(
                "openai",
                " bearer ",
                &serde_json::json!({"api_keys":["sk-one", "sk-two"]}),
                None,
            )
            .unwrap(),
            "bearer"
        );
        assert!(
            validate_provider_auth("deepgram", "token", &serde_json::json!({}), None).is_err()
        );
        assert!(
            validate_provider_auth(
                "openai",
                "bedrock",
                &serde_json::json!({}),
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn bedrock_auth_methods_are_strict_and_do_not_fallback() {
        let access_key = BedrockConfig {
            auth_method: BedrockAuthMethod::AccessKey,
            region: " us-east-1 ".into(),
            profile: None,
        };
        validate_provider_auth(
            "bedrock",
            "bedrock",
            &serde_json::json!({
                "access_key_id":"AKIA",
                "secret_access_key":"secret",
                "session_token":"session"
            }),
            Some(&access_key),
        )
        .unwrap();
        assert!(
            validate_provider_auth(
                "bedrock",
                "bedrock",
                &serde_json::json!({}),
                Some(&access_key),
            )
            .is_err()
        );

        let profile = BedrockConfig {
            auth_method: BedrockAuthMethod::Profile,
            region: "us-east-1".into(),
            profile: Some("work".into()),
        };
        validate_provider_auth(
            "bedrock",
            "bedrock",
            &serde_json::json!({}),
            Some(&profile),
        )
        .unwrap();
        assert!(
            validate_provider_auth(
                "bedrock",
                "bedrock",
                &serde_json::json!({"access_key_id":"AKIA","secret_access_key":"secret"}),
                Some(&profile),
            )
            .is_err()
        );

        let default_chain = BedrockConfig {
            auth_method: BedrockAuthMethod::DefaultChain,
            region: "us-east-1".into(),
            profile: None,
        };
        validate_provider_auth(
            "bedrock",
            "bedrock",
            &serde_json::json!({}),
            Some(&default_chain),
        )
        .unwrap();
    }

    #[test]
    fn bedrock_provider_rejects_http_base_url() {
        validate_provider_base_url("bedrock", "").unwrap();
        assert!(
            validate_provider_base_url(
                "bedrock",
                "https://bedrock-runtime.us-east-1.amazonaws.com"
            )
            .is_err()
        );
    }

    #[test]
    fn initial_capability_requires_declared_named_connection() {
        let capability: ProviderModelCapabilityInput = serde_json::from_value(serde_json::json!({
            "task":"chat",
            "protocol":"openai.chat_text",
            "connection_role":"secondary"
        }))
        .unwrap();
        let error = validate_capability(
            "openai",
            "https://api.openai.com/v1",
            "bearer",
            &HashMap::new(),
            &capability,
        )
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(message) if message.contains("not present")));
    }
}

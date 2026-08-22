use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use nomifun_api_types::{
    CapabilityHealth, ModelTask, ModelTrait, ProviderModelCapabilityInput,
    ProviderModelCapabilityResponse, ProviderModelResponse, SaveProviderModelRequest,
};
use nomifun_common::{AppError, ProviderId};
use nomifun_db::{
    CoordinatedProviderModelDelete, IProviderConnectionRepository,
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    NewProviderModel, NewProviderModelCapability, ProviderModelCapabilityRow, ProviderModelRow,
};
use nomifun_model_invoke::{
    ProtocolScope, ProtocolTransportKind, protocol_descriptor,
    validate_credentialed_target_url, validate_provider_params_for_protocol,
};
use reqwest::Url;

use crate::managed_model::is_managed_provider_platform;
use crate::provider_connection::{normalize_auth_scheme, validate_role};
use crate::provider_deletion::SharedProviderDeletionCoordinator;

#[derive(Clone)]
pub struct ProviderModelService {
    model_repo: Arc<dyn IProviderModelRepository>,
    capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
    provider_repo: Arc<dyn IProviderRepository>,
    connection_repo: Arc<dyn IProviderConnectionRepository>,
    deletion_coordinator: SharedProviderDeletionCoordinator,
}

impl ProviderModelService {
    pub fn new(
        model_repo: Arc<dyn IProviderModelRepository>,
        capability_repo: Arc<dyn IProviderModelCapabilityRepository>,
        provider_repo: Arc<dyn IProviderRepository>,
        connection_repo: Arc<dyn IProviderConnectionRepository>,
        deletion_coordinator: SharedProviderDeletionCoordinator,
    ) -> Self {
        Self {
            model_repo,
            capability_repo,
            provider_repo,
            connection_repo,
            deletion_coordinator,
        }
    }

    pub async fn list(
        &self,
        provider_id: Option<&str>,
    ) -> Result<Vec<ProviderModelResponse>, AppError> {
        if let Some(provider_id) = provider_id {
            validate_provider_id(provider_id)?;
        }
        let (models, capabilities) = match provider_id {
            Some(provider_id) => (
                self.model_repo.list_for_provider(provider_id).await?,
                self.capability_repo.list_for_provider(provider_id).await?,
            ),
            None => (self.model_repo.list().await?, self.capability_repo.list().await?),
        };
        rows_to_model_responses(models, capabilities)
    }

    pub async fn get(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<Option<ProviderModelResponse>, AppError> {
        validate_provider_id(provider_id)?;
        let Some(row) = self.model_repo.get(provider_id, model).await? else {
            return Ok(None);
        };
        let capabilities = self
            .capability_repo
            .list_for_model(provider_id, model)
            .await?;
        Ok(Some(row_to_model_response(row, capabilities)?))
    }

    /// Upsert one model and replace its complete capability configuration in
    /// the repository's single transaction.
    pub async fn save(
        &self,
        req: SaveProviderModelRequest,
    ) -> Result<ProviderModelResponse, AppError> {
        validate_provider_id(&req.provider_id)?;
        let provider = self
            .provider_repo
            .find_by_id(&req.provider_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("Provider {} not found", req.provider_id))
            })?;
        if is_managed_provider_platform(&provider.platform) {
            return Err(AppError::Forbidden(
                "Managed model providers must be changed through their dedicated model-service API"
                    .into(),
            ));
        }

        let existing = self
            .model_repo
            .get(&req.provider_id, &req.model.model)
            .await?;
        let sort_order = match req.model.sort_order {
            Some(value) => {
                validate_sort_order(value)?;
                value
            }
            None => match existing {
                Some(ref row) => row.sort_order,
                None => self
                    .model_repo
                    .list_for_provider(&req.provider_id)
                    .await?
                    .into_iter()
                    .map(|row| row.sort_order)
                    .max()
                    .unwrap_or(-1)
                    .saturating_add(1),
            },
        };

        self.validate_capabilities(&provider, &req.model.capabilities)
            .await?;
        let serialized = serialize_capabilities(&req.model.capabilities)?;
        let db_capabilities = serialized
            .iter()
            .map(SerializedCapability::as_db)
            .collect::<Vec<_>>();
        let new_model = NewProviderModel {
            model: req.model.model.trim(),
            enabled: req.model.enabled,
            sort_order,
            description: req.model.description.as_deref(),
            capabilities: &db_capabilities,
        };
        let row = self
            .model_repo
            .save(&req.provider_id, provider.config_revision, &new_model)
            .await?;
        let capabilities = self
            .capability_repo
            .list_for_model(&req.provider_id, &row.model)
            .await?;
        row_to_model_response(row, capabilities)
    }

    pub async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, AppError> {
        validate_provider_id(provider_id)?;
        let model = model.trim();
        if model.is_empty() || model.chars().count() > 512 {
            return Err(AppError::BadRequest(
                "provider model must contain 1 to 512 characters".into(),
            ));
        }
        let provider = self
            .provider_repo
            .find_by_id(provider_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Provider {provider_id} not found")))?;
        if is_managed_provider_platform(&provider.platform) {
            return Err(AppError::Forbidden(
                "Managed model providers must be changed through their dedicated model-service API"
                    .into(),
            ));
        }
        let lifecycle_barrier = self.deletion_coordinator.provider_lifecycle_barrier();
        let _lifecycle_guard = if let Some(barrier) = lifecycle_barrier.as_ref() {
            Some(barrier.write().await)
        } else {
            None
        };
        if self.model_repo.get(provider_id, model).await?.is_none() {
            return Ok(false);
        }
        let cleanup = self
            .deletion_coordinator
            .prepare_soft_model_cleanup(provider_id, model)
            .await?;
        Ok(self
            .model_repo
            .delete_coordinated(&CoordinatedProviderModelDelete {
                provider_id: provider_id.to_owned(),
                model: model.to_owned(),
                expected_config_revision: provider.config_revision,
                cleanup,
            })
            .await?)
    }

    async fn validate_capabilities(
        &self,
        provider: &nomifun_db::models::Provider,
        capabilities: &[ProviderModelCapabilityInput],
    ) -> Result<(), AppError> {
        if capabilities.is_empty() {
            return Err(AppError::BadRequest(
                "provider model must declare at least one capability".into(),
            ));
        }
        let mut tasks = HashSet::with_capacity(capabilities.len());
        for capability in capabilities {
            if !tasks.insert(capability.task) {
                return Err(AppError::BadRequest(format!(
                    "capability task {} is duplicated",
                    task_wire(capability.task)?
                )));
            }
            validate_positive_token_limit("context_limit", capability.context_limit)?;
            validate_positive_token_limit("output_limit", capability.output_limit)?;
            validate_provider_params(
                &capability.protocol,
                capability.task,
                &capability.provider_params,
            )?;
            validate_protocol(&provider.platform, capability)?;

            let (connection_base_url, connection_auth_scheme) =
                if capability.connection_role == "default" {
                    (provider.base_url.clone(), provider.auth_scheme.clone())
            } else {
                validate_role(&capability.connection_role).map_err(|error| match error {
                    AppError::BadRequest(message) => AppError::BadRequest(format!(
                        "invalid capability connection_role {:?}: {message}",
                        capability.connection_role
                    )),
                    other => other,
                })?;
                let connection = self
                    .connection_repo
                    .get(&provider.provider_id, &capability.connection_role)
                    .await?
                    .ok_or_else(|| {
                        AppError::BadRequest(format!(
                            "capability connection_role {:?} does not exist for provider {}",
                            capability.connection_role, provider.provider_id
                        ))
                    })?;
                    (connection.base_url, connection.auth_scheme)
                };
            validate_capability_auth_scheme(capability, &connection_auth_scheme)?;
            validate_capability_urls(capability, &connection_base_url)?;
        }
        Ok(())
    }
}

fn validate_provider_id(provider_id: &str) -> Result<(), AppError> {
    ProviderId::parse(provider_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid provider id: {error}")))
}

fn validate_sort_order(value: i64) -> Result<(), AppError> {
    if value < 0 {
        return Err(AppError::BadRequest(
            "sort_order must be greater than or equal to zero".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_positive_token_limit(
    field: &str,
    value: Option<i64>,
) -> Result<(), AppError> {
    if value.is_some_and(|value| value <= 0) {
        return Err(AppError::BadRequest(format!(
            "capability {field} must be greater than zero"
        )));
    }
    Ok(())
}

pub(crate) fn validate_protocol(
    platform: &str,
    capability: &ProviderModelCapabilityInput,
) -> Result<(), AppError> {
    let protocol = capability.protocol.trim();
    if protocol.is_empty() {
        return Err(AppError::BadRequest(
            "capability protocol must not be blank".into(),
        ));
    }
    let descriptor = protocol_descriptor(protocol).ok_or_else(|| {
        AppError::BadRequest(format!("unknown capability protocol {protocol:?}"))
    })?;
    if !descriptor.supported_tasks.contains(&capability.task) {
        return Err(AppError::BadRequest(format!(
            "protocol {protocol:?} does not support task {}",
            task_wire(capability.task)?
        )));
    }
    if descriptor.requires_output_ceiling && capability.output_limit.is_none() {
        return Err(AppError::BadRequest(format!(
            "protocol {protocol:?} requires capability output_limit (Max output tokens)"
        )));
    }
    let supports_platform = descriptor
        .platforms
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(platform))
        || descriptor.scopes.contains(&ProtocolScope::Custom);
    if !supports_platform {
        return Err(AppError::BadRequest(format!(
            "protocol {protocol:?} is not available for provider platform {platform:?}"
        )));
    }
    Ok(())
}

pub(crate) fn validate_capability_auth_scheme(
    capability: &ProviderModelCapabilityInput,
    raw_auth_scheme: &str,
) -> Result<(), AppError> {
    let descriptor = protocol_descriptor(capability.protocol.trim()).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown capability protocol {:?}",
            capability.protocol.trim()
        ))
    })?;
    let auth_scheme = normalize_auth_scheme(raw_auth_scheme)?;
    let supported = descriptor.allowed_auth_schemes.iter().any(|allowed| {
        auth_scheme_exact_match(allowed, &auth_scheme)
            || (allowed == "header_key:<name>"
                && auth_scheme
                    .strip_prefix("header_key:")
                    .is_some_and(|name| !name.trim().is_empty()))
            || (allowed == "query_key:<param>"
                && auth_scheme
                    .strip_prefix("query_key:")
                    .is_some_and(|param| !param.trim().is_empty()))
    });
    if supported {
        return Ok(());
    }
    Err(AppError::BadRequest(format!(
        "protocol {:?} does not support connection auth_scheme {:?}; allowed: {}",
        capability.protocol.trim(),
        auth_scheme,
        descriptor.allowed_auth_schemes.join(", ")
    )))
}

fn validate_endpoint_overrides(
    descriptor: &nomifun_model_invoke::ProtocolDescriptor,
    capability: &ProviderModelCapabilityInput,
) -> Result<(), AppError> {
    for (field, value) in [
        ("endpoint", capability.endpoint.as_deref()),
        ("poll_endpoint", capability.poll_endpoint.as_deref()),
        ("content_endpoint", capability.content_endpoint.as_deref()),
        ("realtime_endpoint", capability.realtime_endpoint.as_deref()),
    ] {
        let Some(value) = value else { continue };
        let endpoint = descriptor
            .endpoints
            .iter()
            .find(|endpoint| endpoint.task == capability.task && endpoint.field == field)
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "protocol {:?} does not define {field} for task {}",
                    descriptor.protocol_id,
                    task_wire(capability.task).unwrap_or_else(|_| "unknown".into())
                ))
            })?;
        if !endpoint.editable && value.trim() != endpoint.default_value {
            return Err(AppError::BadRequest(format!(
                "protocol {:?} does not allow overriding {field}",
                descriptor.protocol_id
            )));
        }
        nomifun_model_invoke::validate_endpoint_template(
            &descriptor.protocol_id,
            capability.task,
            field,
            value,
        )
        .map_err(|error| AppError::BadRequest(error.message))?;
    }
    Ok(())
}

fn auth_scheme_exact_match(allowed: &str, actual: &str) -> bool {
    match (
        allowed.strip_prefix("header_key:"),
        actual.strip_prefix("header_key:"),
    ) {
        (Some(allowed_header), Some(actual_header)) => {
            allowed_header.eq_ignore_ascii_case(actual_header)
        }
        _ => allowed == actual,
    }
}

pub(crate) fn validate_provider_params(
    protocol: &str,
    task: ModelTask,
    params: &serde_json::Value,
) -> Result<(), AppError> {
    validate_provider_params_for_protocol(protocol.trim(), task, params).map_err(Into::into)
}

pub(crate) fn validate_capability_urls(
    capability: &ProviderModelCapabilityInput,
    connection_base_url: &str,
) -> Result<(), AppError> {
    let descriptor = protocol_descriptor(capability.protocol.trim()).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown capability protocol {:?}",
            capability.protocol.trim()
        ))
    })?;
    validate_endpoint_overrides(&descriptor, capability)?;
    let effective_base = capability
        .base_url_override
        .as_deref()
        .unwrap_or(connection_base_url);

    if descriptor.transport == ProtocolTransportKind::Sdk {
        if !connection_base_url.trim().is_empty()
            || capability
            .base_url_override
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            || capability.endpoint.is_some()
            || capability.poll_endpoint.is_some()
            || capability.content_endpoint.is_some()
            || capability.realtime_endpoint.is_some()
        {
            return Err(AppError::BadRequest(
                "SDK capability must not use a connection/base URL override or transport endpoints"
                    .into(),
            ));
        }
        return Ok(());
    }

    if let Some(override_url) = capability.base_url_override.as_deref() {
        validate_credentialed_target_url(
            connection_base_url,
            capability.allow_cross_origin_credentials,
            override_url,
            "base_url_override",
            descriptor.transport,
            false,
        )
        .map_err(|error| AppError::BadRequest(error.message))?;
    }

    let base = match descriptor.transport {
        ProtocolTransportKind::Http => parse_http_url(effective_base, "capability base URL")?,
        ProtocolTransportKind::Websocket => {
            parse_websocket_base_url(effective_base, "capability base URL")?
        }
        ProtocolTransportKind::Sdk => unreachable!("handled above"),
    };

    if descriptor.transport == ProtocolTransportKind::Websocket
        && (capability.endpoint.is_some()
            || capability.poll_endpoint.is_some()
            || capability.content_endpoint.is_some())
    {
        return Err(AppError::BadRequest(
            "WebSocket capability must use realtime_endpoint, not HTTP job endpoints".into(),
        ));
    }
    if descriptor.transport == ProtocolTransportKind::Http
        && capability.realtime_endpoint.is_some()
    {
        return Err(AppError::BadRequest(
            "HTTP capability must not define realtime_endpoint".into(),
        ));
    }

    for (field, value) in [
        ("endpoint", capability.endpoint.as_deref()),
        ("poll_endpoint", capability.poll_endpoint.as_deref()),
        ("content_endpoint", capability.content_endpoint.as_deref()),
    ] {
        let Some(value) = value else { continue };
        validate_credentialed_target_url(
            effective_base,
            capability.allow_cross_origin_credentials,
            value,
            field,
            ProtocolTransportKind::Http,
            true,
        )
        .map_err(|error| AppError::BadRequest(error.message))?;
        resolve_http_endpoint(&base, value, field)?;
    }

    if let Some(value) = capability.realtime_endpoint.as_deref() {
        validate_credentialed_target_url(
            effective_base,
            capability.allow_cross_origin_credentials,
            value,
            "realtime_endpoint",
            ProtocolTransportKind::Websocket,
            true,
        )
        .map_err(|error| AppError::BadRequest(error.message))?;
        resolve_realtime_endpoint(&base, value)?;
    }
    Ok(())
}

pub(crate) fn validate_base_url(value: &str) -> Result<(), AppError> {
    parse_http_url(value, "base_url").map(|_| ())
}

fn parse_http_url(value: &str, field: &str) -> Result<Url, AppError> {
    let url = Url::parse(value.trim())
        .map_err(|error| AppError::BadRequest(format!("{field} is not a valid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::BadRequest(format!(
            "{field} must be an absolute http(s) URL with a host"
        )));
    }
    validate_safe_url(&url, field)?;
    if url.query().is_some() {
        return Err(AppError::BadRequest(format!(
            "{field} must not contain a query; put task-specific query parameters on the endpoint"
        )));
    }
    Ok(url)
}

fn parse_realtime_url(value: &str, field: &str) -> Result<Url, AppError> {
    let url = Url::parse(value.trim())
        .map_err(|error| AppError::BadRequest(format!("{field} is not a valid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") || url.host_str().is_none() {
        return Err(AppError::BadRequest(format!(
            "{field} must be an absolute http(s) or ws(s) URL with a host"
        )));
    }
    validate_safe_url(&url, field)?;
    Ok(url)
}

fn parse_websocket_base_url(value: &str, field: &str) -> Result<Url, AppError> {
    let url = Url::parse(value.trim())
        .map_err(|error| AppError::BadRequest(format!("{field} is not a valid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") || url.host_str().is_none() {
        return Err(AppError::BadRequest(format!(
            "{field} must be an absolute http(s) or ws(s) URL with a host"
        )));
    }
    validate_safe_url(&url, field)?;
    if url.query().is_some() {
        return Err(AppError::BadRequest(format!(
            "{field} must not contain a query; put task-specific query parameters on realtime_endpoint"
        )));
    }
    Ok(url)
}

fn resolve_realtime_endpoint(base: &Url, value: &str) -> Result<Url, AppError> {
    let value = value.trim();
    if value.starts_with("//") {
        return Err(AppError::BadRequest(
            "realtime_endpoint must not be a scheme-relative URL".into(),
        ));
    }
    match Url::parse(value) {
        Ok(_) => parse_realtime_url(value, "realtime_endpoint"),
        Err(_) => {
            let resolved = resolve_relative_url(base, value, "realtime_endpoint")?;
            validate_safe_url(&resolved, "realtime_endpoint")?;
            Ok(resolved)
        }
    }
}

fn validate_safe_url(url: &Url, field: &str) -> Result<(), AppError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::BadRequest(format!(
            "{field} must not contain URL credentials"
        )));
    }
    if url.fragment().is_some() {
        return Err(AppError::BadRequest(format!(
            "{field} must not contain a fragment"
        )));
    }
    Ok(())
}

fn resolve_http_endpoint(base: &Url, value: &str, field: &str) -> Result<Url, AppError> {
    let value = value.trim();
    if value.starts_with("//") {
        return Err(AppError::BadRequest(format!(
            "{field} must not be a scheme-relative URL"
        )));
    }
    let url = match Url::parse(value) {
        Ok(url) => {
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(AppError::BadRequest(format!(
                    "absolute {field} must be an http(s) URL with a host"
                )));
            }
            url
        }
        Err(_) => resolve_relative_url(base, value, field)?,
    };
    validate_safe_url(&url, field)?;
    Ok(url)
}

/// Join a relative endpoint onto the connection root using the same algebra the
/// runtime uses. Save-time and runtime previously carried two independent
/// implementations of this join that had to stay byte-identical by hand.
fn resolve_relative_url(base: &Url, value: &str, field: &str) -> Result<Url, AppError> {
    let relative = value.trim();
    if relative.is_empty() || relative.starts_with('?') || relative.starts_with('#') {
        return Err(AppError::BadRequest(format!(
            "relative {field} must contain a path"
        )));
    }
    if relative.starts_with("//") {
        return Err(AppError::BadRequest(format!(
            "{field} must not be a scheme-relative URL"
        )));
    }
    let combined = nomifun_model_invoke::join_endpoint(base.as_str(), relative);
    Url::parse(&combined)
        .map_err(|error| AppError::BadRequest(format!("{field} is not a valid relative URL: {error}")))
}

pub(crate) struct SerializedCapability {
    task: String,
    traits: String,
    protocol: String,
    connection_role: String,
    base_url_override: Option<String>,
    endpoint: Option<String>,
    poll_endpoint: Option<String>,
    content_endpoint: Option<String>,
    realtime_endpoint: Option<String>,
    allow_cross_origin_credentials: bool,
    provider_params: String,
    context_limit: Option<i64>,
    output_limit: Option<i64>,
}

impl SerializedCapability {
    pub(crate) fn as_db(&self) -> NewProviderModelCapability<'_> {
        NewProviderModelCapability {
            task: &self.task,
            traits: &self.traits,
            protocol: &self.protocol,
            connection_role: &self.connection_role,
            base_url_override: self.base_url_override.as_deref(),
            endpoint: self.endpoint.as_deref(),
            poll_endpoint: self.poll_endpoint.as_deref(),
            content_endpoint: self.content_endpoint.as_deref(),
            realtime_endpoint: self.realtime_endpoint.as_deref(),
            allow_cross_origin_credentials: self.allow_cross_origin_credentials,
            provider_params: &self.provider_params,
            context_limit: self.context_limit,
            output_limit: self.output_limit,
        }
    }
}

pub(crate) fn serialize_capabilities(
    capabilities: &[ProviderModelCapabilityInput],
) -> Result<Vec<SerializedCapability>, AppError> {
    capabilities
        .iter()
        .map(|capability| {
            Ok(SerializedCapability {
                task: task_wire(capability.task)?,
                traits: serde_json::to_string(&capability.traits).map_err(|error| {
                    AppError::Internal(format!("failed to serialize capability traits: {error}"))
                })?,
                protocol: capability.protocol.trim().to_owned(),
                connection_role: capability.connection_role.trim().to_owned(),
                base_url_override: capability.base_url_override.clone(),
                endpoint: capability.endpoint.clone(),
                poll_endpoint: capability.poll_endpoint.clone(),
                content_endpoint: capability.content_endpoint.clone(),
                realtime_endpoint: capability.realtime_endpoint.clone(),
                allow_cross_origin_credentials: capability.allow_cross_origin_credentials,
                provider_params: serde_json::to_string(&capability.provider_params).map_err(
                    |error| {
                        AppError::Internal(format!(
                            "failed to serialize capability provider_params: {error}"
                        ))
                    },
                )?,
                context_limit: capability.context_limit,
                output_limit: capability.output_limit,
            })
        })
        .collect()
}

fn task_wire(task: ModelTask) -> Result<String, AppError> {
    serde_json::to_value(task)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| AppError::Internal("failed to serialize model task".into()))
}

pub(crate) fn rows_to_model_responses(
    models: Vec<ProviderModelRow>,
    capabilities: Vec<ProviderModelCapabilityRow>,
) -> Result<Vec<ProviderModelResponse>, AppError> {
    let mut grouped = HashMap::<(String, String), Vec<ProviderModelCapabilityRow>>::new();
    for capability in capabilities {
        grouped
            .entry((capability.provider_id.clone(), capability.model.clone()))
            .or_default()
            .push(capability);
    }
    models
        .into_iter()
        .map(|row| {
            let capabilities = grouped
                .remove(&(row.provider_id.clone(), row.model.clone()))
                .unwrap_or_default();
            row_to_model_response(row, capabilities)
        })
        .collect()
}

pub(crate) fn row_to_model_response(
    row: ProviderModelRow,
    capabilities: Vec<ProviderModelCapabilityRow>,
) -> Result<ProviderModelResponse, AppError> {
    let mut capabilities = capabilities
        .into_iter()
        .map(capability_row_to_response)
        .collect::<Result<Vec<_>, _>>()?;
    capabilities.sort_by_key(|capability| model_task_order(capability.task));
    Ok(ProviderModelResponse {
        provider_id: row.provider_id,
        model: row.model,
        enabled: row.enabled,
        sort_order: row.sort_order,
        description: row.description,
        capabilities,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn model_task_order(task: ModelTask) -> u8 {
    match task {
        ModelTask::Chat => 0,
        ModelTask::RealtimeConversation => 1,
        ModelTask::ImageGeneration => 2,
        ModelTask::ImageEdit => 3,
        ModelTask::VideoGeneration => 4,
        ModelTask::SpeechSynthesis => 5,
        ModelTask::SpeechRecognition => 6,
        ModelTask::Embedding => 7,
        ModelTask::Rerank => 8,
    }
}

pub(crate) fn capability_row_to_response(
    row: ProviderModelCapabilityRow,
) -> Result<ProviderModelCapabilityResponse, AppError> {
    let task = serde_json::from_value(serde_json::Value::String(row.task.clone())).map_err(
        |error| {
            AppError::Internal(format!(
                "stored capability task {:?} is invalid: {error}",
                row.task
            ))
        },
    )?;
    let traits: Vec<ModelTrait> = serde_json::from_str(&row.traits).map_err(|error| {
        AppError::Internal(format!(
            "stored capability traits for {}/{} are invalid: {error}",
            row.provider_id, row.model
        ))
    })?;
    let provider_params = serde_json::from_str(&row.provider_params).map_err(|error| {
        AppError::Internal(format!(
            "stored capability provider_params for {}/{} are invalid: {error}",
            row.provider_id, row.model
        ))
    })?;
    let health = row
        .health
        .as_deref()
        .map(serde_json::from_str::<CapabilityHealth>)
        .transpose()
        .map_err(|error| {
            AppError::Internal(format!(
                "stored capability health for {}/{} is invalid: {error}",
                row.provider_id, row.model
            ))
        })?;
    Ok(ProviderModelCapabilityResponse {
        task,
        traits,
        protocol: row.protocol,
        connection_role: row.connection_role,
        base_url_override: row.base_url_override,
        endpoint: row.endpoint,
        poll_endpoint: row.poll_endpoint,
        content_endpoint: row.content_endpoint,
        realtime_endpoint: row.realtime_endpoint,
        allow_cross_origin_credentials: row.allow_cross_origin_credentials,
        provider_params,
        context_limit: row.context_limit,
        output_limit: row.output_limit,
        health,
        health_checked_at: row.health_checked_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(task: ModelTask, protocol: &str) -> ProviderModelCapabilityInput {
        ProviderModelCapabilityInput {
            task,
            traits: Vec::new(),
            protocol: protocol.into(),
            connection_role: "default".into(),
            base_url_override: None,
            endpoint: None,
            poll_endpoint: None,
            content_endpoint: None,
            realtime_endpoint: None,
            allow_cross_origin_credentials: false,
            provider_params: serde_json::json!({}),
            context_limit: None,
            output_limit: None,
        }
    }

    #[test]
    fn provider_params_reject_locally_owned_routing_fields() {
        for key in nomifun_model_invoke::reserved_local_transport_param_keys() {
            assert!(nomifun_model_invoke::is_reserved_local_transport_param_key(key));
            let mut object = serde_json::Map::new();
            object.insert((*key).to_owned(), serde_json::Value::Null);
            let error = validate_provider_params(
                "openai.audio_speech",
                ModelTask::SpeechSynthesis,
                &serde_json::Value::Object(object),
            )
            .unwrap_err();
            assert!(matches!(error, AppError::BadRequest(message) if message.contains("reserved local transport/auth")));
        }
        validate_provider_params(
            "openai.audio_speech",
            ModelTask::SpeechSynthesis,
            &serde_json::json!({"voice":"alloy","speed":1.1}),
        )
        .unwrap();
    }

    #[test]
    fn provider_params_follow_the_exact_protocol_encoding_contract() {
        validate_provider_params(
            "bedrock.anthropic_messages",
            ModelTask::Chat,
            &serde_json::json!({"top_k":7,"future":{"nested":true}}),
        )
        .unwrap();
        validate_provider_params(
            "xai.stt",
            ModelTask::SpeechRecognition,
            &serde_json::json!({"keyterm":["NomiFun","StepFun"]}),
        )
        .unwrap();
        assert!(
            validate_provider_params(
                "openai.images",
                ModelTask::ImageEdit,
                &serde_json::json!({"future":{"nested":true}}),
            )
            .is_err()
        );
    }

    #[test]
    fn response_task_order_matches_the_model_management_contract() {
        let tasks = [
            ModelTask::Chat,
            ModelTask::RealtimeConversation,
            ModelTask::ImageGeneration,
            ModelTask::ImageEdit,
            ModelTask::VideoGeneration,
            ModelTask::SpeechSynthesis,
            ModelTask::SpeechRecognition,
            ModelTask::Embedding,
            ModelTask::Rerank,
        ];
        assert_eq!(
            tasks.map(model_task_order),
            [0_u8, 1_u8, 2_u8, 3_u8, 4_u8, 5_u8, 6_u8, 7_u8, 8_u8]
        );
    }

    #[test]
    fn endpoints_are_same_origin_unless_explicitly_allowed() {
        validate_credentialed_target_url(
            "https://api.example.com/v1",
            false,
            "wss://api.example.com/realtime",
            "realtime_endpoint",
            ProtocolTransportKind::Websocket,
            false,
        )
        .unwrap();
        assert!(
            validate_credentialed_target_url(
                "https://api.example.com/v1",
                false,
                "https://media.example.com/jobs",
                "endpoint",
                ProtocolTransportKind::Http,
                false,
            )
            .is_err()
        );
        validate_credentialed_target_url(
            "https://api.example.com/v1",
            true,
            "https://media.example.com/jobs",
            "endpoint",
            ProtocolTransportKind::Http,
            false,
        )
        .unwrap();
    }

    #[test]
    fn connection_roots_reject_queries_while_http_endpoints_allow_them() {
        assert!(validate_base_url("https://api.example.com/v1?tenant=one").is_err());
        let mut chat = capability(ModelTask::Chat, "openai.chat_text");
        chat.endpoint = Some("/chat/completions?tenant=one".into());
        validate_capability_urls(&chat, "https://api.example.com/v1").unwrap();
    }

    #[test]
    fn websocket_capability_accepts_relative_endpoint_and_ws_base_override() {
        let mut realtime = capability(
            ModelTask::RealtimeConversation,
            "stepfun.realtime_s2s",
        );
        realtime.realtime_endpoint = Some("/realtime?model={model}".into());
        validate_capability_urls(&realtime, "https://api.stepfun.com/v1").unwrap();

        realtime.base_url_override = Some("wss://api.stepfun.com/v1".into());
        validate_capability_urls(&realtime, "https://api.stepfun.com/v1").unwrap();
        assert!(validate_capability_urls(&realtime, "https://unused.example/v1").is_err());
        realtime.allow_cross_origin_credentials = true;
        validate_capability_urls(&realtime, "https://unused.example/v1").unwrap();

        realtime.realtime_endpoint =
            Some("https://api.stepfun.com/v1/realtime?model={model}".into());
        realtime.allow_cross_origin_credentials = false;
        validate_capability_urls(&realtime, "https://api.stepfun.com/v1").unwrap();
    }

    #[test]
    fn http_base_override_requires_same_origin_or_explicit_acknowledgement() {
        let mut chat = capability(ModelTask::Chat, "openai.chat_text");
        chat.base_url_override = Some("https://api.example.com/v2".into());
        validate_capability_urls(&chat, "https://api.example.com/v1").unwrap();
        chat.base_url_override = Some("https://gateway.example.com/v1".into());
        assert!(validate_capability_urls(&chat, "https://api.example.com/v1").is_err());
        chat.allow_cross_origin_credentials = true;
        validate_capability_urls(&chat, "https://api.example.com/v1").unwrap();
    }

    #[test]
    fn relative_endpoints_preserve_the_connection_version_root() {
        let base = Url::parse("https://api.example.com/v1").unwrap();
        for endpoint in ["chat/completions", "/chat/completions"] {
            let resolved = resolve_http_endpoint(&base, endpoint, "endpoint").unwrap();
            assert_eq!(resolved.as_str(), "https://api.example.com/v1/chat/completions");
        }
        for endpoint in ["realtime?model=x", "/realtime?model=x"] {
            let resolved = resolve_realtime_endpoint(&base, endpoint).unwrap();
            assert_eq!(resolved.as_str(), "https://api.example.com/v1/realtime?model=x");
        }
    }

    #[test]
    fn sdk_capability_rejects_transport_urls() {
        let mut bedrock = capability(ModelTask::Chat, "bedrock.anthropic_messages");
        validate_capability_urls(&bedrock, "").unwrap();
        assert!(validate_capability_urls(&bedrock, "https://runtime.example").is_err());
        bedrock.endpoint = Some("/converse".into());
        assert!(validate_capability_urls(&bedrock, "").is_err());
    }

    #[test]
    fn protocol_auth_validation_honors_exact_and_parameterized_schemes() {
        let chat = capability(ModelTask::Chat, "openai.chat_text");
        validate_capability_auth_scheme(&chat, "bearer").unwrap();
        assert!(validate_capability_auth_scheme(&chat, "header_key:x-api-key").is_err());

        let speech = capability(ModelTask::SpeechSynthesis, "openai.audio_speech");
        validate_capability_auth_scheme(&speech, "header_key:x-api-key").unwrap();
        validate_capability_auth_scheme(&speech, "query_key:key").unwrap();

        let anthropic = capability(ModelTask::Chat, "anthropic.messages");
        validate_capability_auth_scheme(&anthropic, "header_key:X-API-Key").unwrap();

        let gemini = capability(ModelTask::Chat, "gemini.generate_text");
        validate_capability_auth_scheme(&gemini, "header_key:X-Goog-Api-Key").unwrap();
        assert!(validate_capability_auth_scheme(&gemini, "bearer").is_err());

        let bedrock = capability(ModelTask::Chat, "bedrock.anthropic_messages");
        validate_capability_auth_scheme(&bedrock, "bedrock").unwrap();
        assert!(validate_capability_auth_scheme(&bedrock, "bearer").is_err());
    }

    #[test]
    fn async_endpoint_overrides_preserve_manifest_placeholders() {
        let mut video = capability(ModelTask::VideoGeneration, "openai.videos");
        video.poll_endpoint = Some("videos/{id}".into());
        video.content_endpoint = Some("videos/{id}/content".into());
        validate_capability_urls(&video, "https://api.example.com/v1").unwrap();

        video.poll_endpoint = Some("videos/static".into());
        assert!(validate_capability_urls(&video, "https://api.example.com/v1").is_err());
        video.poll_endpoint = Some("videos/{request_id}".into());
        assert!(validate_capability_urls(&video, "https://api.example.com/v1").is_err());

        let mut fixed_poll = capability(ModelTask::VideoGeneration, "siliconflow.video_jobs");
        fixed_poll.poll_endpoint = Some("video/status".into());
        validate_capability_urls(&fixed_poll, "https://api.example.com/v1").unwrap();

        let mut zhipu = capability(ModelTask::VideoGeneration, "zhipu.video_jobs");
        zhipu.poll_endpoint = Some("async-result/{task_id}".into());
        validate_capability_urls(&zhipu, "https://open.bigmodel.cn/api/paas/v4").unwrap();
        zhipu.poll_endpoint = Some("async-result/{request_id}".into());
        assert!(validate_capability_urls(&zhipu, "https://open.bigmodel.cn/api/paas/v4").is_err());
    }
}

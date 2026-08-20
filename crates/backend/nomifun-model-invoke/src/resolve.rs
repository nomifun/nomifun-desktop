//! Canonical task-scoped catalog resolution.
//!
//! Every runtime path resolves exactly one
//! `provider_model_capabilities(provider_id, model, task)` row. Protocol,
//! connection role, transport and provider request parameters have no other
//! source and there is deliberately no compatibility or platform fallback.

use std::sync::Arc;

use nomifun_api_types::{ModelTask, ModelTrait, validate_model_traits_unique};
use nomifun_common::ProviderId;
use nomifun_db::models::Provider;

use crate::adapter::ProtocolAdapter;
use crate::auth::{AuthMaterial, AuthScheme};
use crate::call::{
    CredentialedUrlKind, ResolvedCall, ResolvedConnection, ResolvedTaskConfig,
    ResolvedTaskTransport, validate_credentialed_url,
};
use crate::error::{InvokeError, InvokeErrorKind};
use crate::realtime::{RealtimeProtocolAdapter, ResolvedRealtimeCall};
use crate::service::ModelInvokeService;
use crate::types::{ModelRef, TaskRequest};
use crate::manifest::{
    ProtocolEndpointPurpose, ProtocolExecutorKind, ProtocolTransportKind,
    expand_protocol_endpoint_template, protocol_descriptor, protocol_task_descriptor,
    validate_endpoint_template,
};

const MAX_RESOLUTION_ATTEMPTS: usize = 3;

fn catalog_err(what: &str, error: nomifun_db::DbError) -> InvokeError {
    InvokeError::catalog(format!("{what}: {error}"))
}

fn task_key(task: ModelTask) -> String {
    serde_json::to_value(task)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("ModelTask serializes as a string")
}

fn strict_json_object(raw: &str, field: &str) -> Result<serde_json::Value, InvokeError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| InvokeError::config(format!("{field} is invalid JSON: {error}")))?;
    if !value.is_object() {
        return Err(InvokeError::config(format!("{field} must be a JSON object")));
    }
    Ok(value)
}

fn decrypt_credentials_payload(
    encrypted: &str,
    encryption_key: &[u8; 32],
    field: &str,
) -> Result<serde_json::Value, InvokeError> {
    let decrypted = nomifun_common::decrypt_string(encrypted, encryption_key)
        .map_err(|error| InvokeError::config(format!("failed to decrypt {field}: {error}")))?;
    strict_json_object(&decrypted, field)
}

fn parse_provider_params(
    raw: &str,
    protocol: &str,
    task: ModelTask,
) -> Result<serde_json::Value, InvokeError> {
    let value = strict_json_object(raw, "capability provider_params")?;
    crate::manifest::validate_provider_params_for_protocol(protocol, task, &value)?;
    Ok(value)
}

fn parse_capability_traits(raw: &str) -> Result<Vec<ModelTrait>, InvokeError> {
    let traits: Vec<ModelTrait> = serde_json::from_str(raw).map_err(|error| {
        InvokeError::config(format!("capability traits is invalid JSON: {error}"))
    })?;
    validate_model_traits_unique(&traits).map_err(InvokeError::config)?;
    Ok(traits)
}

fn auth_scheme_key(scheme: &AuthScheme) -> String {
    match scheme {
        AuthScheme::Bearer => "bearer".to_owned(),
        AuthScheme::TokenHeader => "token".to_owned(),
        AuthScheme::HeaderKey(name) => format!("header_key:{}", name.to_ascii_lowercase()),
        AuthScheme::QueryKey(name) => format!("query_key:{}", name.to_ascii_lowercase()),
        AuthScheme::MultiHeader(_) => "volc_voice".to_owned(),
        AuthScheme::Bedrock => "bedrock".to_owned(),
    }
}

fn validate_protocol_auth(protocol: &str, scheme: &AuthScheme) -> Result<(), InvokeError> {
    let descriptor = protocol_descriptor(protocol)
        .ok_or_else(|| InvokeError::config(format!("unknown protocol {protocol:?}")))?;
    let actual = auth_scheme_key(scheme);
    let allowed = descriptor.allowed_auth_schemes.iter().any(|candidate| {
        let candidate = candidate.trim().to_ascii_lowercase();
        candidate == actual
            || (candidate == "header_key:<name>" && actual.starts_with("header_key:"))
            || (candidate == "query_key:<param>" && actual.starts_with("query_key:"))
    });
    if allowed {
        Ok(())
    } else {
        Err(InvokeError::config(format!(
            "protocol {protocol:?} does not accept connection auth scheme {actual:?}"
        )))
    }
}

fn endpoint_kind(raw: &str, default: CredentialedUrlKind) -> CredentialedUrlKind {
    match reqwest::Url::parse(raw.trim()).ok().map(|url| url.scheme().to_owned()) {
        Some(scheme) if matches!(scheme.as_str(), "ws" | "wss") => {
            CredentialedUrlKind::WebSocket
        }
        Some(scheme) if matches!(scheme.as_str(), "http" | "https") => {
            CredentialedUrlKind::Http
        }
        _ => default,
    }
}

fn validate_connection_root(
    raw: &str,
    transport: ProtocolTransportKind,
) -> Result<(), InvokeError> {
    if transport == ProtocolTransportKind::Sdk {
        return if raw.trim().is_empty() {
            Ok(())
        } else {
            Err(InvokeError::config(
                "SDK protocols require an empty connection base URL",
            ))
        };
    }
    let parsed = reqwest::Url::parse(raw.trim())
        .map_err(|_| InvokeError::config("selected connection base URL must be absolute"))?;
    let scheme_ok = match transport {
        ProtocolTransportKind::Http => matches!(parsed.scheme(), "http" | "https"),
        ProtocolTransportKind::Websocket => {
            matches!(parsed.scheme(), "http" | "https" | "ws" | "wss")
        }
        ProtocolTransportKind::Sdk => true,
    };
    if !scheme_ok
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(InvokeError::config(
            "selected connection base URL has an invalid scheme/host or contains userinfo/query/fragment",
        ));
    }
    Ok(())
}

fn apply_base_url_override(
    connection: &mut ResolvedConnection,
    override_url: Option<&str>,
    transport: ProtocolTransportKind,
    allow_cross_origin_credentials: bool,
) -> Result<(), InvokeError> {
    let Some(override_url) = override_url else {
        return Ok(());
    };
    if transport == ProtocolTransportKind::Sdk {
        return Err(InvokeError::config(
            "SDK protocols do not accept a capability base_url_override",
        ));
    }
    let kind = endpoint_kind(
        override_url,
        match transport {
            ProtocolTransportKind::Websocket => CredentialedUrlKind::WebSocket,
            ProtocolTransportKind::Http | ProtocolTransportKind::Sdk => {
                CredentialedUrlKind::Http
            }
        },
    );
    validate_credentialed_url(
        connection,
        allow_cross_origin_credentials,
        override_url,
        "base_url_override",
        kind,
        false,
    )?;
    connection.base_url = override_url.trim().trim_end_matches('/').to_owned();
    Ok(())
}

fn validate_transport(
    connection: &ResolvedConnection,
    transport: &ResolvedTaskTransport,
    protocol: &str,
    task: ModelTask,
    protocol_transport: ProtocolTransportKind,
) -> Result<(), InvokeError> {
    if protocol_transport == ProtocolTransportKind::Sdk {
        if transport.endpoint.is_some()
            || transport.poll_endpoint.is_some()
            || transport.content_endpoint.is_some()
            || transport.realtime_endpoint.is_some()
        {
            return Err(InvokeError::config(
                "SDK protocols do not accept HTTP or WebSocket endpoint fields",
            ));
        }
        return Ok(());
    }

    for (field, endpoint) in [
        ("endpoint", transport.endpoint.as_deref()),
        ("poll_endpoint", transport.poll_endpoint.as_deref()),
        ("content_endpoint", transport.content_endpoint.as_deref()),
    ] {
        if let Some(endpoint) = endpoint {
            validate_endpoint_template(protocol, task, field, endpoint)?;
            let default_kind = match protocol_transport {
                ProtocolTransportKind::Websocket => CredentialedUrlKind::WebSocket,
                ProtocolTransportKind::Http | ProtocolTransportKind::Sdk => {
                    CredentialedUrlKind::Http
                }
            };
            validate_credentialed_url(
                connection,
                transport.allow_cross_origin_credentials,
                endpoint,
                field,
                endpoint_kind(endpoint, default_kind),
                true,
            )?;
        }
    }
    if let Some(endpoint) = transport.realtime_endpoint.as_deref() {
        validate_endpoint_template(protocol, task, "realtime_endpoint", endpoint)?;
        validate_credentialed_url(
            connection,
            transport.allow_cross_origin_credentials,
            endpoint,
            "realtime_endpoint",
            endpoint_kind(endpoint, CredentialedUrlKind::WebSocket),
            true,
        )?;
    }
    Ok(())
}

impl ModelInvokeService {
    /// Resolve the one authoritative task capability. Missing models/tasks,
    /// blank protocol/role, malformed params and incompatible descriptors all
    /// fail closed; no name heuristic or platform preset participates.
    pub async fn resolve_task_config(
        &self,
        model_ref: &ModelRef,
        task: ModelTask,
    ) -> Result<ResolvedTaskConfig, InvokeError> {
        let provider_id = ProviderId::parse(model_ref.provider_id.as_str()).map_err(|error| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "provider_id {:?} is not a canonical ProviderId: {error}",
                    model_ref.provider_id
                ),
            )
        })?;

        for _ in 0..MAX_RESOLUTION_ATTEMPTS {
            let start = self
                .provider_repo
                .find_by_id(provider_id.as_str())
                .await
                .map_err(|error| catalog_err("failed to read provider revision", error))?
                .ok_or_else(|| InvokeError::config(format!("provider not found: {provider_id}")))?;
            if !start.enabled {
                return Err(InvokeError::config(format!(
                    "provider disabled: {}",
                    start.name
                )));
            }
            let start_revision = start.config_revision;
            let result = self.resolve_task_config_once(model_ref, task).await;
            let end = self
                .provider_repo
                .find_by_id(provider_id.as_str())
                .await
                .map_err(|error| catalog_err("failed to verify provider revision", error))?;
            let stable = end
                .as_ref()
                .is_some_and(|provider| provider.enabled && provider.config_revision == start_revision);
            if stable {
                match &result {
                    Ok(config) if config.config_revision != start_revision => continue,
                    _ => return result,
                }
            }
        }

        Err(InvokeError::config(
            "provider invocation graph changed repeatedly while resolving the task; retry the request",
        ))
    }

    async fn resolve_task_config_once(
        &self,
        model_ref: &ModelRef,
        task: ModelTask,
    ) -> Result<ResolvedTaskConfig, InvokeError> {
        let provider_id = ProviderId::parse(model_ref.provider_id.as_str()).map_err(|error| {
            InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "provider_id {:?} is not a canonical ProviderId: {error}",
                    model_ref.provider_id
                ),
            )
        })?;
        let provider = self
            .provider_repo
            .find_by_id(provider_id.as_str())
            .await
            .map_err(|error| catalog_err("failed to read provider", error))?
            .ok_or_else(|| InvokeError::config(format!("provider not found: {provider_id}")))?;
        if !provider.enabled {
            return Err(InvokeError::config(format!(
                "provider disabled: {}",
                provider.name
            )));
        }

        let model = self
            .provider_model_repo
            .get(provider_id.as_str(), &model_ref.model)
            .await
            .map_err(|error| catalog_err("failed to read model", error))?
            .ok_or_else(|| {
                InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("model not in catalog: {}", model_ref.model),
                )
            })?;
        if !model.enabled {
            return Err(InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("model disabled: {}", model_ref.model),
            ));
        }

        let capability = self
            .provider_model_capability_repo
            .get(provider_id.as_str(), &model_ref.model, &task_key(task))
            .await
            .map_err(|error| catalog_err("failed to read model capability", error))?
            .ok_or_else(|| {
                InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!(
                        "model {:?} has no configured capability for task {task:?}",
                        model_ref.model
                    ),
                )
            })?;

        let protocol = capability.protocol.trim();
        if protocol.is_empty() {
            return Err(InvokeError::config(
                "model capability protocol must be explicit and non-empty",
            ));
        }
        let role = capability.connection_role.trim();
        if role.is_empty() {
            return Err(InvokeError::config(
                "model capability connection_role must be explicit and non-empty",
            ));
        }
        let descriptor = protocol_task_descriptor(protocol, task).ok_or_else(|| {
            InvokeError::config(format!(
                "capability declares unknown or task-incompatible protocol {protocol:?} for {task:?}"
            ))
        })?;

        let traits = parse_capability_traits(&capability.traits)?;
        let provider_params = parse_provider_params(&capability.provider_params, protocol, task)?;
        let mut connection = if role == "default" {
            self.default_connection(&provider)?
        } else {
            self.role_connection(provider_id.as_str(), role).await?
        };
        validate_connection_root(&connection.base_url, descriptor.transport)?;
        apply_base_url_override(
            &mut connection,
            capability.base_url_override.as_deref(),
            descriptor.transport,
            capability.allow_cross_origin_credentials,
        )?;
        validate_connection_root(&connection.base_url, descriptor.transport)?;
        validate_protocol_auth(protocol, &connection.auth.scheme)?;

        if descriptor.transport == ProtocolTransportKind::Sdk
            && connection.auth.scheme != AuthScheme::Bedrock
        {
            return Err(InvokeError::config(
                "SDK Chat capability requires an explicit bedrock connection auth scheme",
            ));
        }
        if descriptor.transport != ProtocolTransportKind::Sdk
            && connection.auth.scheme == AuthScheme::Bedrock
        {
            return Err(InvokeError::config(
                "bedrock connection auth cannot be used by HTTP/WebSocket protocols",
            ));
        }

        let descriptor_default = |purpose| {
            descriptor
                .endpoints
                .iter()
                .find(|endpoint| endpoint.purpose == purpose)
                .map(|endpoint| endpoint.default_value.clone())
        };
        let transport = ResolvedTaskTransport {
            endpoint: capability
                .endpoint
                .or_else(|| descriptor_default(ProtocolEndpointPurpose::Submit)),
            poll_endpoint: capability
                .poll_endpoint
                .or_else(|| descriptor_default(ProtocolEndpointPurpose::Poll)),
            content_endpoint: capability
                .content_endpoint
                .or_else(|| descriptor_default(ProtocolEndpointPurpose::Content)),
            realtime_endpoint: capability
                .realtime_endpoint
                .or_else(|| descriptor_default(ProtocolEndpointPurpose::Session)),
            allow_cross_origin_credentials: capability.allow_cross_origin_credentials,
        };
        validate_transport(&connection, &transport, protocol, task, descriptor.transport)?;
        if capability.context_limit.is_some_and(|value| value <= 0) {
            return Err(InvokeError::config(
                "model capability context_limit must be positive",
            ));
        }
        if capability.output_limit.is_some_and(|value| value <= 0) {
            return Err(InvokeError::config(
                "model capability output_limit must be positive",
            ));
        }

        Ok(ResolvedTaskConfig {
            provider_id: provider.provider_id,
            config_revision: provider.config_revision,
            platform: provider.platform,
            model: model_ref.model.clone(),
            task,
            traits,
            protocol: protocol.to_owned(),
            connection,
            transport,
            provider_params,
            context_limit: capability.context_limit,
            output_limit: capability.output_limit,
            bedrock_config: provider.bedrock_config,
        })
    }

    pub(crate) async fn resolve(
        &self,
        model_ref: &ModelRef,
        task: ModelTask,
        request: TaskRequest,
    ) -> Result<(ResolvedCall, Arc<dyn ProtocolAdapter>), InvokeError> {
        let config = self.resolve_task_config(model_ref, task).await?;
        let descriptor = protocol_task_descriptor(&config.protocol, task).ok_or_else(|| {
            InvokeError::config(format!(
                "capability declares unknown or task-incompatible protocol {:?}",
                config.protocol
            ))
        })?;
        if !matches!(
            descriptor.executor,
            ProtocolExecutorKind::ModelInvoke | ProtocolExecutorKind::AsyncJob
        ) {
            return Err(InvokeError::config(format!(
                "protocol {:?} is not a one-shot model-invoke executor",
                config.protocol
            )));
        }
        if descriptor.transport != ProtocolTransportKind::Http {
            return Err(InvokeError::config(format!(
                "one-shot protocol {:?} must use HTTP transport",
                config.protocol
            )));
        }
        let submit_url = config.http_endpoint()?;
        let adapter = self.registry.get(&config.protocol, task)?;
        let mut model_params = config.execution_params();
        model_params
            .as_object_mut()
            .expect("execution params are an object")
            .insert("endpoint".into(), serde_json::Value::String(submit_url));
        let call = ResolvedCall {
            provider_id: config.provider_id,
            config_revision: config.config_revision,
            platform: config.platform,
            model: config.model,
            task,
            protocol: config.protocol,
            connection: config.connection,
            model_params,
            request,
        };
        Ok((call, adapter))
    }

    pub(crate) async fn resolve_realtime(
        &self,
        model_ref: &ModelRef,
    ) -> Result<(ResolvedRealtimeCall, Arc<dyn RealtimeProtocolAdapter>), InvokeError> {
        let task = ModelTask::RealtimeConversation;
        let config = self.resolve_task_config(model_ref, task).await?;
        let descriptor = protocol_task_descriptor(&config.protocol, task).ok_or_else(|| {
            InvokeError::config(format!(
                "capability declares unknown realtime protocol {:?}",
                config.protocol
            ))
        })?;
        if descriptor.executor != ProtocolExecutorKind::RealtimeSession
            || descriptor.transport != ProtocolTransportKind::Websocket
        {
            return Err(InvokeError::config(format!(
                "protocol {:?} is not a realtime WebSocket executor",
                config.protocol
            )));
        }
        let session_endpoint = config
            .transport
            .realtime_endpoint
            .as_deref()
            .ok_or_else(|| {
                InvokeError::config(format!(
                    "realtime protocol {:?} has no session endpoint",
                    config.protocol
                ))
            })?;
        let session_endpoint = expand_protocol_endpoint_template(
            &config.protocol,
            task,
            "realtime_endpoint",
            session_endpoint,
            &config.model,
        )?;
        validate_credentialed_url(
            &config.connection,
            config.transport.allow_cross_origin_credentials,
            &session_endpoint,
            "realtime_endpoint",
            endpoint_kind(&session_endpoint, CredentialedUrlKind::WebSocket),
            true,
        )?;
        let adapter = self.realtime_registry.get(&config.protocol)?;
        let mut model_params = config.execution_params();
        model_params.as_object_mut().expect("execution params are an object").insert(
            "realtime_endpoint".into(),
            serde_json::Value::String(session_endpoint),
        );
        let call = ResolvedRealtimeCall {
            provider_id: config.provider_id,
            platform: config.platform,
            model: config.model,
            protocol: config.protocol,
            connection: config.connection,
            model_params,
        };
        Ok((call, adapter))
    }

    fn default_connection(&self, provider: &Provider) -> Result<ResolvedConnection, InvokeError> {
        let scheme = AuthScheme::parse(&provider.auth_scheme)?;
        let credentials = decrypt_credentials_payload(
            &provider.credentials_encrypted,
            &self.encryption_key,
            "provider credentials",
        )?;
        let auth = AuthMaterial {
            scheme,
            credentials,
        };
        auth.validate_credentials()?;
        Ok(ResolvedConnection {
            role: "default".into(),
            base_url: provider.base_url.trim().trim_end_matches('/').to_owned(),
            auth,
            extra: serde_json::json!({}),
        })
    }

    async fn role_connection(
        &self,
        provider_id: &str,
        role: &str,
    ) -> Result<ResolvedConnection, InvokeError> {
        let row = self
            .provider_connection_repo
            .get(provider_id, role)
            .await
            .map_err(|error| catalog_err("failed to read connection profile", error))?
            .ok_or_else(|| {
                InvokeError::new(
                    InvokeErrorKind::MissingConnection,
                    format!(
                        "connection profile {role:?} is not configured for this provider"
                    ),
                )
            })?;
        let credentials = decrypt_credentials_payload(
            &row.credentials_encrypted,
            &self.encryption_key,
            "named connection credentials",
        )?;
        let auth = AuthMaterial {
            scheme: AuthScheme::parse(&row.auth_scheme)?,
            credentials,
        };
        auth.validate_credentials()?;
        Ok(ResolvedConnection {
            role: row.role,
            base_url: row.base_url.trim().trim_end_matches('/').to_owned(),
            auth,
            extra: strict_json_object(&row.extra, "connection extra")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderConnectionRepository,
        IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
        NewProviderModel, NewProviderModelCapability, SqliteProviderConnectionRepository,
        SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
        SqliteProviderRepository, init_database_memory,
    };

    struct ScriptedProviderRepository {
        provider: Provider,
        revisions: std::sync::Mutex<VecDeque<i64>>,
        find_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl IProviderRepository for ScriptedProviderRepository {
        async fn list(&self) -> Result<Vec<Provider>, nomifun_db::DbError> {
            Ok(vec![self.provider.clone()])
        }

        async fn find_by_id(&self, id: &str) -> Result<Option<Provider>, nomifun_db::DbError> {
            self.find_calls.fetch_add(1, Ordering::SeqCst);
            if id != self.provider.provider_id {
                return Ok(None);
            }
            let revision = self
                .revisions
                .lock()
                .expect("revision script lock")
                .pop_front()
                .expect("revision script exhausted");
            let mut provider = self.provider.clone();
            provider.config_revision = revision;
            Ok(Some(provider))
        }

        async fn create(
            &self,
            _params: nomifun_db::CreateProviderParams<'_>,
            _initial_model: &nomifun_db::NewProviderModel<'_>,
            _connections: &[nomifun_db::UpsertProviderConnectionParams<'_>],
        ) -> Result<(Provider, nomifun_db::ProviderModelRow), nomifun_db::DbError> {
            unreachable!("scripted read-only repository")
        }

        async fn update(
            &self,
            _id: &str,
            _expected_config_revision: i64,
            _params: nomifun_db::UpdateProviderParams<'_>,
        ) -> Result<Provider, nomifun_db::DbError> {
            unreachable!("scripted read-only repository")
        }

        async fn clone_graph(
            &self,
            _source_provider_id: &str,
            _clone_name: &str,
        ) -> Result<Provider, nomifun_db::DbError> {
            unreachable!("scripted read-only repository")
        }

        async fn save_managed_graph(
            &self,
            _params: nomifun_db::CreateProviderParams<'_>,
            _models: &[nomifun_db::NewProviderModel<'_>],
        ) -> Result<Provider, nomifun_db::DbError> {
            unreachable!("scripted read-only repository")
        }

        async fn delete(&self, _id: &str) -> Result<(), nomifun_db::DbError> {
            unreachable!("scripted read-only repository")
        }
    }

    async fn scripted_resolution_service(
        revisions: impl IntoIterator<Item = i64>,
    ) -> (ModelInvokeService, Arc<ScriptedProviderRepository>) {
        const KEY: [u8; 32] = [0x54; 32];
        const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000221";
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        let base_provider_repo = SqliteProviderRepository::new(pool.clone());
        let capabilities = [NewProviderModelCapability {
            task: "image_generation",
            traits: "[]",
            protocol: "openai.images",
            connection_role: "default",
            endpoint: Some("/images/generations"),
            provider_params: "{}",
            ..Default::default()
        }];
        let encrypted = encrypt_string(r#"{"api_keys":["test-key"]}"#, &KEY).unwrap();
        let (provider, _) = base_provider_repo
            .create(
                CreateProviderParams {
                    provider_id: Some(PROVIDER_ID),
                    platform: "unrelated",
                    name: "scripted resolver",
                    base_url: "https://api.example/v1",
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: None,
                },
                &NewProviderModel {
                    model: "image-model",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .unwrap();
        let scripted = Arc::new(ScriptedProviderRepository {
            provider,
            revisions: std::sync::Mutex::new(revisions.into_iter().collect()),
            find_calls: AtomicUsize::new(0),
        });
        let provider_repo: Arc<dyn IProviderRepository> = scripted.clone();
        let model_repo: Arc<dyn IProviderModelRepository> =
            Arc::new(SqliteProviderModelRepository::new(pool.clone()));
        let capability_repo: Arc<dyn IProviderModelCapabilityRepository> =
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone()));
        let connection_repo: Arc<dyn IProviderConnectionRepository> =
            Arc::new(SqliteProviderConnectionRepository::new(pool));
        (
            ModelInvokeService::new(
                provider_repo,
                model_repo,
                capability_repo,
                connection_repo,
                KEY,
                reqwest::Client::new(),
                crate::adapter::AdapterRegistry::new(crate::adapters::default_adapters()),
            ),
            scripted,
        )
    }

    #[test]
    fn provider_params_reject_transport_shadow_keys() {
        for raw in [
            r#"{"protocol":"openai.chat_text"}"#,
            r#"{"endpoint":"/chat/completions"}"#,
            r#"{"base_url_override":"https://example.test"}"#,
            r#"{"api_key":"secret"}"#,
        ] {
            assert!(
                parse_provider_params(raw, "openai.chat_text", ModelTask::Chat).is_err(),
                "accepted: {raw}"
            );
        }
        assert!(
            parse_provider_params(
                r#"{"temperature":0.2}"#,
                "openai.chat_text",
                ModelTask::Chat,
            )
            .is_ok()
        );
    }

    #[test]
    fn provider_params_are_strict_json_objects() {
        for raw in ["not json", "[]", "null"] {
            assert!(
                parse_provider_params(raw, "openai.chat_text", ModelTask::Chat).is_err()
            );
        }
    }

    #[test]
    fn protocol_auth_manifest_is_enforced_at_runtime() {
        assert!(validate_protocol_auth("openai.chat_text", &AuthScheme::Bearer).is_ok());
        assert!(
            validate_protocol_auth(
                "openai.chat_text",
                &AuthScheme::HeaderKey("x-api-key".into())
            )
            .is_err()
        );
        assert!(
            validate_protocol_auth(
                "openai.images",
                &AuthScheme::HeaderKey("x-provider-key".into())
            )
            .is_ok()
        );
        assert!(validate_protocol_auth("volc.tts_v3", &AuthScheme::Bearer).is_err());
    }

    #[test]
    fn sdk_transport_requires_an_empty_connection_root() {
        validate_connection_root("", ProtocolTransportKind::Sdk).unwrap();
        assert!(
            validate_connection_root(
                "https://bedrock-runtime.us-east-1.amazonaws.com",
                ProtocolTransportKind::Sdk
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn resolver_retries_the_whole_graph_when_revision_changes_mid_read() {
        let (service, repo) = scripted_resolution_service([1, 1, 2, 2, 2, 2]).await;
        let config = service
            .resolve_task_config(
                &ModelRef {
                    provider_id: "0190f5fe-7c00-7a00-8000-000000000221".into(),
                    model: "image-model".into(),
                },
                ModelTask::ImageGeneration,
            )
            .await
            .unwrap();
        assert_eq!(config.config_revision, 2);
        assert_eq!(repo.find_calls.load(Ordering::SeqCst), 6);
    }

    #[tokio::test]
    async fn resolver_fails_closed_when_the_graph_never_stabilizes() {
        let (service, repo) =
            scripted_resolution_service([1, 1, 2, 2, 2, 3, 3, 3, 4]).await;
        let result = service
            .resolve_task_config(
                &ModelRef {
                    provider_id: "0190f5fe-7c00-7a00-8000-000000000221".into(),
                    model: "image-model".into(),
                },
                ModelTask::ImageGeneration,
            )
            .await;
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("an unstable invocation graph must not resolve"),
        };
        assert_eq!(error.kind, InvokeErrorKind::Config);
        assert!(error.message.contains("changed repeatedly"));
        assert_eq!(repo.find_calls.load(Ordering::SeqCst), 9);
    }
}

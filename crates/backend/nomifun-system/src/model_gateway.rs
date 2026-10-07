//! Optional community-operated gateway. All inference remains in existing
//! task capabilities; this module only validates and imports its control plane.
use std::collections::HashSet;
use std::time::Duration;

use nomifun_api_types::*;
use nomifun_common::AppError;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

pub const PLATFORM: &str = "nomifun-model-gateway";
const MAX_CONTROL_BODY: usize = 2 * 1024 * 1024;
// These limits enter the existing model editor/runtime DTOs as JSON numbers.
// Reject unsafe renderer integers rather than rounding an imported limit.
const MAX_RENDERER_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Accept either the API root or its exact `/v1` suffix. Never silently drop
/// query material, userinfo, fragments or arbitrary paths supplied by a user.
pub fn normalize_gateway_root(raw: &str) -> Result<String, AppError> {
    let mut url = reqwest::Url::parse(raw.trim())
        .map_err(|_| AppError::BadRequest("Gateway address must be an absolute HTTP(S) URL".into()))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
        || !matches!(url.path().trim_end_matches('/'), "" | "/v1")
    {
        return Err(AppError::BadRequest("Gateway address must be an HTTP(S) root without credentials, query, fragment or extra path".into()));
    }
    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn validate_key(key: &str) -> Result<&str, AppError> {
    let key = key.trim();
    if key.is_empty() || key.chars().any(char::is_control)
        || reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")).is_err()
    {
        return Err(AppError::BadRequest("A valid gateway API key is required".into()));
    }
    Ok(key)
}

fn invalid(message: &str) -> AppError {
    AppError::BadGateway(format!("Invalid model gateway v1 response: {message}"))
}

fn required(value: &Value, keys: &[&str]) -> Result<(), AppError> {
    let object = value.as_object().ok_or_else(|| invalid("expected an object"))?;
    if keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(invalid("a required field is missing"));
    }
    Ok(())
}

fn version(version: &str) -> Result<(), AppError> {
    if version != "1.0" { return Err(invalid("unsupported contract_version")); }
    Ok(())
}

fn nonblank(value: &str) -> Result<(), AppError> {
    if value.trim().is_empty() { return Err(invalid("blank required text")); }
    Ok(())
}

fn unique_text(values: &[String]) -> Result<(), AppError> {
    let mut seen = HashSet::new();
    for value in values { nonblank(value)?; if !seen.insert(value) { return Err(invalid("duplicate list value")); } }
    Ok(())
}

fn https_link(link: &Option<String>) -> Result<(), AppError> {
    if let Some(link) = link {
        let url = reqwest::Url::parse(link).map_err(|_| invalid("operator link must be absolute HTTPS"))?;
        if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty()
            || url.password().is_some() { return Err(invalid("operator link must be absolute HTTPS without credentials")); }
    }
    Ok(())
}

async fn fetch_value(root: &str, path: &str, key: Option<&str>) -> Result<Value, AppError> {
    let root = normalize_gateway_root(root)?;
    let address = reqwest::Url::parse(&root).map_err(|_| invalid("invalid normalized root"))?;
    let is_loopback = address.host_str().is_some_and(|host| host == "localhost"
        || host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()));
    let client = (if is_loopback {
        reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none()).build()
    } else { nomifun_net::http_client_no_redirect() })
        .map_err(|_| AppError::Internal("Cannot initialize gateway HTTP client".into()))?;
    let mut request = client.get(format!("{root}{path}"))
        .timeout(Duration::from_secs(20)).header("accept", "application/json");
    if let Some(key) = key { request = request.bearer_auth(validate_key(key)?); }
    let mut response = request.send().await.map_err(|error| {
        if error.is_timeout() { AppError::Timeout("Gateway control request timed out".into()) }
        else { AppError::BadGateway("Could not connect to the gateway".into()) }
    })?;
    if !response.status().is_success() {
        // Never echo a remote body: an operator can reflect submitted secrets.
        return Err(match response.status().as_u16() {
            401 => AppError::Unauthorized("Gateway rejected the API key or it expired".into()),
            403 => AppError::Forbidden("Gateway denied access; check the operator account and subscription".into()),
            402 => AppError::Forbidden("Gateway balance is insufficient; open the operator purchase page".into()),
            429 => AppError::BadGateway("Gateway rate limit reached; try again later".into()),
            _ => AppError::BadGateway(format!("Gateway control request returned HTTP {}", response.status().as_u16())),
        });
    }
    if response.content_length().is_some_and(|n| n > MAX_CONTROL_BODY as u64) {
        return Err(invalid("response exceeds size limit"));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| invalid("cannot read response"))? {
        if body.len().saturating_add(chunk.len()) > MAX_CONTROL_BODY { return Err(invalid("response exceeds size limit")); }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| invalid("expected JSON"))
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, AppError> {
    serde_json::from_value(value).map_err(|_| invalid("invalid typed fields"))
}

pub async fn fetch_meta(base_url: &str) -> Result<ModelGatewayMetaResponse, AppError> {
    let value = fetch_value(base_url, "/nomifun/v1/meta", None).await?;
    required(&value, &["contract_version", "operator", "capabilities", "optional_endpoints"])?;
    required(&value["operator"], &["name", "homepage_url", "console_url", "purchase_url", "terms_url", "privacy_url"])?;
    let meta: ModelGatewayMetaResponse = decode(value)?;
    version(&meta.contract_version)?; nonblank(&meta.operator.name)?;
    unique_text(&meta.capabilities)?; unique_text(&meta.optional_endpoints)?;
    for link in [&meta.operator.homepage_url, &meta.operator.console_url, &meta.operator.purchase_url, &meta.operator.terms_url, &meta.operator.privacy_url] { https_link(link)?; }
    Ok(meta)
}

#[derive(Deserialize)]
struct CatalogWire { contract_version: String, models: Vec<ModelGatewayCatalogModel> }

pub async fn fetch_catalog(base_url: &str, api_key: &str) -> Result<ModelGatewayCatalogResponse, AppError> {
    let value = fetch_value(base_url, "/nomifun/v1/catalog", Some(api_key)).await?;
    parse_catalog(value)
}

pub fn parse_catalog(value: Value) -> Result<ModelGatewayCatalogResponse, AppError> {
    required(&value, &["contract_version", "models"])?;
    for model in value["models"].as_array().ok_or_else(|| invalid("models must be an array"))? {
        required(model, &["id", "display_name", "vendor", "tasks", "task_endpoints", "context_window", "max_output_tokens", "input_modalities", "traits", "pricing", "included_in_plan", "status"])?;
    }
    let catalog: CatalogWire = decode(value)?;
    version(&catalog.contract_version)?;
    let mut seen = HashSet::new();
    let mut imports = Vec::with_capacity(catalog.models.len());
    for model in &catalog.models {
        if !seen.insert(&model.id) { return Err(invalid("duplicate model id")); }
        imports.push(map_model(model)?);
    }
    Ok(ModelGatewayCatalogResponse { contract_version: catalog.contract_version, models: catalog.models, imports })
}

fn endpoint_protocol(task: ModelTask, endpoint: &str) -> Result<(&'static str, &'static str), AppError> {
    use ModelTask::*;
    Ok(match (task, endpoint) {
        (Chat, "openai") => ("openai.chat_text", "default"),
        (Chat, "openai-response") => ("openai.responses", "default"),
        (Chat, "anthropic") => ("anthropic.messages", "anthropic"),
        (Chat, "gemini") => ("gemini.generate_text", "gemini"),
        (ImageGeneration | ImageEdit, "image-generation") => ("openai.images", "default"),
        (Embedding, "embeddings") => ("openai.embeddings", "default"),
        (Rerank, "jina-rerank") => ("generic.rerank", "default"),
        (SpeechSynthesis, "audio-speech") => ("openai.audio_speech", "default"),
        (SpeechRecognition, "audio-transcription") => ("openai.audio_transcriptions", "default"),
        _ => return Err(invalid("unknown or task-incompatible preferred endpoint")),
    })
}

fn map_model(model: &ModelGatewayCatalogModel) -> Result<ProviderModelInput, AppError> {
    nonblank(&model.id)?; nonblank(&model.display_name)?; nonblank(&model.vendor)?;
    crate::provider_model::validate_display_name(Some(&model.display_name))?;
    if !matches!(model.status.as_str(), "available" | "degraded" | "unavailable") { return Err(invalid("unknown model status")); }
    if model.context_window.is_some_and(|n| n <= 0 || n > MAX_RENDERER_SAFE_INTEGER)
        || model.max_output_tokens.is_some_and(|n| n <= 0 || n > MAX_RENDERER_SAFE_INTEGER)
    { return Err(invalid("token limits must be positive safe renderer integers or null")); }
    unique_text(&model.input_modalities)?; unique_text(&model.traits)?;
    let mut tasks = HashSet::new();
    let mut capabilities = Vec::new();
    for task in &model.tasks {
        if !tasks.insert(*task) { return Err(invalid("duplicate model task")); }
        let task_wire = serde_json::to_value(task).map_err(|_| invalid("invalid task"))?;
        let task_wire = task_wire.as_str().ok_or_else(|| invalid("invalid task"))?;
        let mapping = model.task_endpoints.get(task_wire).ok_or_else(|| invalid("task has no explicit preferred endpoint"))?;
        unique_text(&mapping.endpoints)?;
        if mapping.endpoints.is_empty() || !mapping.endpoints.contains(&mapping.preferred_endpoint) { return Err(invalid("preferred endpoint must belong to declared endpoints")); }
        for endpoint in &mapping.endpoints { endpoint_protocol(*task, endpoint)?; }
        let (protocol, role) = endpoint_protocol(*task, &mapping.preferred_endpoint)?;
        if protocol == "anthropic.messages" && model.max_output_tokens.is_none() { return Err(invalid("Anthropic requires max_output_tokens")); }
        let traits = if *task == ModelTask::Chat {
            let mut traits = Vec::new();
            for (trait_name, modality, typed) in [("vision_input", "image", ModelTrait::VisionInput), ("audio_input", "audio", ModelTrait::AudioInput), ("video_input", "video", ModelTrait::VideoInput), ("web_search", "", ModelTrait::WebSearch)] {
                if model.traits.iter().any(|t| t == trait_name) || (!modality.is_empty() && model.input_modalities.iter().any(|m| m == modality)) { traits.push(typed); }
            }
            traits
        } else { vec![] };
        capabilities.push(ProviderModelCapabilityInput {
            task: *task, traits, protocol: protocol.into(), connection_role: role.into(),
            base_url_override: None, endpoint: None, poll_endpoint: None, content_endpoint: None, realtime_endpoint: None,
            allow_cross_origin_credentials: false, provider_params: json!({}),
            context_limit: model.context_window, output_limit: model.max_output_tokens,
            compaction_threshold_pct: None,
        });
    }
    if tasks.is_empty() || model.task_endpoints.len() != tasks.len() { return Err(invalid("tasks and task_endpoints must match exactly")); }
    let mut prices = HashSet::new();
    for price in &model.pricing {
        if !tasks.contains(&price.task) || price.unit_size <= 0 || price.amount < 0 || !currency(&price.currency) || price.meter.trim().is_empty()
            || !prices.insert((price.task, &price.meter, &price.currency)) { return Err(invalid("invalid or duplicate price")); }
    }
    let mut input = ProviderModelInput { model: model.id.clone(), display_name: Some(model.display_name.clone()), enabled: true, description: None, sort_order: None, capabilities };
    set_baseline(&mut input)?;
    Ok(input)
}

fn currency(raw: &str) -> bool { raw.len() == 3 && raw.bytes().all(|ch| ch.is_ascii_uppercase()) }

pub async fn fetch_account(base_url: &str, api_key: &str) -> Result<ModelGatewayAccountResponse, AppError> {
    let value = fetch_value(base_url, "/nomifun/v1/account", Some(api_key)).await?;
    required(&value, &["contract_version", "plan", "balance", "key", "rate_limits"])?;
    required(&value["key"], &["name", "expires_at", "quota_unit", "remaining_quota"])?;
    required(&value["balance"], &["amount", "currency"])?;
    required(&value["rate_limits"], &["requests_per_minute", "tokens_per_minute", "concurrent_requests"])?;
    if !value["plan"].is_null() {
        required(&value["plan"], &["name", "period_start", "period_end", "quota"])?;
        required(&value["plan"]["quota"], &["unit", "total", "used"])?;
    }
    let account: ModelGatewayAccountResponse = decode(value)?;
    version(&account.contract_version)?;
    nonblank(&account.key.name)?; nonblank(&account.key.quota_unit)?;
    if !currency(&account.balance.currency) { return Err(invalid("invalid currency")); }
    for value in [account.key.remaining_quota, account.rate_limits.requests_per_minute, account.rate_limits.tokens_per_minute, account.rate_limits.concurrent_requests] {
        if value.is_some_and(|n| n < 0) { return Err(invalid("quota and limits cannot be negative")); }
    }
    validate_time(account.key.expires_at.as_deref())?;
    if let Some(plan) = &account.plan {
        nonblank(&plan.name)?; nonblank(&plan.quota.unit)?;
        if plan.quota.used < 0 || plan.quota.total.is_some_and(|n| n < 0) { return Err(invalid("invalid plan quota")); }
        validate_time(plan.period_start.as_deref())?; validate_time(plan.period_end.as_deref())?;
        if let (Some(start), Some(end)) = (&plan.period_start, &plan.period_end) {
            if chrono::DateTime::parse_from_rfc3339(start).unwrap() > chrono::DateTime::parse_from_rfc3339(end).unwrap() { return Err(invalid("plan end precedes start")); }
        }
    }
    Ok(account)
}

fn validate_time(value: Option<&str>) -> Result<(), AppError> {
    if let Some(value) = value {
        if !value.ends_with('Z') || chrono::DateTime::parse_from_rfc3339(value).is_err()
        { return Err(invalid("timestamps must be UTC RFC3339")); }
    }
    Ok(())
}

impl crate::provider::ProviderService {
    async fn require_gateway(&self, provider_id: &str) -> Result<nomifun_db::models::Provider, AppError> {
        nomifun_common::ProviderId::parse(provider_id).map_err(|_| AppError::BadRequest("Invalid provider id".into()))?;
        let provider = self.repo.find_by_id(provider_id).await?.ok_or_else(|| AppError::NotFound("Model gateway provider not found".into()))?;
        if provider.platform != PLATFORM { return Err(AppError::BadRequest("Provider is not a NomiFun model gateway".into())); }
        normalize_gateway_root(&provider.base_url)?;
        Ok(provider)
    }

    fn gateway_key(&self, provider: &nomifun_db::models::Provider) -> Result<String, AppError> {
        let credentials = crate::provider_connection::decrypt_credentials(&provider.credentials_encrypted, &self.encryption_key)?;
        let material = nomifun_model_invoke::AuthMaterial { scheme: nomifun_model_invoke::AuthScheme::Bearer, credentials };
        material.primary_secret().map_err(|_| AppError::BadRequest("Gateway API key is not configured".into()))
    }

    pub async fn gateway_meta(&self, provider_id: &str) -> Result<ModelGatewayMetaResponse, AppError> {
        let provider = self.require_gateway(provider_id).await?;
        fetch_meta(&provider.base_url).await
    }

    pub async fn gateway_account(&self, provider_id: &str) -> Result<ModelGatewayAccountResponse, AppError> {
        let provider = self.require_gateway(provider_id).await?;
        fetch_account(&provider.base_url, &self.gateway_key(&provider)?).await
    }

    pub async fn create_gateway(&self, req: CreateModelGatewayRequest) -> Result<ProviderResponse, AppError> {
        let root = normalize_gateway_root(&req.base_url)?;
        let key = validate_key(&req.api_key)?;
        if req.name.trim().is_empty() || req.name.chars().count() > 128 { return Err(AppError::BadRequest("Gateway provider name must contain 1-128 characters".into())); }
        // Validate against a fresh authorized directory; client-supplied protocol
        // graphs never bypass gateway contract validation.
        fetch_meta(&root).await?;
        let catalog = fetch_catalog(&root, key).await?;
        let requested = req.models.iter().collect::<HashSet<_>>();
        if requested.is_empty() || requested.len() != req.models.len() { return Err(AppError::BadRequest("Select at least one distinct catalog model".into())); }
        let models = catalog.imports.into_iter().filter(|model| requested.contains(&model.model)).collect::<Vec<_>>();
        if models.len() != requested.len() { return Err(AppError::BadRequest("Selected models are no longer present in the authorized catalog".into())); }
        let credentials = json!({"api_keys":[key]});
        let prepared = gateway_connections(&root, &credentials).iter().map(|connection| crate::provider_connection::prepare_new_connection(connection, &self.encryption_key)).collect::<Result<Vec<_>,_>>()?;
        let targets = crate::provider::unique_connection_targets(&prepared)?;
        let default_url = format!("{root}/v1");
        for model in &models { crate::provider::validate_capability_set(PLATFORM, &default_url, "bearer", &targets, &model.capabilities)?; }
        let serialized = models.iter().map(|model| crate::provider_model::serialize_capabilities(&model.capabilities)).collect::<Result<Vec<_>,_>>()?;
        let db_caps = serialized.iter().map(|caps| caps.iter().map(|cap| cap.as_db()).collect::<Vec<_>>()).collect::<Vec<_>>();
        let db_models = models.iter().zip(&db_caps).enumerate().map(|(index,(model,capabilities))| nomifun_db::NewProviderModel {
            model: &model.model, enabled: model.enabled, sort_order: index as i64, description: model.description.as_deref(), capabilities,
        }).collect::<Vec<_>>();
        let display_names = models.iter().map(|model| model.display_name.clone()).collect::<Vec<_>>();
        let connections = prepared.iter().map(crate::provider_connection::PreparedProviderConnection::as_db).collect::<Vec<_>>();
        let encrypted = crate::provider_connection::encrypt_credentials(&credentials, &self.encryption_key)?;
        let (provider, _) = self.repo.create_graph(nomifun_db::CreateProviderParams {
            provider_id: None, platform: PLATFORM, name: req.name.trim(), base_url: &default_url, auth_scheme: "bearer", credentials_encrypted: &encrypted, enabled: true, bedrock_config: None, sort_order: None,
        }, &db_models, &display_names, &connections).await?;
        self.gateway_response(provider).await
    }

    async fn gateway_response(&self, provider: nomifun_db::models::Provider) -> Result<ProviderResponse, AppError> {
        let models = crate::provider_model::rows_to_model_responses(self.model_repo.list_for_provider(&provider.provider_id).await?, self.capability_repo.list_for_provider(&provider.provider_id).await?)?;
        self.row_to_response(provider, models)
    }

    pub async fn update_gateway_connection(&self, provider_id: &str, req: UpdateModelGatewayConnectionRequest) -> Result<ProviderResponse, AppError> {
        let existing = self.require_gateway(provider_id).await?;
        let root = normalize_gateway_root(&req.base_url)?;
        let old_root = normalize_gateway_root(&existing.base_url)?;
        let supplied_key = req.api_key.as_deref().map(str::trim).filter(|value| !value.is_empty());
        if root != old_root && supplied_key.is_none() { return Err(AppError::BadRequest("Enter a key explicitly when changing the gateway address".into())); }
        let old_key = self.gateway_key(&existing)?;
        let key = validate_key(supplied_key.unwrap_or(&old_key))?;
        if let Some(name) = &req.name { if name.trim().is_empty() || name.chars().count() > 128 { return Err(AppError::BadRequest("Gateway provider name must contain 1-128 characters".into())); } }
        fetch_meta(&root).await?;
        fetch_catalog(&root, key).await?;
        let credentials = json!({"api_keys":[key]});
        let prepared = gateway_connections(&root, &credentials).iter().map(|connection| crate::provider_connection::prepare_new_connection(connection, &self.encryption_key)).collect::<Result<Vec<_>,_>>()?;
        let targets = crate::provider::unique_connection_targets(&prepared)?;
        let default_url = format!("{root}/v1");
        let models = crate::provider_model::rows_to_model_responses(self.model_repo.list_for_provider(provider_id).await?, self.capability_repo.list_for_provider(provider_id).await?)?;
        for model in &models {
            for cap in &model.capabilities {
                let input: ProviderModelCapabilityInput = decode(json!({"task":cap.task,"traits":cap.traits,"protocol":cap.protocol,"connection_role":cap.connection_role,"base_url_override":cap.base_url_override,"endpoint":cap.endpoint,"poll_endpoint":cap.poll_endpoint,"content_endpoint":cap.content_endpoint,"realtime_endpoint":cap.realtime_endpoint,"allow_cross_origin_credentials":cap.allow_cross_origin_credentials,"provider_params":cap.provider_params,"context_limit":cap.context_limit,"output_limit":cap.output_limit,"compaction_threshold_pct":cap.compaction_threshold_pct}))?;
                crate::provider::validate_capability(PLATFORM, &default_url, "bearer", &targets, &input)?;
            }
        }
        let encrypted = if key == old_key { existing.credentials_encrypted.clone() } else { crate::provider_connection::encrypt_credentials(&credentials, &self.encryption_key)? };
        let existing_connections = self.connection_repo.list_for_provider(provider_id).await?;
        let connections = prepared.iter().map(|prepared| {
            let mut connection = prepared.as_db();
            if let Some(stored) = existing_connections.iter().find(|stored| stored.role == prepared.role()) {
                if crate::provider_connection::decrypt_credentials(&stored.credentials_encrypted, &self.encryption_key).is_ok_and(|stored| stored == credentials) {
                    connection.credentials_encrypted = &stored.credentials_encrypted;
                }
            }
            connection
        }).collect::<Vec<_>>();
        let provider = self.repo.update_with_connections(provider_id, existing.config_revision, nomifun_db::UpdateProviderParams {
            name: req.name.as_deref().map(str::trim), base_url: Some(&default_url), auth_scheme: Some("bearer"), credentials_encrypted: Some(&encrypted), ..Default::default()
        }, &connections).await?;
        self.gateway_response(provider).await
    }

    pub async fn sync_gateway_catalog(&self, provider_id: &str) -> Result<SyncModelGatewayResponse, AppError> {
        let provider = self.require_gateway(provider_id).await?;
        let catalog = fetch_catalog(&provider.base_url, &self.gateway_key(&provider)?).await?;
        let existing = crate::provider_model::rows_to_model_responses(self.model_repo.list_for_provider(provider_id).await?, self.capability_repo.list_for_provider(provider_id).await?)?;
        let mut next_sort = existing.iter().map(|model| model.sort_order).max().unwrap_or(-1).saturating_add(1);
        let mut added = 0; let mut updated = 0; let mut changes = Vec::new();
        for mut model in catalog.imports {
            if let Some(current) = existing.iter().find(|current| current.model == model.model) {
                if let Some(merged) = merge_import(current, &model)? { changes.push(merged); updated += 1; }
            } else {
                model.sort_order = Some(next_sort); next_sort = next_sort.saturating_add(1);
                changes.push(model); added += 1;
            }
        }
        let connections = self.connection_repo.list_for_provider(provider_id).await?;
        let targets = connections.into_iter().map(|connection| (connection.role, crate::provider::ConnectionTarget { base_url: connection.base_url, auth_scheme: connection.auth_scheme })).collect();
        for model in &changes { crate::provider::validate_capability_set(PLATFORM, &provider.base_url, &provider.auth_scheme, &targets, &model.capabilities)?; }
        if !changes.is_empty() {
            let serialized = changes.iter().map(|model| crate::provider_model::serialize_capabilities(&model.capabilities)).collect::<Result<Vec<_>,_>>()?;
            let db_caps = serialized.iter().map(|caps| caps.iter().map(|cap| cap.as_db()).collect::<Vec<_>>()).collect::<Vec<_>>();
            let db_models = changes.iter().zip(&db_caps).map(|(model, capabilities)| nomifun_db::NewProviderModel { model: &model.model, enabled: model.enabled, sort_order: model.sort_order.unwrap_or(0), description: model.description.as_deref(), capabilities }).collect::<Vec<_>>();
            let display_names = changes.iter().map(|model| model.display_name.clone()).collect::<Vec<_>>();
            self.repo.save_graph_models(provider_id, provider.config_revision, &db_models, &display_names).await?;
        }
        let models = crate::provider_model::rows_to_model_responses(self.model_repo.list_for_provider(provider_id).await?, self.capability_repo.list_for_provider(provider_id).await?)?;
        Ok(SyncModelGatewayResponse { added, updated, models })
    }
}

pub fn gateway_connections(root: &str, credentials: &Value) -> Vec<ProviderConnectionInput> {
    [("anthropic", "header_key:x-api-key"), ("gemini", "header_key:x-goog-api-key")].into_iter()
        .map(|(role, scheme)| ProviderConnectionInput { role: role.into(), label: None, base_url: root.into(), auth_scheme: scheme.into(), credentials: credentials.clone(), extra: None }).collect()
}

fn clean_model_value(model: &ProviderModelInput) -> Result<Value, AppError> {
    let mut value = serde_json::to_value(model).map_err(|_| invalid("cannot serialize import baseline"))?;
    if let Some(capabilities) = value["capabilities"].as_array_mut() {
        for capability in capabilities { if let Some(params) = capability["provider_params"].as_object_mut() { params.remove(MODEL_GATEWAY_CATALOG_BASELINE_PARAM); } }
    }
    // Ordering and enabled state are user decisions, never synchronized.
    value.as_object_mut().unwrap().remove("sort_order");
    value.as_object_mut().unwrap().remove("enabled");
    Ok(value)
}

pub fn set_baseline(model: &mut ProviderModelInput) -> Result<(), AppError> {
    let baseline = clean_model_value(model)?;
    for capability in &mut model.capabilities {
        capability.provider_params.as_object_mut().ok_or_else(|| invalid("provider params must be an object"))?
            .insert(MODEL_GATEWAY_CATALOG_BASELINE_PARAM.into(), baseline.clone());
    }
    Ok(())
}

/// Field-by-field three-way merge. Rows without provenance are user-authored
/// and left intact; missing remote rows/capabilities are never removed.
pub fn merge_import(existing: &ProviderModelResponse, incoming: &ProviderModelInput) -> Result<Option<ProviderModelInput>, AppError> {
    let baseline = existing.capabilities.iter().find_map(|cap| cap.provider_params.get(MODEL_GATEWAY_CATALOG_BASELINE_PARAM)).cloned();
    let Some(baseline) = baseline else { return Ok(None); };
    let mut current: ProviderModelInput = decode(json!({"model":existing.model,"display_name":existing.display_name,"enabled":existing.enabled,"description":existing.description,"sort_order":existing.sort_order,"capabilities":existing.capabilities.iter().map(|cap| json!({
        "task":cap.task,"traits":cap.traits,"protocol":cap.protocol,"connection_role":cap.connection_role,
        "base_url_override":cap.base_url_override,"endpoint":cap.endpoint,"poll_endpoint":cap.poll_endpoint,"content_endpoint":cap.content_endpoint,"realtime_endpoint":cap.realtime_endpoint,
        "allow_cross_origin_credentials":cap.allow_cross_origin_credentials,"provider_params":cap.provider_params,"context_limit":cap.context_limit,"output_limit":cap.output_limit,"compaction_threshold_pct":cap.compaction_threshold_pct
    })).collect::<Vec<_>>()}))?;
    let before = clean_model_value(&current)?;
    let next = clean_model_value(incoming)?;
    let mut merged = before.clone();
    for key in ["display_name", "description"] {
        if before.get(key) == baseline.get(key) { if let Some(value) = next.get(key) { merged[key] = value.clone(); } else { merged.as_object_mut().unwrap().remove(key); } }
    }
    let previous_caps = baseline["capabilities"].as_array().ok_or_else(|| invalid("invalid stored import provenance"))?;
    let next_caps = next["capabilities"].as_array().unwrap();
    for cap in merged["capabilities"].as_array_mut().unwrap() {
        let original = cap.clone();
        let task = cap["task"].clone();
        let Some(old) = previous_caps.iter().find(|c| c["task"] == task) else { continue; };
        let Some(new) = next_caps.iter().find(|c| c["task"] == task) else { continue; };
        let transport_fields = ["protocol", "connection_role", "base_url_override", "endpoint", "poll_endpoint", "content_endpoint", "realtime_endpoint", "allow_cross_origin_credentials"];
        let transport_overridden = transport_fields.iter().any(|field| original.get(*field) != old.get(*field));
        for (key, value) in new.as_object().unwrap() {
            if transport_overridden && transport_fields.contains(&key.as_str()) { continue; }
            if cap.get(key) == old.get(key) { cap[key] = value.clone(); }
        }
        // A protocol/role/required-limit change must stay coherent with a
        // manual override. Preserve that capability when only part of the
        // remote transition would otherwise make it unusable.
        let candidate: ProviderModelCapabilityInput = decode(cap.clone())?;
        let auth = match candidate.connection_role.as_str() {
            "anthropic" => "header_key:x-api-key",
            "gemini" => "header_key:x-goog-api-key",
            _ => "bearer",
        };
        if crate::provider_model::validate_protocol(PLATFORM, &candidate).is_err()
            || crate::provider_model::validate_capability_auth_scheme(&candidate, auth).is_err()
        { *cap = original; }
    }
    let current_tasks = before["capabilities"].as_array().unwrap().iter().map(|c| c["task"].to_string()).collect::<HashSet<_>>();
    let previous_tasks = previous_caps.iter().map(|c| c["task"].to_string()).collect::<HashSet<_>>();
    if current_tasks == previous_tasks {
        for cap in next_caps { if !current_tasks.contains(&cap["task"].to_string()) { merged["capabilities"].as_array_mut().unwrap().push(cap.clone()); } }
    }
    // Advance each remote baseline even when a user override was preserved.
    merged["enabled"] = json!(existing.enabled); merged["sort_order"] = json!(existing.sort_order);
    current = decode(merged)?;
    let incoming_baseline = clean_model_value(incoming)?;
    if clean_model_value(&current)? == before && baseline == incoming_baseline { return Ok(None); }
    for cap in &mut current.capabilities { cap.provider_params.as_object_mut().unwrap().insert(MODEL_GATEWAY_CATALOG_BASELINE_PARAM.into(), incoming_baseline.clone()); }
    Ok(Some(current))
}

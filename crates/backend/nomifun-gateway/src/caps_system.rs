//! System-domain capabilities (registry form): desktop settings, client
//! preferences (theme / zoom / keep-awake / feature toggles), model-provider
//! CRUD, model fetching, and read-only system info.
//!
//! These tools let the LLM agent configure the desktop environment on behalf
//! of the user — the headline use case is "set my theme to dark" / "add a
//! new provider" / "change my zoom level" spoken to the companion.
//!
//! SKIPPED tools (listed at the bottom of this file) need extra CompatibilityCapabilityHost
//! fields the parent has not yet wired:
//! - `nomi_system_check_update` — needs `VersionCheckService`
//! - `nomi_system_factory_reset` — needs `data_dir: PathBuf`

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use nomifun_api_types::{
    CreateProviderRequest, FetchModelsRequest, ProviderModelInput, UpdateProviderRequest,
    UpdateSettingsRequest,
};
use nomifun_common::ProviderId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::deps::CompatibilityCapabilityHost;
use crate::registry::{Capability, CapabilityMeta, EffectClass};
use crate::server::ok;

// ── param structs (single source: schema + runtime) ──────────────────────

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetSettingsParams {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateSettingsParams {
    /// System language code. Allowed: "en-US" or "zh-CN".
    #[serde(default)]
    language: Option<String>,
    /// Enable/disable desktop notifications globally.
    #[serde(default)]
    notification_enabled: Option<bool>,
    /// Enable/disable notifications specifically for cron-job results.
    #[serde(default)]
    cron_notification_enabled: Option<bool>,
    /// Enable/disable the command queue (batch-queued execution of LLM requests).
    #[serde(default)]
    command_queue_enabled: Option<bool>,
    /// Whether uploaded files should be saved to the current workspace.
    #[serde(default)]
    save_upload_to_workspace: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetPreferencesParams {
    /// Optional list of preference keys to fetch (omit to return all).
    /// Common keys: "theme", "ui.zoomFactor", "system.closeToTray",
    /// "companion.size", "system.keepAwake", "feature.*".
    #[serde(default)]
    keys: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdatePreferencesParams {
    /// Map of key → JSON value to set. A `null` value deletes the key.
    /// Keys must be non-empty and at most 255 characters.
    ///
    /// Common keys (non-exhaustive):
    ///   "theme" (string: "light" | "dark" | "rhythm-dark" | …),
    ///   "ui.zoomFactor" (number: 0.5–2.0),
    ///   "system.closeToTray" (bool),
    ///   "system.keepAwake" (bool),
    ///   "companion.size" (number: px),
    ///   "feature.<name>" (bool).
    preferences: HashMap<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateProviderParams {
    /// Provider platform identifier (e.g. "openai", "anthropic", "gemini",
    /// "new-api", "bedrock", "vertex-ai", "minimax", "dashscope-coding", etc.).
    platform: String,
    /// Human-readable display name for this provider.
    name: String,
    /// API base URL (must start with http:// or https://). Empty string allowed
    /// only for bedrock platform.
    base_url: String,
    /// Authentication transport, for example `bearer`,
    /// `header_key:x-api-key`, `query_key:key`, or `bedrock`.
    auth_scheme: String,
    /// Write-only typed credential material selected by `auth_scheme`, for
    /// example `{ "api_keys": ["sk-..."] }` for bearer/header schemes.
    credentials: Value,
    /// First fully configured model. A provider cannot be created without a
    /// usable task capability.
    initial_model: ProviderModelParams,
    /// Whether the provider is enabled (default true).
    #[serde(default)]
    enabled: Option<bool>,
    /// Optional AWS Bedrock configuration (required when platform = "bedrock").
    /// Pass the full BedrockConfig object as JSON.
    #[serde(default)]
    bedrock_config: Option<Value>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProviderModelParams {
    /// Exact model identifier accepted by the provider. Custom identifiers are allowed.
    model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sort_order: Option<i64>,
    /// Complete non-empty modality configuration for this model.
    capabilities: Vec<ProviderModelCapabilityParams>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProviderModelCapabilityParams {
    /// Modality task such as `chat`, `speech_synthesis`, `speech_recognition`,
    /// `realtime_conversation`, `image_generation`, or `video_generation`.
    task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    traits: Option<Vec<String>>,
    /// Exact invoke protocol supported by NomiFun for this provider/task.
    protocol: String,
    /// Provider connection role, normally `default`.
    connection_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    poll_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    realtime_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow_cross_origin_credentials: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_limit: Option<i64>,
    /// Declared maximum output tokens for this model/task. Required by
    /// Anthropic Messages protocols; omit to use the provider default where supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateProviderParams {
    /// Provider ID (from nomi_list_providers).
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    provider_id: ProviderId,
    /// New display name (omit to keep).
    #[serde(default)]
    name: Option<String>,
    /// New API base URL (omit to keep).
    #[serde(default)]
    base_url: Option<String>,
    /// New default authentication scheme (omit to keep).
    #[serde(default)]
    auth_scheme: Option<String>,
    /// New typed credential material (omit to keep the encrypted value).
    #[serde(default)]
    credentials: Option<Value>,
    /// Enable or disable (omit to keep).
    #[serde(default)]
    enabled: Option<bool>,
    /// AWS Bedrock configuration update (omit to keep).
    #[serde(default)]
    bedrock_config: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteProviderParams {
    /// Provider ID to permanently delete.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    provider_id: ProviderId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FetchModelsParams {
    /// Provider ID whose models to fetch from the remote API.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    provider_id: ProviderId,
    /// If true, attempt automatic URL correction on failure for
    /// OpenAI-compatible providers (probes common URL suffixes).
    #[serde(default)]
    try_fix: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetInfoParams {}

// ── handlers ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SystemCapabilityDeps {
    settings: nomifun_system::SettingsService,
    preferences: nomifun_system::ClientPrefService,
    providers: nomifun_system::ProviderService,
    model_fetch: nomifun_system::ModelFetchService,
}

fn adapt<P, F, Fut>(
    handler: F,
) -> impl Fn(Arc<CompatibilityCapabilityHost>, crate::deps::CallerCtx, P) -> Fut + Send + Sync + 'static
where
    P: Send + 'static,
    F: Fn(Arc<SystemCapabilityDeps>, P) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    move |deps, _ctx, params| {
        handler(
            Arc::new(SystemCapabilityDeps {
                settings: deps.settings_service.clone(),
                preferences: deps.client_pref_service.clone(),
                providers: deps.provider_service.clone(),
                model_fetch: deps.model_fetch_service.clone(),
            }),
            params,
        )
    }
}

async fn get_settings(deps: Arc<SystemCapabilityDeps>, _p: GetSettingsParams) -> Value {
    match deps.settings.get_settings().await {
        Ok(settings) => ok(settings),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn update_settings(deps: Arc<SystemCapabilityDeps>, p: UpdateSettingsParams) -> Value {
    let req = UpdateSettingsRequest {
        language: p.language,
        notification_enabled: p.notification_enabled,
        cron_notification_enabled: p.cron_notification_enabled,
        command_queue_enabled: p.command_queue_enabled,
        save_upload_to_workspace: p.save_upload_to_workspace,
    };
    if req.is_empty() {
        return json!({ "error": "nothing to update: provide at least one field" });
    }
    match deps.settings.update_settings(req).await {
        Ok(settings) => ok(settings),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn get_preferences(deps: Arc<SystemCapabilityDeps>, p: GetPreferencesParams) -> Value {
    let keys_owned = p.keys.unwrap_or_default();
    let keys_ref: Vec<&str> = keys_owned.iter().map(String::as_str).collect();
    let filter = if keys_ref.is_empty() { None } else { Some(keys_ref.as_slice()) };
    match deps.preferences.get_preferences(filter).await {
        Ok(prefs) => ok(prefs),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn update_preferences(deps: Arc<SystemCapabilityDeps>, p: UpdatePreferencesParams) -> Value {
    if p.preferences.is_empty() {
        return json!({ "error": "preferences map must not be empty" });
    }
    match deps.preferences.update_preferences(p.preferences).await {
        Ok(()) => ok(json!({ "updated": true })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn create_provider(deps: Arc<SystemCapabilityDeps>, p: CreateProviderParams) -> Value {
    // Map the bedrock_config Value passthrough into the typed struct.
    let bedrock_config = match p.bedrock_config {
        Some(val) => match serde_json::from_value(val) {
            Ok(cfg) => Some(cfg),
            Err(e) => return json!({ "error": format!("invalid bedrock_config: {e}") }),
        },
        None => None,
    };
    let initial_model: ProviderModelInput = match serde_json::to_value(p.initial_model)
        .and_then(serde_json::from_value)
    {
        Ok(model) => model,
        Err(error) => return json!({ "error": format!("invalid initial_model: {error}") }),
    };
    let req = CreateProviderRequest {
        provider_id: None,
        platform: p.platform,
        name: p.name,
        base_url: p.base_url,
        auth_scheme: p.auth_scheme,
        credentials: p.credentials,
        enabled: p.enabled.unwrap_or(true),
        bedrock_config,
        sort_order: None,
        initial_model,
        connections: Vec::new(),
    };
    match deps.providers.create(req).await {
        Ok(resp) => ok(json!({
            "provider_id": resp.provider_id,
            "platform": resp.platform,
            "name": resp.name,
            "base_url": resp.base_url,
            "has_credentials": resp.has_credentials,
            "models": resp.models,
            "enabled": resp.enabled,
            "note": "provider and its first fully configured model were created atomically",
        })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn update_provider(deps: Arc<SystemCapabilityDeps>, p: UpdateProviderParams) -> Value {
    let bedrock_config = match p.bedrock_config {
        Some(val) => match serde_json::from_value(val) {
            Ok(cfg) => Some(cfg),
            Err(e) => return json!({ "error": format!("invalid bedrock_config: {e}") }),
        },
        None => None,
    };
    let req = UpdateProviderRequest {
        name: p.name,
        base_url: p.base_url,
        auth_scheme: p.auth_scheme,
        credentials: p.credentials,
        enabled: p.enabled,
        bedrock_config,
        sort_order: None,
    };
    match deps.providers.update(p.provider_id.as_str(), req).await {
        Ok(resp) => ok(json!({
            "provider_id": resp.provider_id,
            "platform": resp.platform,
            "name": resp.name,
            "base_url": resp.base_url,
            "has_credentials": resp.has_credentials,
            "models": resp.models,
            "enabled": resp.enabled,
        })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn delete_provider(deps: Arc<SystemCapabilityDeps>, p: DeleteProviderParams) -> Value {
    match deps.providers.delete(p.provider_id.as_str()).await {
        Ok(()) => json!({ "result": format!("provider {} deleted", p.provider_id) }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn fetch_models(deps: Arc<SystemCapabilityDeps>, p: FetchModelsParams) -> Value {
    let req = FetchModelsRequest {
        try_fix: p.try_fix.unwrap_or(false),
    };
    match deps
        .model_fetch
        .fetch_models(p.provider_id.as_str(), &req)
        .await
    {
        Ok(resp) => {
            let mut result = json!({
                "models": resp.models,
                "count": resp.models.len(),
            });
            if let Some(fixed_url) = resp.fixed_base_url {
                result["fixed_base_url"] = json!(fixed_url);
                result["note"] = json!(
                    "the provider's base URL was auto-corrected; the new URL has been applied"
                );
            }
            ok(result)
        }
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn get_info(_deps: Arc<SystemCapabilityDeps>, _p: GetInfoParams) -> Value {
    let info = nomifun_system::sysinfo::get_system_info();
    ok(info)
}

// ── registration ─────────────────────────────────────────────────────────

/// Register the system-domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    // 1. Settings (read)
    out.push(Capability::new::<GetSettingsParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_get_settings",
            "system",
            "Read the desktop's system settings (language, notification toggles, etc.).",
            EffectClass::Read,
        ),
        adapt(get_settings),
    ));

    // 2. Settings (write)
    out.push(Capability::new::<UpdateSettingsParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_update_settings",
            "system",
            "Partially update system settings (language, notification toggles, command queue, workspace upload). Only provided fields are changed.",
            EffectClass::Write,
        ),
        adapt(update_settings),
    ));

    // 3. Preferences (read)
    out.push(Capability::new::<GetPreferencesParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_get_preferences",
            "system",
            "Read client preferences (theme, zoom, keep-awake, companion size, feature toggles, etc.). Omit keys to get all.",
            EffectClass::Read,
        ),
        adapt(get_preferences),
    ));

    // 4. Preferences (write) — the headline "set theme / zoom / keep-awake" tool
    out.push(Capability::new::<UpdatePreferencesParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_update_preferences",
            "system",
            "Batch set/delete client preferences (theme, ui.zoomFactor, system.closeToTray, system.keepAwake, companion.size, feature toggles). Pass null value to delete a key.",
            EffectClass::Write,
        ),
        adapt(update_preferences),
    ));

    // 5. Create provider (sensitive — handles credentials)
    out.push(Capability::new::<CreateProviderParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_create_provider",
            "system",
            "Register a model provider with one fully configured model. Credentials are typed according to auth_scheme, validated, and encrypted at rest.",
            EffectClass::Sensitive,
        ),
        adapt(create_provider),
    ));

    // 6. Update provider (sensitive — may update credentials)
    out.push(Capability::new::<UpdateProviderParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_update_provider",
            "system",
            "Partially update an existing model provider (name, URL, authentication, credentials, enabled). Only provided fields are changed.",
            EffectClass::Sensitive,
        ),
        adapt(update_provider),
    ));

    // 7. Delete provider (destructive)
    out.push(Capability::new::<DeleteProviderParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_delete_provider",
            "system",
            "Permanently delete a model provider and all its stored credentials.",
            EffectClass::Destructive,
        ),
        adapt(delete_provider),
    ));

    // 8. Fetch models (write — triggers a network call and may auto-fix the URL)
    out.push(Capability::new::<FetchModelsParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_fetch_models",
            "system",
            "Fetch the model list from a provider's remote API (by provider id). Use after creating a provider without specifying models.",
            EffectClass::Write,
        ),
        adapt(fetch_models),
    ));

    // 9. System info (read — pure, no service dependency beyond sysinfo)
    out.push(Capability::new::<GetInfoParams, _, _>(
        CapabilityMeta::new(
            "nomi_system_get_info",
            "system",
            "Read system info: data/cache/log directories, OS platform, and CPU architecture.",
            EffectClass::Read,
        ),
        adapt(get_info),
    ));
}

// ── SKIPPED tools ────────────────────────────────────────────────────────
//
// 10. `nomi_system_check_update` (Read)
//     Needs: `deps.version_check_service: nomifun_system::VersionCheckService`
//     Method: `version_check_service.check_update(&UpdateCheckRequest { .. })`
//     Not wired because VersionCheckService is not in the assumed CompatibilityCapabilityHost.
//
// 11. `nomi_system_factory_reset` (Destructive)
//     Needs: `deps.data_dir: PathBuf`
//     Method: `nomifun_common::factory_reset::request_v3_dataset_reset(&data_dir, &work_dir)`
//     Not wired because data_dir is not in the assumed CompatibilityCapabilityHost.

#[cfg(test)]
mod tests {
    use super::*;

    fn initial_model() -> Value {
        json!({
            "model": "gpt-test",
            "capabilities": [{
                "task": "chat",
                "protocol": "openai.chat_text",
                "connection_role": "default"
            }]
        })
    }

    #[test]
    fn create_provider_tool_accepts_only_typed_credentials() {
        let params: CreateProviderParams = serde_json::from_value(json!({
            "platform": "openai",
            "name": "OpenAI",
            "base_url": "https://api.openai.com/v1",
            "auth_scheme": "bearer",
            "credentials": {"api_keys": ["sk-test"]},
            "initial_model": initial_model()
        }))
        .unwrap();
        assert_eq!(params.credentials, json!({"api_keys": ["sk-test"]}));

        let legacy = serde_json::from_value::<CreateProviderParams>(json!({
            "platform": "openai",
            "name": "OpenAI",
            "base_url": "https://api.openai.com/v1",
            "auth_scheme": "bearer",
            "api_key": "sk-test",
            "initial_model": initial_model()
        }));
        assert!(legacy.is_err(), "the removed flat api_key contract must stay rejected");
    }

    #[test]
    fn update_provider_tool_uses_typed_optional_credentials() {
        let provider_id = ProviderId::new();
        let params: UpdateProviderParams = serde_json::from_value(json!({
            "provider_id": provider_id.as_str(),
            "credentials": {"api_keys": ["sk-next"]}
        }))
        .unwrap();
        assert_eq!(params.credentials, Some(json!({"api_keys": ["sk-next"]})));

        let legacy = serde_json::from_value::<UpdateProviderParams>(json!({
            "provider_id": provider_id.as_str(),
            "api_key": "sk-next"
        }));
        assert!(legacy.is_err(), "the removed flat api_key contract must stay rejected");
    }
}

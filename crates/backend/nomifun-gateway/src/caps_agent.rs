//! Agent-stack domain capabilities: agent catalog/health, custom agent CRUD,
//! remote agent management, and model failover configuration.
//!
//! Backed by:
//! - `nomifun_ai_agent::AgentService` — installed agent listing, health checks,
//!   custom agent CRUD, enable/disable.
//!   authentication, pairing, and connection testing.
//! - `nomifun_conversation::model_failover` — global model-failover config read/write
//!   (stored in `client_preferences` key `agent.model_failover`).
//!
//! NEW GatewayDeps fields assumed (parent wires):
//! - `agent_service: Arc<nomifun_ai_agent::AgentService>`
//! - `client_pref_repo: Arc<dyn nomifun_db::IClientPreferenceRepository>`

use std::sync::Arc;

use nomifun_api_types::{
    BehaviorPolicy, CustomAgentAdvancedOverrides, CustomAgentUpsertRequest, ModelFailoverConfig,
    ModelTask, ProviderHealthCheckRequest,
    TryConnectCustomAgentRequest,
};
use nomifun_common::{AgentId, ProviderId};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::GatewayDeps;
use crate::id_schema::ModelRefParam;
use crate::registry::{Capability, CapabilityMeta, DangerTier, Surface};
use crate::server::ok;

// ── param structs (single source: schema + runtime) ──────────────────────

/// List all installed agent backends with their status and metadata.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentListParams {}

/// Run an ACP health check against a specific agent backend.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentHealthCheckParams {
    /// The agent backend identifier to health-check (e.g. "claude", "codex").
    backend: String,
}

/// Run a Chat-capability health check for an Agent model.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentProviderHealthCheckParams {
    /// Provider id to test against.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    provider_id: ProviderId,
    /// Model name to probe (must be enabled on the provider).
    #[serde(deserialize_with = "crate::id_schema::deserialize_model_name")]
    model: String,
}

/// Enable or disable an agent backend.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentSetEnabledParams {
    /// Canonical agent_metadata.agent_id UUIDv7.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    agent_id: AgentId,
    /// Whether to enable (true) or disable (false) the agent.
    enabled: bool,
}

/// Create a custom (user-registered) agent backend.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentCustomCreateParams {
    /// Display name for the custom agent.
    name: String,
    /// CLI command to launch the agent process (absolute path or PATH-resolvable).
    command: String,
    /// Optional icon URL or data URI.
    #[serde(default)]
    icon: Option<String>,
    /// Extra CLI arguments passed after `command`.
    #[serde(default)]
    args: Vec<String>,
    /// Environment variables injected into the agent process.
    #[serde(default)]
    env: Vec<AgentEnvEntryParam>,
    /// Advanced behavior overrides (yolo_id, native_skills_dirs, behavior_policy, description).
    #[serde(default)]
    advanced: Option<CustomAgentAdvancedParam>,
}

/// Update an existing custom agent backend.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentCustomUpdateParams {
    /// The custom agent id to update.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    agent_id: AgentId,
    /// Display name for the custom agent.
    name: String,
    /// CLI command to launch the agent process.
    command: String,
    /// Optional icon URL or data URI.
    #[serde(default)]
    icon: Option<String>,
    /// Extra CLI arguments passed after `command`.
    #[serde(default)]
    args: Vec<String>,
    /// Environment variables injected into the agent process.
    #[serde(default)]
    env: Vec<AgentEnvEntryParam>,
    /// Advanced behavior overrides.
    #[serde(default)]
    advanced: Option<CustomAgentAdvancedParam>,
}

/// Delete a custom agent backend (irreversible).
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentCustomDeleteParams {
    /// The custom agent id to permanently delete.
    #[schemars(schema_with = "crate::id_schema::canonical_uuid_v7_schema")]
    agent_id: AgentId,
}

/// Test connectivity to a custom agent binary (try-connect handshake).
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentCustomTryConnectParams {
    /// CLI command to launch the agent process.
    command: String,
    /// ACP protocol arguments (if any).
    #[serde(default)]
    acp_args: Vec<String>,
    /// Environment variables for the test subprocess.
    #[serde(default)]
    env: std::collections::HashMap<String, String>,
}

/// An environment variable entry for custom agent configuration.
#[derive(Deserialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
struct AgentEnvEntryParam {
    /// Variable name.
    name: String,
    /// Variable value.
    value: String,
    /// Optional human-readable description of what this variable controls.
    #[serde(default)]
    description: Option<String>,
}

/// Fixed wire shape for the custom-agent advanced editor. Keep this local to
/// the gateway because capability schemas must be generated from types that
/// implement `JsonSchema`; the API crate intentionally remains schemars-free.
#[derive(Deserialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
struct CustomAgentAdvancedParam {
    #[serde(default)]
    yolo_id: Option<String>,
    #[serde(default)]
    native_skills_dirs: Option<Vec<String>>,
    #[serde(default)]
    behavior_policy: Option<BehaviorPolicyParam>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize, JsonSchema, Clone)]
#[serde(deny_unknown_fields)]
struct BehaviorPolicyParam {
    #[serde(default)]
    supports_side_question: bool,
    #[serde(default)]
    self_identity_sticky: bool,
    #[serde(default)]
    session_load_via_meta_field: bool,
}

impl From<BehaviorPolicyParam> for BehaviorPolicy {
    fn from(value: BehaviorPolicyParam) -> Self {
        Self {
            supports_side_question: value.supports_side_question,
            self_identity_sticky: value.self_identity_sticky,
            session_load_via_meta_field: value.session_load_via_meta_field,
        }
    }
}

impl From<CustomAgentAdvancedParam> for CustomAgentAdvancedOverrides {
    fn from(value: CustomAgentAdvancedParam) -> Self {
        Self {
            yolo_id: value.yolo_id,
            native_skills_dirs: value.native_skills_dirs,
            behavior_policy: value.behavior_policy.map(Into::into),
            description: value.description,
        }
    }
}

// ── Remote agent param structs ──────────────────────────────────────────

// ── Model failover param structs ────────────────────────────────────────

/// Read the global model-failover configuration.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelFailoverGetParams {}

/// Set the global model-failover configuration.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelFailoverSetParams {
    /// Whether model failover is enabled.
    enabled: bool,
    /// Ordered list of provider+model pairs to try on failure (first = primary fallback).
    /// Each entry has exactly `{ "provider_id": "...", "model": "..." }`.
    #[serde(default)]
    queue: Vec<ModelRefParam>,
    /// Maximum number of model switches per conversation turn (default: 4).
    #[serde(default = "default_max_switches")]
    max_switches: u32,
}

fn default_max_switches() -> u32 {
    4
}

// ── handlers ──────────────────────────────────────────────────────────────

async fn agent_list(deps: Arc<GatewayDeps>, _p: AgentListParams) -> Value {
    match deps.agent_service.list_agents().await {
        Ok(agents) => ok(agents),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_health_check(deps: Arc<GatewayDeps>, p: AgentHealthCheckParams) -> Value {
    let req = nomifun_api_types::AcpHealthCheckRequest {
        backend: p.backend,
    };
    match deps.agent_service.acp_health_check(req).await {
        Ok(resp) => ok(resp),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_provider_health_check(
    deps: Arc<GatewayDeps>,
    p: AgentProviderHealthCheckParams,
) -> Value {
    let req = ProviderHealthCheckRequest {
        provider_id: p.provider_id.into_string(),
        model: p.model,
        task: ModelTask::Chat,
    };
    match deps.agent_service.provider_health_check(req).await {
        Ok(resp) => ok(resp),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_set_enabled(deps: Arc<GatewayDeps>, p: AgentSetEnabledParams) -> Value {
    let agent_id = p.agent_id.into_string();
    match deps.agent_service.set_agent_enabled(&agent_id, p.enabled).await {
        Ok(meta) => ok(meta),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_custom_create(deps: Arc<GatewayDeps>, p: AgentCustomCreateParams) -> Value {
    let req = CustomAgentUpsertRequest {
        name: p.name,
        command: p.command,
        icon: p.icon,
        args: p.args,
        env: p
            .env
            .into_iter()
            .map(|e| nomifun_api_types::AgentEnvEntry {
                name: e.name,
                value: e.value,
                description: e.description,
            })
            .collect(),
        advanced: p.advanced.map(Into::into),
    };
    match deps.agent_service.create_custom_agent(req).await {
        Ok(meta) => ok(meta),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_custom_update(deps: Arc<GatewayDeps>, p: AgentCustomUpdateParams) -> Value {
    let req = CustomAgentUpsertRequest {
        name: p.name,
        command: p.command,
        icon: p.icon,
        args: p.args,
        env: p
            .env
            .into_iter()
            .map(|e| nomifun_api_types::AgentEnvEntry {
                name: e.name,
                value: e.value,
                description: e.description,
            })
            .collect(),
        advanced: p.advanced.map(Into::into),
    };
    match deps
        .agent_service
        .update_custom_agent(p.agent_id.as_str(), req)
        .await
    {
        Ok(meta) => ok(meta),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_custom_delete(deps: Arc<GatewayDeps>, p: AgentCustomDeleteParams) -> Value {
    match deps
        .agent_service
        .delete_custom_agent(p.agent_id.as_str())
        .await
    {
        Ok(()) => ok(json!({ "deleted": p.agent_id })),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn agent_custom_try_connect(
    deps: Arc<GatewayDeps>,
    p: AgentCustomTryConnectParams,
) -> Value {
    let req = TryConnectCustomAgentRequest {
        command: p.command,
        acp_args: p.acp_args,
        env: p.env,
    };
    match deps.agent_service.try_connect_custom_agent(req).await {
        Ok(resp) => ok(resp),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ── remote agent handlers ───────────────────────────────────────────────

// ── model failover handlers ─────────────────────────────────────────────

async fn model_failover_get(deps: Arc<GatewayDeps>, _p: ModelFailoverGetParams) -> Value {
    let cfg =
        nomifun_conversation::model_failover::get_global_failover_config(&deps.client_pref_repo)
            .await;
    ok(cfg)
}

async fn model_failover_set(deps: Arc<GatewayDeps>, p: ModelFailoverSetParams) -> Value {
    let cfg = ModelFailoverConfig {
        enabled: p.enabled,
        queue: p.queue.into_iter().map(Into::into).collect(),
        max_switches: p.max_switches,
    };

    match nomifun_conversation::model_failover::set_global_failover_config(
        &deps.client_pref_repo,
        &cfg,
    )
    .await
    {
        Ok(()) => ok(cfg),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ── registration ─────────────────────────────────────────────────────────

/// Register the agent-stack domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    // ─── Agent catalog ───────────────────────────────────────────────────

    // 1. List agents (Read)
    out.push(Capability::new::<AgentListParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_list",
            "agent",
            "List all installed agent backends with their availability status, type, and configuration.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| agent_list(deps, p),
    ));

    // 2. ACP health check (Read)
    out.push(Capability::new::<AgentHealthCheckParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_health_check",
            "agent",
            "Run an ACP health check against a specific agent backend to verify it is responsive.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| agent_health_check(deps, p),
    ));

    // 3. Provider health check (Read)
    out.push(Capability::new::<AgentProviderHealthCheckParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_provider_health_check",
            "agent",
            "Test model reachability through a specific provider (verify API key, model availability, latency).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| agent_provider_health_check(deps, p),
    ));

    // 4. Set agent enabled (Write)
    out.push(Capability::new::<AgentSetEnabledParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_set_enabled",
            "agent",
            "Enable or disable an agent backend. Disabled agents are not available for new conversations.",
            DangerTier::Write,
        ),
        |deps, _ctx, p| agent_set_enabled(deps, p),
    ));

    // ─── Custom agents ───────────────────────────────────────────────────

    // 5. Create custom agent (Write)
    out.push(Capability::new::<AgentCustomCreateParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_custom_create",
            "agent",
            "Register a new custom agent backend (user-provided CLI binary). The process will be launched on demand.",
            DangerTier::Write,
        ),
        |deps, _ctx, p| agent_custom_create(deps, p),
    ));

    // 6. Update custom agent (Write)
    out.push(Capability::new::<AgentCustomUpdateParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_custom_update",
            "agent",
            "Update an existing custom agent backend's configuration (name, command, args, env, advanced overrides).",
            DangerTier::Write,
        ),
        |deps, _ctx, p| agent_custom_update(deps, p),
    ));

    // 7. Delete custom agent (Destructive, deny_on Channel)
    out.push(Capability::new::<AgentCustomDeleteParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_custom_delete",
            "agent",
            "Permanently delete a custom agent backend registration. Running sessions using this agent will fail on next turn.",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        |deps, _ctx, p| agent_custom_delete(deps, p),
    ));

    // 8. Try-connect custom agent (Read — network probe, no state change)
    out.push(Capability::new::<AgentCustomTryConnectParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_custom_try_connect",
            "agent",
            "Test connectivity to a custom agent binary by spawning it and performing an ACP handshake (dry-run, no persistence).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| agent_custom_try_connect(deps, p),
    ));

    // ─── Model failover ──────────────────────────────────────────────────

    // 15. Get model failover config (Read)
    out.push(Capability::new::<ModelFailoverGetParams, _, _>(
        CapabilityMeta::new(
            "nomi_model_failover_get",
            "agent",
            "Read the global model-failover configuration (enabled flag, ordered queue of fallback provider+model pairs, max switches).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| model_failover_get(deps, p),
    ));

    // 16. Set model failover config (Write)
    out.push(Capability::new::<ModelFailoverSetParams, _, _>(
        CapabilityMeta::new(
            "nomi_model_failover_set",
            "agent",
            "Set the global model-failover configuration. Controls automatic fallback to alternative models when the primary provider fails.",
            DangerTier::Write,
        ),
        |deps, _ctx, p| model_failover_set(deps, p),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_enabled_accepts_only_canonical_agent_ids() {
        let canonical_id = AgentId::new().into_string();
        let parsed: AgentSetEnabledParams =
            serde_json::from_value(json!({ "agent_id": canonical_id, "enabled": true })).unwrap();
        assert_eq!(parsed.agent_id.as_str(), canonical_id);

        for invalid_id in [
            "claude",
            "nomi",
            "extension:agent",
            "agent_extension_demo",
        ] {
            assert!(
                serde_json::from_value::<AgentSetEnabledParams>(
                    json!({ "agent_id": invalid_id })
                )
                .is_err(),
                "set-enabled must reject non-UUIDv7 agent_id {invalid_id}"
            );
        }
    }

}

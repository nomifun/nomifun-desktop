//! Agent-stack domain capabilities: agent catalog listing and model
//! provider/failover configuration.
//!
//! Backed by:
//! - `nomifun_ai_agent::AgentService` — installed agent listing and model
//!   provider health checks.
//! - `nomifun_conversation::model_failover` — global model-failover config read/write
//!   (stored in `client_preferences` key `agent.model_failover`).
//!
//! NEW GatewayDeps fields assumed (parent wires):
//! - `agent_service: Arc<nomifun_ai_agent::AgentService>`
//! - `client_pref_repo: Arc<dyn nomifun_db::IClientPreferenceRepository>`

use std::sync::Arc;

use nomifun_api_types::{
    ModelFailoverConfig, ModelTask, ProviderHealthCheckRequest,
};
use nomifun_common::ProviderId;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::GatewayDeps;
use crate::id_schema::ModelRefParam;
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::ok;

// ── param structs (single source: schema + runtime) ──────────────────────

/// List all installed agent backends with their status and metadata.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentListParams {}

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

    // 2. Provider health check (Read)
    out.push(Capability::new::<AgentProviderHealthCheckParams, _, _>(
        CapabilityMeta::new(
            "nomi_agent_provider_health_check",
            "agent",
            "Test model reachability through a specific provider (verify API key, model availability, latency).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| agent_provider_health_check(deps, p),
    ));

    // ─── Model failover ──────────────────────────────────────────────────

    // 3. Get model failover config (Read)
    out.push(Capability::new::<ModelFailoverGetParams, _, _>(
        CapabilityMeta::new(
            "nomi_model_failover_get",
            "agent",
            "Read the global model-failover configuration (enabled flag, ordered queue of fallback provider+model pairs, max switches).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| model_failover_get(deps, p),
    ));

    // 4. Set model failover config (Write)
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

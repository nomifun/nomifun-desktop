//! IDMM (Intelligent Decision-Making Mode) capabilities (registry form):
//! read + set the per-session supervision config for a conversation or terminal
//! target.
//!
//! Clean migration of `tools_idmm.rs` onto the capability registry. The typed
//! params structs are now the single source (schema + runtime deserialization).
//! Handler logic is identical to the legacy: overlay onto the previously
//! persisted config so unexposed knobs (fault watch / strategy / budget) keep
//! their values; the gateway exposes the DECISION watch's core knobs
//! (enabled / tier / freeform policy).

use std::sync::Arc;

use nomifun_api_types::{IdmmTargetKind, WatchTier};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::{ok, require_user};

// ─── Params ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, JsonSchema)]
struct SetIdmmParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id to supervise.
    target_id: String,
    /// Enable (true) or disable (false) IDMM supervision.
    enabled: bool,
    /// Escalation tier: "rule" (no-LLM rules only, default) or
    /// "rule_plus_sidecar" (adds a backup-model sidecar; requires a
    /// steering_prompt and a configured backup provider).
    #[serde(default)]
    tier: Option<String>,
    /// Bounds what the sidecar may decide on the user's behalf. Required
    /// (non-empty) for the rule_plus_sidecar tier.
    #[serde(default)]
    steering_prompt: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GetIdmmParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id to inspect.
    target_id: String,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn parse_kind(raw: &str) -> Result<IdmmTargetKind, Value> {
    IdmmTargetKind::parse(raw)
        .ok_or_else(|| json!({"error": format!("unknown kind '{raw}' (expected conversation | terminal)")}))
}

/// Shared Gateway ownership boundary for every target-scoped IDMM capability.
/// Keeping this in one helper prevents the base and extended capability sets
/// from drifting into different authorization behavior.
pub(crate) async fn verify_target(
    deps: &GatewayDeps,
    ctx: &CallerCtx,
    kind: IdmmTargetKind,
    target_id: &str,
) -> Option<Value> {
    let user_id = match require_user(ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return Some(e),
    };
    match deps
        .idmm_service
        .verify_target_owner(kind, target_id, &user_id)
        .await
    {
        Ok(()) => None,
        Err(e) => Some(json!({"error": e.to_string()})),
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn set(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: SetIdmmParams) -> Value {
    let kind = match parse_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    if let Some(err) = verify_target(&deps, &ctx, kind, &p.target_id).await {
        return err;
    }

    // Overlay onto the previously persisted config so unexposed knobs
    // (fault watch / strategy / budget details) keep their values. The gateway
    // exposes the DECISION watch (the agent-facing decision capability).
    let mut cfg = match deps
        .idmm_service
        .read_config_persisted(ctx.user_id.as_str(), kind, &p.target_id)
        .await
    {
        Ok(c) => c.unwrap_or_default(),
        Err(e) => return json!({"error": e.to_string()}),
    };
    cfg.decision_watch.base.enabled = p.enabled;
    if let Some(tier) = p.tier.as_deref() {
        cfg.decision_watch.base.tier = match tier {
            "rule" | "rule_only" => WatchTier::RuleOnly,
            "rule_plus_sidecar" | "rule_plus_model" => WatchTier::RulePlusModel,
            other => {
                return json!({"error": format!("unknown tier '{other}' (expected rule_only | rule_plus_model)")});
            }
        };
    }
    if let Some(sp) = p.steering_prompt {
        cfg.decision_watch.strategy.freeform_policy = Some(sp);
    }

    if let Err(e) = deps
        .idmm_service
        .save_config(ctx.user_id.as_str(), kind, &p.target_id, &cfg)
        .await
    {
        // Typical validation errors: sidecar tier without a steering prompt
        // or without a resolvable backup provider — relay them verbatim so
        // the agent can fix the call or ask the owner.
        return json!({"error": e.to_string()});
    }
    match deps
        .idmm_service
        .build_state(ctx.user_id.as_str(), kind, &p.target_id)
        .await
    {
        Ok(state) => ok(state),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn get(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: GetIdmmParams) -> Value {
    let kind = match parse_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    if let Some(err) = verify_target(&deps, &ctx, kind, &p.target_id).await {
        return err;
    }
    match deps
        .idmm_service
        .build_state(ctx.user_id.as_str(), kind, &p.target_id)
        .await
    {
        Ok(state) => ok(state),
        Err(e) => json!({"error": e.to_string()}),
    }
}

// ─── Registration ────────────────────────────────────────────────────────────

/// Register the IDMM-domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<SetIdmmParams, _, _>(
        CapabilityMeta::new(
            "nomi_set_idmm",
            "idmm",
            "Update IDMM supervision knobs (enabled / tier / steering prompt) and (re)arm the live supervisor for a conversation or terminal.",
            DangerTier::Write,
        ),
        |deps, ctx, p| set(deps, ctx, p),
    ));
    out.push(Capability::new::<GetIdmmParams, _, _>(
        CapabilityMeta::new(
            "nomi_get_idmm",
            "idmm",
            "Read the current IDMM config and live supervision state for a conversation or terminal.",
            DangerTier::Read,
        ),
        |deps, ctx, p| get(deps, ctx, p),
    ));
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kind_accepts_valid_values() {
        assert_eq!(parse_kind("conversation").unwrap(), IdmmTargetKind::Conversation);
        assert_eq!(parse_kind("terminal").unwrap(), IdmmTargetKind::Terminal);
    }

    #[test]
    fn parse_kind_rejects_unknown() {
        let err = parse_kind("unknown").unwrap_err();
        let msg = err["error"].as_str().unwrap();
        assert!(msg.contains("unknown kind 'unknown'"));
        assert!(msg.contains("conversation | terminal"));
    }

    /// Verify the tier mapping accepts all documented aliases and rejects unknowns.
    #[test]
    fn tier_mapping_coverage() {
        fn map_tier(s: &str) -> Result<WatchTier, String> {
            match s {
                "rule" | "rule_only" => Ok(WatchTier::RuleOnly),
                "rule_plus_sidecar" | "rule_plus_model" => Ok(WatchTier::RulePlusModel),
                other => Err(format!("unknown tier '{other}'")),
            }
        }
        assert_eq!(map_tier("rule").unwrap(), WatchTier::RuleOnly);
        assert_eq!(map_tier("rule_only").unwrap(), WatchTier::RuleOnly);
        assert_eq!(map_tier("rule_plus_sidecar").unwrap(), WatchTier::RulePlusModel);
        assert_eq!(map_tier("rule_plus_model").unwrap(), WatchTier::RulePlusModel);
        assert!(map_tier("bogus").is_err());
    }
}

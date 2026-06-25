//! AutoWork-domain capabilities (registry form): enable/disable + inspect the
//! AutoWork binding for a conversation or terminal target.
//!
//! Mirrors `POST /api/requirements/autowork`: persist the config via
//! `RequirementService`, then start/stop the live orchestrator loop and
//! broadcast the state — a config write alone would only take effect after the
//! next desktop boot.

use std::sync::Arc;

use nomifun_api_types::{AutoWorkState, AutoWorkTargetKind};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::ok;

#[derive(Deserialize, JsonSchema)]
struct SetAutoworkParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id to bind.
    target_id: String,
    /// Enable (true) or disable (false) AutoWork on the target.
    enabled: bool,
    /// Requirement tag the session works through. REQUIRED when enabling.
    #[serde(default)]
    tag: Option<String>,
    /// Stop after this many completed requirements (omit for unlimited).
    #[serde(default)]
    max_requirements: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct GetAutoworkParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id to inspect.
    target_id: String,
}

fn parse_kind(raw: &str) -> Result<AutoWorkTargetKind, Value> {
    AutoWorkTargetKind::parse(raw)
        .ok_or_else(|| json!({ "error": format!("unknown kind '{raw}' (expected conversation | terminal)") }))
}

/// Parse the string `target_id` into the integer conversation id the requirement
/// service's owner check uses (the AutoWork target handle stays a string).
fn parse_conv_id(target_id: &str) -> Result<i64, nomifun_common::AppError> {
    target_id
        .parse::<i64>()
        .map_err(|_| nomifun_common::AppError::NotFound(format!("conversation {target_id}")))
}

/// Assemble the persisted config + the orchestrator's live view into one
/// `AutoWorkState` (the same shape the REST routes return and broadcast).
async fn build_state(deps: &GatewayDeps, kind: AutoWorkTargetKind, target_id: &str) -> Result<AutoWorkState, Value> {
    let (enabled, tag, _max) = deps
        .requirement_service
        .read_autowork_config(kind, target_id)
        .await
        .map_err(|e| json!({ "error": e.to_string() }))?;
    let running = deps.autowork_orchestrator.is_running(kind, target_id);
    let live_tag = deps.autowork_orchestrator.running_tag(kind, target_id).or(tag);
    let (current_requirement_id, completed_count) = deps
        .autowork_orchestrator
        .live_progress(kind, target_id)
        .unwrap_or((None, 0));
    let run_state = AutoWorkState::run_state(enabled, current_requirement_id.as_deref());
    Ok(AutoWorkState {
        kind,
        target_id: target_id.to_owned(),
        enabled,
        tag: live_tag,
        running,
        run_state,
        current_requirement_id,
        completed_count,
    })
}

async fn set(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: SetAutoworkParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({ "error": "missing caller user identity" });
    }
    let kind = match parse_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let target_id = p.target_id;
    if p.enabled && p.tag.is_none() {
        return json!({ "error": "tag is required when enabling autowork (the tag groups the requirements this session will work through)" });
    }

    // Ownership + (terminal) eligibility — same gates as the REST route.
    let owner_check = match kind {
        AutoWorkTargetKind::Conversation => match parse_conv_id(&target_id) {
            Ok(conv_id) => deps.requirement_service.verify_conversation_owner(conv_id, &ctx.user_id).await,
            Err(e) => Err(e),
        },
        AutoWorkTargetKind::Terminal => deps.requirement_service.verify_terminal_owner(&target_id, &ctx.user_id).await,
    };
    if let Err(e) = owner_check {
        return json!({ "error": e.to_string() });
    }
    if p.enabled
        && kind == AutoWorkTargetKind::Terminal
        && let Err(e) = deps.requirement_service.ensure_terminal_autowork_eligible(&target_id).await
    {
        return json!({ "error": e.to_string() });
    }

    if let Err(e) = deps
        .requirement_service
        .save_autowork_config(kind, &target_id, p.enabled, p.tag.as_deref(), p.max_requirements)
        .await
    {
        return json!({ "error": e.to_string() });
    }

    if p.enabled {
        if let Some(tag) = p.tag.clone() {
            deps.autowork_orchestrator
                .start(kind, target_id.clone(), tag, p.max_requirements);
        }
    } else {
        deps.autowork_orchestrator.stop(kind, &target_id);
    }

    match build_state(&deps, kind, &target_id).await {
        Ok(state) => {
            deps.requirement_service.emit_autowork_state(&state);
            ok(state)
        }
        Err(e) => e,
    }
}

async fn get(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: GetAutoworkParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({ "error": "missing caller user identity" });
    }
    let kind = match parse_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let target_id = p.target_id;
    let owner_check = match kind {
        AutoWorkTargetKind::Conversation => match parse_conv_id(&target_id) {
            Ok(conv_id) => deps.requirement_service.verify_conversation_owner(conv_id, &ctx.user_id).await,
            Err(e) => Err(e),
        },
        AutoWorkTargetKind::Terminal => deps.requirement_service.verify_terminal_owner(&target_id, &ctx.user_id).await,
    };
    if let Err(e) = owner_check {
        return json!({ "error": e.to_string() });
    }
    match build_state(&deps, kind, &target_id).await {
        Ok(state) => ok(state),
        Err(e) => e,
    }
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<SetAutoworkParams, _, _>(
        CapabilityMeta::new(
            "nomi_set_autowork",
            "autowork",
            "Enable/disable AutoWork (autonomous requirement execution) on a conversation or terminal and bind a requirement tag.",
            DangerTier::Write,
        ),
        set,
    ));
    out.push(Capability::new::<GetAutoworkParams, _, _>(
        CapabilityMeta::new(
            "nomi_get_autowork",
            "autowork",
            "Read the current AutoWork binding + live run state for a conversation or terminal.",
            DangerTier::Read,
        ),
        get,
    ));
}

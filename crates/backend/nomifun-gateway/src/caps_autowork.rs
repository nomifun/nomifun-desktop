//! AutoWork-domain capabilities (registry form): enable/disable + inspect the
//! AutoWork binding for a conversation or terminal target.
//!
//! Mirrors `POST /api/requirements/autowork`: persist the config via
//! `RequirementService`, then start/stop the live AutoWork runner and
//! broadcast the state — a config write alone would only take effect after the
//! next desktop boot.

use std::future::Future;
use std::sync::Arc;

use nomifun_api_types::{AutoWorkState, AutoWorkTargetKind};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, CompatibilityCapabilityHost};
use crate::id_schema::{CanonicalEntityId, SessionTargetKind};
use crate::registry::{Capability, CapabilityMeta, EffectClass};
use crate::server::ok;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetAutoworkParams {
    /// Target kind: "conversation" or "terminal".
    kind: SessionTargetKind,
    /// The conversation id or terminal id to bind.
    target_id: CanonicalEntityId,
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
#[serde(deny_unknown_fields)]
struct GetAutoworkParams {
    /// Target kind: "conversation" or "terminal".
    kind: SessionTargetKind,
    /// The conversation id or terminal id to inspect.
    target_id: CanonicalEntityId,
}

fn parse_target_id(kind: AutoWorkTargetKind, raw: String) -> Result<String, Value> {
    match kind {
        AutoWorkTargetKind::Conversation => nomifun_common::ConversationId::parse(raw)
            .map(nomifun_common::ConversationId::into_string)
            .map_err(|error| json!({ "error": format!("invalid conversation target_id: {error}") })),
        AutoWorkTargetKind::Terminal => nomifun_common::TerminalId::parse(raw)
            .map(nomifun_common::TerminalId::into_string)
            .map_err(|error| json!({ "error": format!("invalid terminal target_id: {error}") })),
    }
}

#[derive(Clone)]
struct AutoWorkCapabilityDeps {
    requirements: Arc<nomifun_requirement::RequirementService>,
    runner: Arc<nomifun_requirement::AutoWorkRunner>,
}

fn adapt<P, F, Fut>(
    handler: F,
) -> impl Fn(Arc<CompatibilityCapabilityHost>, CallerCtx, P) -> Fut + Send + Sync + 'static
where
    P: Send + 'static,
    F: Fn(Arc<AutoWorkCapabilityDeps>, CallerCtx, P) -> Fut
        + Send
        + Sync
        + Clone
        + 'static,
    Fut: Future<Output = Value> + Send + 'static,
{
    move |deps, ctx, params| {
        handler(
            Arc::new(AutoWorkCapabilityDeps {
                requirements: deps.requirement_service.clone(),
                runner: deps.auto_work_runner.clone(),
            }),
            ctx,
            params,
        )
    }
}

/// Assemble the persisted config + the AutoWork runner's live view into one
/// `AutoWorkState` (the same shape the REST routes return and broadcast).
async fn build_state(
    deps: &AutoWorkCapabilityDeps,
    kind: AutoWorkTargetKind,
    target_id: &str,
) -> Result<AutoWorkState, Value> {
    let (enabled, tag, _max) = deps
        .requirements
        .read_autowork_config(kind, target_id)
        .await
        .map_err(|e| json!({ "error": e.to_string() }))?;
    let running = deps.runner.is_running(kind, target_id);
    let live_tag = deps.runner.running_tag(kind, target_id).or(tag);
    let (current_requirement_id, completed_count) = deps
        .runner
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

async fn set(
    deps: Arc<AutoWorkCapabilityDeps>,
    ctx: CallerCtx,
    p: SetAutoworkParams,
) -> Value {
    let kind = AutoWorkTargetKind::from(p.kind);
    let target_id = match parse_target_id(kind, p.target_id.into_string()) {
        Ok(target_id) => target_id,
        Err(error) => return error,
    };
    if p.enabled && p.tag.is_none() {
        return json!({ "error": "tag is required when enabling autowork (the tag groups the requirements this session will work through)" });
    }

    // Ownership + (terminal) eligibility — same gates as the REST route.
    let owner_check = match kind {
        AutoWorkTargetKind::Conversation => deps
            .requirements
            .verify_conversation_owner(&target_id, ctx.user_id.as_str())
            .await,
        AutoWorkTargetKind::Terminal => deps
            .requirements
            .verify_terminal_owner(&target_id, ctx.user_id.as_str())
            .await,
    };
    if let Err(e) = owner_check {
        return json!({ "error": e.to_string() });
    }
    if p.enabled
        && kind == AutoWorkTargetKind::Terminal
        && let Err(e) = deps
            .requirements
            .ensure_terminal_autowork_eligible(&target_id)
            .await
    {
        return json!({ "error": e.to_string() });
    }

    if let Err(e) = deps
        .requirements
        .save_autowork_config(kind, &target_id, p.enabled, p.tag.as_deref(), p.max_requirements)
        .await
    {
        return json!({ "error": e.to_string() });
    }

    if p.enabled {
        if let Some(tag) = p.tag.clone() {
            deps.runner
                .start(kind, target_id.clone(), tag, p.max_requirements)
                .await;
        }
    } else {
        deps.runner.stop(kind, &target_id).await;
    }

    match build_state(&deps, kind, &target_id).await {
        Ok(state) => {
            deps.requirements.emit_autowork_state(&state);
            ok(state)
        }
        Err(e) => e,
    }
}

async fn get(
    deps: Arc<AutoWorkCapabilityDeps>,
    ctx: CallerCtx,
    p: GetAutoworkParams,
) -> Value {
    let kind = AutoWorkTargetKind::from(p.kind);
    let target_id = match parse_target_id(kind, p.target_id.into_string()) {
        Ok(target_id) => target_id,
        Err(error) => return error,
    };
    let owner_check = match kind {
        AutoWorkTargetKind::Conversation => deps
            .requirements
            .verify_conversation_owner(&target_id, ctx.user_id.as_str())
            .await,
        AutoWorkTargetKind::Terminal => deps
            .requirements
            .verify_terminal_owner(&target_id, ctx.user_id.as_str())
            .await,
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
            EffectClass::Write,
        ),
        adapt(set),
    ));
    out.push(Capability::new::<GetAutoworkParams, _, _>(
        CapabilityMeta::new(
            "nomi_get_autowork",
            "autowork",
            "Read the current AutoWork binding + live run state for a conversation or terminal.",
            EffectClass::Read,
        ),
        adapt(get),
    ));
}

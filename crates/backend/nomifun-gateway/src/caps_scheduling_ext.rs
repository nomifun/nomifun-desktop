//! Extended SCHEDULING / AUTONOMY gateway tools — surface service methods that
//! the base `caps_cron`, `caps_requirement`, `caps_autowork`, and `caps_idmm`
//! files do NOT already register.
//!
//! Each handler follows the established pattern: typed `*Params` (single source
//! of schema + deserialization), `(Arc<GatewayDeps>, CallerCtx, P) -> Value` or
//! `(Arc<GatewayDeps>, P) -> Value`, `crate::server::ok` for success, structured
//! `json!({"error":…})` on failure.

use std::sync::Arc;

use nomifun_cron::types::cron_job_to_response;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::caps_idmm::{parse_kind as parse_idmm_kind, verify_target as verify_idmm_target};
use crate::registry::{Capability, CapabilityMeta, DangerTier, Surface};
use crate::server::ok;

// ═══════════════════════════════════════════════════════════════════════════════
// CRON DOMAIN (extensions)
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct CronGetJobParams {
    /// The id of the cron job to retrieve (from nomi_cron_list).
    job_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct CronRunNowParams {
    /// The id of the cron job to trigger immediately.
    job_id: String,
}

async fn cron_get_job(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CronGetJobParams) -> Value {
    match deps.cron_service.get_job(&ctx.user_id, &p.job_id).await {
        Ok(job) => ok(cron_job_to_response(&job)),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn cron_run_now(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CronRunNowParams) -> Value {
    match deps.cron_service.run_now(&ctx.user_id, &p.job_id).await {
        Ok(resp) => ok(json!({
            "triggered": true,
            "conversation_id": resp.conversation_id,
        })),
        Err(e) => json!({"error": e.to_string()}),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// REQUIREMENT DOMAIN (extensions)
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct RequirementGetParams {
    /// The id of the requirement to fetch.
    id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct RequirementListTagsParams {
    // Intentionally empty — tags() takes no arguments.
    // A unit struct would also work, but an empty object is friendlier to
    // schema consumers.
}

#[derive(Deserialize, JsonSchema)]
struct RequirementGetBoardParams {
    /// The tag whose kanban board to retrieve.
    tag: String,
}

#[derive(Deserialize, JsonSchema)]
struct RequirementResumeTagParams {
    /// The tag to resume (un-pause).
    tag: String,
    /// Re-queue ALL failed requirements in the tag back to pending (default false).
    #[serde(default)]
    requeue_failed: bool,
    /// Re-queue these specific failed requirement ids back to pending.
    #[serde(default)]
    requeue_ids: Vec<i64>,
}

async fn requirement_get(deps: Arc<GatewayDeps>, p: RequirementGetParams) -> Value {
    match deps.requirement_service.get(p.id).await {
        Ok(req) => ok(req),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn requirement_list_tags(deps: Arc<GatewayDeps>, _p: RequirementListTagsParams) -> Value {
    match deps.requirement_service.tags().await {
        Ok(tags) => ok(tags),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn requirement_get_board(deps: Arc<GatewayDeps>, p: RequirementGetBoardParams) -> Value {
    match deps.requirement_service.board(&p.tag).await {
        Ok(board) => ok(board),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn requirement_resume_tag(deps: Arc<GatewayDeps>, p: RequirementResumeTagParams) -> Value {
    // Mirror the REST route: if `requeue_failed`, collect all failed ids from
    // the board and merge with explicit ids.
    let mut requeue_ids = p.requeue_ids;
    if p.requeue_failed {
        match deps.requirement_service.board(&p.tag).await {
            Ok(board) => {
                requeue_ids.extend(board.failed.into_iter().map(|r| r.id));
            }
            Err(e) => return json!({"error": e.to_string()}),
        }
    }
    if let Err(e) = deps.requirement_service.resume_tag(&p.tag, &requeue_ids).await {
        return json!({"error": e.to_string()});
    }
    // Return the updated tag summary (same as the REST route).
    match deps.requirement_service.tags().await {
        Ok(tags) => {
            let summary = tags.into_iter().find(|t| t.tag == p.tag);
            ok(json!({
                "resumed": true,
                "requeued_count": requeue_ids.len(),
                "tag_summary": summary,
            }))
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// IDMM DOMAIN (extensions)
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Deserialize, JsonSchema)]
struct IdmmGetLogParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id to inspect.
    target_id: String,
    /// Maximum rows to return (default 50, clamped to 1..=500).
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct IdmmGetActivityParams {
    /// Maximum rows to return (default 50, clamped to 1..=500).
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct IdmmInterveneParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id to intervene on.
    target_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct IdmmGetSettingsParams {
    // No parameters — global settings.
}

#[derive(Deserialize, JsonSchema)]
struct IdmmSetSettingsParams {
    /// Backup provider id for sidecar model fallback (omit to clear).
    #[serde(default)]
    backup_provider_id: Option<String>,
    /// Backup model id for sidecar fallback (omit to clear).
    #[serde(default)]
    backup_model: Option<String>,
    /// Default steering prompt injected into new IDMM supervision configs.
    #[serde(default)]
    default_steering_prompt: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct IdmmClearLogParams {
    /// Target kind: "conversation" or "terminal".
    kind: String,
    /// The conversation id or terminal id whose log to clear.
    target_id: String,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

// ─── Handlers ───────────────────────────────────────────────────────────────

async fn idmm_get_log(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: IdmmGetLogParams) -> Value {
    let kind = match parse_idmm_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    if let Some(err) = verify_idmm_target(&deps, &ctx, kind, &p.target_id).await {
        return err;
    }
    let limit = p.limit.unwrap_or(50).clamp(1, 500);
    match deps.idmm_service.log(&ctx.user_id, kind, &p.target_id, limit).await {
        Ok(records) => ok(records),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn idmm_get_activity(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: IdmmGetActivityParams) -> Value {
    if ctx.user_id.trim().is_empty() {
        return json!({"error": "missing caller user identity"});
    }
    let limit = p.limit.unwrap_or(50).clamp(1, 500);
    match deps.idmm_service.recent_activity(&ctx.user_id, limit).await {
        Ok(records) => ok(records),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn idmm_intervene(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: IdmmInterveneParams) -> Value {
    let kind = match parse_idmm_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    if let Some(err) = verify_idmm_target(&deps, &ctx, kind, &p.target_id).await {
        return err;
    }
    match deps
        .idmm_service
        .intervene_now(&ctx.user_id, kind, &p.target_id)
        .await
    {
        Ok(()) => {
            // Return the updated state (same as the REST route).
            match deps
                .idmm_service
                .build_state(&ctx.user_id, kind, &p.target_id)
                .await
            {
                Ok(state) => ok(json!({
                    "intervened": true,
                    "state": state,
                })),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn idmm_get_settings(deps: Arc<GatewayDeps>, _p: IdmmGetSettingsParams) -> Value {
    match deps.idmm_service.get_settings().await {
        Ok(settings) => ok(settings),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn idmm_set_settings(deps: Arc<GatewayDeps>, p: IdmmSetSettingsParams) -> Value {
    // Read current settings and overlay provided fields (same partial-update
    // semantics as the REST route).
    let mut settings = match deps.idmm_service.get_settings().await {
        Ok(s) => s,
        Err(e) => return json!({"error": e.to_string()}),
    };
    if p.backup_provider_id.is_some() {
        settings.backup_provider_id = p.backup_provider_id;
    }
    if p.backup_model.is_some() {
        settings.backup_model = p.backup_model;
    }
    if let Some(prompt) = p.default_steering_prompt {
        settings.default_steering_prompt = prompt;
    }

    match deps.idmm_service.set_settings(&settings).await {
        Ok(()) => ok(settings),
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn idmm_clear_log(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: IdmmClearLogParams) -> Value {
    let kind = match parse_idmm_kind(&p.kind) {
        Ok(k) => k,
        Err(e) => return e,
    };
    if let Some(err) = verify_idmm_target(&deps, &ctx, kind, &p.target_id).await {
        return err;
    }
    match deps.idmm_service.clear_log(&ctx.user_id, kind, &p.target_id).await {
        Ok(count) => json!({"result": format!("cleared {count} intervention records")}),
        Err(e) => json!({"error": e.to_string()}),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// REGISTRATION
// ═══════════════════════════════════════════════════════════════════════════════

/// Register the scheduling/autonomy extension capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    // ── Cron extensions ─────────────────────────────────────────────────────
    out.push(Capability::new::<CronGetJobParams, _, _>(
        CapabilityMeta::new(
            "nomi_cron_get_job",
            "cron",
            "Get a single cron job by id (full detail including schedule, next/last run, error).",
            DangerTier::Read,
        ),
        cron_get_job,
    ));
    out.push(Capability::new::<CronRunNowParams, _, _>(
        CapabilityMeta::new(
            "nomi_cron_run_now",
            "cron",
            "Trigger a cron job to execute immediately (out-of-schedule one-shot run).",
            DangerTier::Write,
        ),
        cron_run_now,
    ));

    // ── Requirement extensions ──────────────────────────────────────────────
    out.push(Capability::new::<RequirementGetParams, _, _>(
        CapabilityMeta::new(
            "nomi_requirement_get",
            "requirement",
            "Fetch a single requirement by id (full detail including attachments, timestamps, status).",
            DangerTier::Read,
        )
        .instance_owner(),
        |deps, _ctx, p| requirement_get(deps, p),
    ));
    out.push(Capability::new::<RequirementListTagsParams, _, _>(
        CapabilityMeta::new(
            "nomi_requirement_list_tags",
            "requirement",
            "List all AutoWork tags with per-status counts, paused state, and totals.",
            DangerTier::Read,
        )
        .instance_owner(),
        |deps, _ctx, p| requirement_list_tags(deps, p),
    ));
    out.push(Capability::new::<RequirementGetBoardParams, _, _>(
        CapabilityMeta::new(
            "nomi_requirement_get_board",
            "requirement",
            "Get the kanban board view for a tag (requirements grouped by status column).",
            DangerTier::Read,
        )
        .instance_owner(),
        |deps, _ctx, p| requirement_get_board(deps, p),
    ));
    out.push(Capability::new::<RequirementResumeTagParams, _, _>(
        CapabilityMeta::new(
            "nomi_requirement_resume_tag",
            "requirement",
            "Resume a paused AutoWork tag and optionally re-queue failed requirements back to pending.",
            DangerTier::Write,
        )
        .instance_owner(),
        |deps, _ctx, p| requirement_resume_tag(deps, p),
    ));

    // ── IDMM extensions ─────────────────────────────────────────────────────
    out.push(Capability::new::<IdmmGetLogParams, _, _>(
        CapabilityMeta::new(
            "nomi_idmm_get_log",
            "idmm",
            "Read the persisted intervention log for a conversation or terminal (most-recent-first).",
            DangerTier::Read,
        ),
        idmm_get_log,
    ));
    out.push(Capability::new::<IdmmGetActivityParams, _, _>(
        CapabilityMeta::new(
            "nomi_idmm_get_activity",
            "idmm",
            "Read the caller's cross-session intervention feed (their targets only, most-recent-first).",
            DangerTier::Read,
        ),
        idmm_get_activity,
    ));
    out.push(Capability::new::<IdmmInterveneParams, _, _>(
        CapabilityMeta::new(
            "nomi_idmm_intervene",
            "idmm",
            "Force one IDMM supervision pass now (manual 'act now') and return the resulting state.",
            DangerTier::Write,
        ),
        idmm_intervene,
    ));
    out.push(Capability::new::<IdmmGetSettingsParams, _, _>(
        CapabilityMeta::new(
            "nomi_idmm_get_settings",
            "idmm",
            "Read global IDMM settings (backup provider/model, default steering prompt).",
            DangerTier::Read,
        )
        .instance_owner(),
        |deps, _ctx, p| idmm_get_settings(deps, p),
    ));
    out.push(Capability::new::<IdmmSetSettingsParams, _, _>(
        CapabilityMeta::new(
            "nomi_idmm_set_settings",
            "idmm",
            "Update global IDMM settings (backup provider/model, default steering prompt). Partial update: omitted fields keep their current value.",
            DangerTier::Sensitive,
        )
        .instance_owner(),
        |deps, _ctx, p| idmm_set_settings(deps, p),
    ));
    out.push(Capability::new::<IdmmClearLogParams, _, _>(
        CapabilityMeta::new(
            "nomi_idmm_clear_log",
            "idmm",
            "Clear all persisted intervention records for a conversation or terminal. Irreversible.",
            DangerTier::Destructive,
        )
        .deny_on(&[Surface::Channel]),
        idmm_clear_log,
    ));
}

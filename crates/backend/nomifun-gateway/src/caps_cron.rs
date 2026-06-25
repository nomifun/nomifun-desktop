//! Cron-domain capabilities (registry form). Create/update reuse the
//! `ICronService` implementation behind the `[CRON_*]` text protocol, so a
//! gateway session gets the same context derivation (agent type / model from
//! the bound conversation) and bind-back behavior as the in-chat protocol.

use std::sync::Arc;

use nomifun_api_types::{ListCronJobsQuery, UpdateConversationRequest};
use nomifun_common::AgentType;
use nomifun_conversation::response_middleware::{
    CronCreateParams as SvcCronCreate, CronUpdateParams as SvcCronUpdate, ICronService,
};
use nomifun_cron::types::cron_job_to_response;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::{CallerCtx, GatewayDeps};
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::ok;
use crate::tools_provider;

#[derive(Deserialize, JsonSchema)]
struct CronListParams {
    /// Restrict to jobs bound to one conversation (default: all jobs).
    #[serde(default)]
    conversation_id: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct CronCreateParams {
    /// Short human-readable job name.
    name: String,
    /// Standard 5-field cron expression, e.g. "0 9 * * *" for daily 09:00.
    cron: String,
    /// Human-readable description of the schedule (e.g. "every day at 9am").
    #[serde(default)]
    description: Option<String>,
    /// The prompt message sent to the agent on every trigger.
    message: String,
    /// Conversation to run the job in (default: the calling conversation).
    #[serde(default)]
    conversation_id: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct CronUpdateParams {
    /// The id of the cron job to update (from nomi_cron_list).
    job_id: String,
    /// New job name (full replacement; pass the existing value to keep it).
    name: String,
    /// New cron expression (full replacement).
    cron: String,
    /// New human-readable schedule description.
    #[serde(default)]
    description: Option<String>,
    /// New trigger message (full replacement).
    message: String,
    /// Conversation the job is bound to (default: the calling conversation).
    #[serde(default)]
    conversation_id: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct CronDeleteParams {
    /// The id of the cron job to delete. Confirm the target with the user first.
    job_id: String,
}

/// Duplicate-create guard: an ACTIVE job in the same conversation with the same
/// (trimmed) name or the exact same (trimmed) message counts as a duplicate.
fn is_duplicate_job(existing_name: &str, existing_message: &str, new_name: &str, new_message: &str) -> bool {
    existing_name.trim().eq_ignore_ascii_case(new_name.trim()) || existing_message.trim() == new_message.trim()
}

async fn list(deps: Arc<GatewayDeps>, p: CronListParams) -> Value {
    let query = ListCronJobsQuery {
        conversation_id: p.conversation_id,
    };
    match deps.cron_service.list_jobs(&query).await {
        Ok(jobs) => ok(jobs.iter().map(cron_job_to_response).collect::<Vec<_>>()),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn create(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CronCreateParams) -> Value {
    if ctx.user_id.is_empty() {
        return json!({ "error": "missing caller user identity" });
    }
    let target_conv_id = match p.conversation_id.or_else(|| ctx.conversation_id.parse::<i64>().ok()) {
        Some(id) => id,
        None => {
            return json!({ "error": "missing required field: conversation_id (no calling conversation to bind to)" });
        }
    };
    let target_conversation = target_conv_id.to_string();

    // ── duplicate guard ──────────────────────────────────────────────
    match deps
        .cron_service
        .list_jobs(&ListCronJobsQuery {
            conversation_id: Some(target_conv_id),
        })
        .await
    {
        Ok(jobs) => {
            if let Some(existing) = jobs
                .iter()
                .find(|j| j.enabled && is_duplicate_job(&j.name, &j.message, &p.name, &p.message))
            {
                return ok(json!({
                    "duplicate": true,
                    "existing_job": cron_job_to_response(existing),
                    "note": "an ACTIVE cron job with the same name or message already exists in this conversation — nothing was created. Use nomi_cron_update to modify it; only create a second job if the owner explicitly asked for a duplicate this turn."
                }));
            }
        }
        Err(e) => return json!({ "error": e.to_string() }),
    }

    // ── model guard (nomi conversations only) ────────────────────────
    let mut model_note: Option<String> = None;
    match deps.conversation_service.get(&ctx.user_id, &target_conversation).await {
        Ok(conv) => {
            let model_missing = conv.model.as_ref().is_none_or(|m| m.provider_id.trim().is_empty());
            if conv.r#type == AgentType::Nomi && model_missing {
                match tools_provider::resolve_nomi_model(&deps, &ctx, None, None).await {
                    Ok((m, source)) => {
                        let req = UpdateConversationRequest {
                            name: None,
                            pinned: None,
                            model: Some(m.clone()),
                            extra: None,
                        };
                        if let Err(e) = deps
                            .conversation_service
                            .update(&ctx.user_id, &target_conversation, req, &deps.task_manager)
                            .await
                        {
                            return json!({ "error": format!("failed to persist auto-selected model onto the bound conversation: {e}") });
                        }
                        model_note = Some(format!(
                            "the bound conversation had no model configured; auto-selected {}/{} (source: {source}) and saved it onto the conversation — mention this to the owner",
                            m.provider_id, m.model
                        ));
                    }
                    Err(e) => return e,
                }
            }
        }
        Err(e) => {
            return json!({
                "error": format!("cannot create the cron job: the bound conversation '{target_conversation}' is not accessible ({e}); a job bound to a missing conversation would never run")
            });
        }
    }

    let params = SvcCronCreate {
        name: p.name,
        schedule: p.cron,
        schedule_description: p.description.unwrap_or_default(),
        message: p.message,
    };
    let result = ICronService::create_job(deps.cron_service.as_ref(), &ctx.user_id, &target_conversation, &params).await;
    if result.success {
        ok(json!({ "message": result.message, "model_note": model_note }))
    } else {
        json!({ "error": result.message })
    }
}

async fn update(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CronUpdateParams) -> Value {
    let target_conversation = p
        .conversation_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| ctx.conversation_id.clone());
    let params = SvcCronUpdate {
        job_id: p.job_id,
        name: p.name,
        schedule: p.cron,
        schedule_description: p.description.unwrap_or_default(),
        message: p.message,
    };
    command_result(ICronService::update_job(deps.cron_service.as_ref(), &ctx.user_id, &target_conversation, &params).await)
}

async fn delete(deps: Arc<GatewayDeps>, ctx: CallerCtx, p: CronDeleteParams) -> Value {
    command_result(ICronService::delete_job(deps.cron_service.as_ref(), &ctx.user_id, &p.job_id).await)
}

fn command_result(result: nomifun_conversation::response_middleware::CronCommandResult) -> Value {
    if result.success {
        json!({ "result": result.message })
    } else {
        json!({ "error": result.message })
    }
}

pub(crate) fn register(out: &mut Vec<Capability>) {
    out.push(Capability::new::<CronListParams, _, _>(
        CapabilityMeta::new(
            "nomi_cron_list",
            "cron",
            "List scheduled cron jobs (all jobs by default; pass conversation_id to filter to one session).",
            DangerTier::Read,
        ),
        |deps, _ctx, p| list(deps, p),
    ));
    out.push(Capability::new::<CronCreateParams, _, _>(
        CapabilityMeta::new(
            "nomi_cron_create",
            "cron",
            "Schedule a recurring prompt (cron). Binds to conversation_id or the calling conversation; guards against duplicates and model-less nomi sessions.",
            DangerTier::Write,
        ),
        create,
    ));
    out.push(Capability::new::<CronUpdateParams, _, _>(
        CapabilityMeta::new(
            "nomi_cron_update",
            "cron",
            "Update a cron job (full replacement of name/cron/message).",
            DangerTier::Write,
        ),
        update,
    ));
    out.push(Capability::new::<CronDeleteParams, _, _>(
        CapabilityMeta::new(
            "nomi_cron_delete",
            "cron",
            "Delete a cron job. Confirm the target with the user first.",
            DangerTier::Destructive,
        ),
        delete,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_when_name_matches_ignoring_case_and_whitespace() {
        assert!(is_duplicate_job("Daily Report", "msg a", "  daily report ", "msg b"));
    }

    #[test]
    fn duplicate_when_message_matches_exactly_after_trim() {
        assert!(is_duplicate_job("job a", " summarize inbox ", "job b", "summarize inbox"));
    }

    #[test]
    fn not_duplicate_when_both_differ() {
        assert!(!is_duplicate_job("job a", "message a", "job b", "message b"));
    }

    #[test]
    fn message_comparison_is_case_sensitive() {
        assert!(!is_duplicate_job("job a", "Do The Thing", "job b", "do the thing"));
    }
}

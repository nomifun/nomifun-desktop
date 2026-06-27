//! 智能编排 (orchestration) domain capabilities (registry form): create an
//! orchestration run from a goal + fleet, inspect its task DAG status, and read
//! the aggregated result once the run completes.
//!
//! Backed by:
//! - `nomifun_orchestrator::RunService` — the run control-plane
//!   (`create` snapshots the fleet + parks in `planning`; `plan` decomposes the
//!   goal into a task DAG + assignments + flips to `running`; `get_detail` reads
//!   the run + tasks + deps + assignments).
//! - `nomifun_orchestrator::RunEngine` — the serial execution loop; `start`
//!   spawns (or restarts) the loop that drives ready tasks to completion.
//!
//! `nomi_run_create` performs the full create → plan → start choreography so a
//! single tool call kicks off a run end-to-end. As of the conversation-native
//! redesign (P1) it takes ONLY `{goal, autonomy?}` and pulls everything else —
//! `work_dir`, `model_range`, `lead_conv_id` — from the CALLING conversation's
//! `extra` (the "orchestration lead" context), then drives the workspace-less
//! [`create_adhoc`](nomifun_orchestrator::RunService::create_adhoc) path. The two
//! read tools project the rich `RunDetail` down to a compact, LLM-friendly shape
//! (run status + per-task title/status, and on result the per-task
//! `output_summary`).
//!
//! ## `ModelRange::Auto` expansion (Task 3 decision)
//! `RunService::create_adhoc` rejects an unexpanded `Auto` — it has no provider
//! access (its struct holds only run/fleet/ws repos + a planner + an emitter). The
//! gateway DOES (`GatewayDeps::provider_repo`, surfaced via
//! [`load_provider_summaries`](crate::tools_provider::load_provider_summaries),
//! already filtered to enabled providers × enabled models). So we expand `Auto`
//! → a concrete `Range` of every enabled `(provider, model)` pair HERE, in the
//! caps layer, before calling `create_adhoc`. `Single`/`Range` pass through
//! verbatim.

use std::sync::Arc;

use nomifun_api_types::{CreateAdhocRunRequest, ModelRange, ModelRef, RunDetail, UpdateConversationRequest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::GatewayDeps;
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::{ok, require_user};
use crate::tools_provider::{ProviderSummary, load_provider_summaries};

// ── param structs (single source: schema + runtime) ──────────────────────

/// Create and kick off an orchestration run from the calling conversation's
/// context. The conversation must be an orchestration "lead" (its `extra` carries
/// a `model_range`); `work_dir` / `lead_conv_id` / `model_range` are read from
/// there, so the tool only needs the goal (and, optionally, an autonomy override).
#[derive(Deserialize, JsonSchema)]
struct RunCreateParams {
    /// The high-level goal to decompose into tasks and execute.
    goal: String,
    /// Autonomy mode: "supervised" (default) or "autonomous". Controls how much
    /// the run pauses for confirmation. Omit for the default.
    #[serde(default)]
    autonomy: Option<String>,
}

/// Inspect a run's current status and the status of each of its tasks.
#[derive(Deserialize, JsonSchema)]
struct RunStatusParams {
    /// The run id (from nomi_run_create).
    run_id: String,
}

/// Read a run's aggregated result: the run summary and each task's output
/// summary. While the run is still executing, `status` reflects that.
#[derive(Deserialize, JsonSchema)]
struct RunResultParams {
    /// The run id (from nomi_run_create).
    run_id: String,
}

// ── handlers ──────────────────────────────────────────────────────────────

async fn create(deps: Arc<GatewayDeps>, ctx: crate::deps::CallerCtx, p: RunCreateParams) -> Value {
    let user = match require_user(&ctx) {
        Ok(u) => u.to_owned(),
        Err(e) => return e,
    };
    if ctx.conversation_id.is_empty() {
        return json!({ "error": "missing caller conversation identity (NOMI_GW_MCP_CONVERSATION_ID)" });
    }

    // 1. Read the calling ("lead") conversation's context.
    let conv = match deps.conversation_service.get(&user, &ctx.conversation_id).await {
        Ok(c) => c,
        Err(e) => return json!({ "error": e.to_string() }),
    };
    let (work_dir, model_range) = match parse_lead_extra(&conv.extra) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    // 2. Expand `Auto` to a concrete `Range` using the gateway's provider access
    //    (RunService::create_adhoc rejects an unexpanded Auto). Single/Range pass
    //    through unchanged — load_provider_summaries is only hit for Auto.
    let model_range = if matches!(model_range, ModelRange::Auto) {
        let summaries = match load_provider_summaries(&deps).await {
            Ok(s) => s,
            Err(e) => return e,
        };
        match expand_auto_range(&summaries) {
            Ok(r) => r,
            Err(e) => return e,
        }
    } else {
        model_range
    };

    let lead_conv_id = ctx.conversation_id.parse::<i64>().ok();
    let req = CreateAdhocRunRequest {
        goal: p.goal,
        work_dir,
        model_range,
        pinned_roles: vec![],
        autonomy: p.autonomy,
        // Serial loop (P1): parallelism is not yet a gateway-exposed knob.
        max_parallel: None,
        lead_conv_id,
    };

    // 3. Create: synthesize the fleet from the model range + park in `planning`.
    let run = match deps.orchestrator_run_service.create_adhoc(&user, req).await {
        Ok(run) => run,
        Err(e) => return json!({ "error": e.to_string() }),
    };
    // 4. Plan: decompose the goal → task DAG + assignments, flip to `running`.
    if let Err(e) = deps.orchestrator_run_service.plan(&run.id).await {
        return json!({ "error": format!("run {} created but planning failed: {e}", run.id) });
    }
    // 5. Start the serial execution loop (idempotent; restarts any existing loop).
    deps.orchestrator_run_engine.start(run.id.clone());

    // 6. Write the run id back into the lead conversation's `extra` so the
    //    frontend DAG can locate this run later (P2). `ConversationService::update`
    //    MERGES `extra` (top-level keys overwritten, others preserved), so this
    //    does not clobber `workspace` / `model_range` / etc. Best-effort: a
    //    write-back failure is logged but does not fail the (already-started) run.
    let update = UpdateConversationRequest {
        name: None,
        pinned: None,
        model: None,
        extra: Some(json!({ "orchestrator_run_id": run.id })),
    };
    if let Err(e) = deps
        .conversation_service
        .update(&user, &ctx.conversation_id, update, &deps.task_manager)
        .await
    {
        tracing::warn!(
            run_id = %run.id,
            lead_conv_id = %ctx.conversation_id,
            error = %e,
            "failed to write orchestrator_run_id back to lead conversation extra"
        );
    }

    // Re-read so the returned status reflects the post-plan state (`running`).
    let status = match deps.orchestrator_run_service.get_detail(&run.id).await {
        Ok(detail) => detail.run.status,
        // The run exists (we just created it); fall back to the create-time status.
        Err(_) => run.status,
    };
    ok(json!({ "run_id": run.id, "status": status }))
}

// ── lead-conversation context parsing + Auto expansion ────────────────────

/// Read the run's `work_dir` + `model_range` out of a lead conversation's `extra`.
///
/// - `work_dir` ← `extra.workspace` (string, optional → `None` when absent/empty).
/// - `model_range` ← `extra.model_range` (the tagged [`ModelRange`] JSON). Absent
///   or unparseable ⇒ a clear error: this conversation is not an orchestration
///   lead (it never picked a model range), so it cannot drive a run.
///
/// `Auto` is returned verbatim here — its expansion to a concrete `Range` needs
/// provider access and happens in [`expand_auto_range`] at the handler.
fn parse_lead_extra(extra: &Value) -> Result<(Option<String>, ModelRange), Value> {
    let work_dir = extra
        .get("workspace")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let model_range: ModelRange = match extra.get("model_range") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
            json!({
                "error": format!("this conversation's model_range is malformed ({e}); it cannot drive an orchestration run")
            })
        })?,
        None => {
            return Err(json!({
                "error": "this conversation is not an orchestration lead: it has no model_range in its context. Start the run from a conversation configured with a model range (single / range / auto)."
            }));
        }
    };
    Ok((work_dir, model_range))
}

/// Expand `ModelRange::Auto` into a concrete `Range` of every ENABLED provider ×
/// its enabled models (the summaries are already `model_enabled`-filtered). An
/// empty result (no provider/model configured) is a clear error rather than an
/// empty run.
fn expand_auto_range(summaries: &[ProviderSummary]) -> Result<ModelRange, Value> {
    let models: Vec<ModelRef> = summaries
        .iter()
        .filter(|p| p.enabled)
        .flat_map(|p| {
            p.models.iter().map(move |m| ModelRef {
                provider_id: p.id.clone(),
                model: m.clone(),
            })
        })
        .collect();
    if models.is_empty() {
        return Err(json!({
            "error": "auto model range selected, but no provider/model is enabled on this desktop. Configure one in Settings → Providers (or pick a concrete model range) before starting a run."
        }));
    }
    Ok(ModelRange::Range { models })
}

async fn status(deps: Arc<GatewayDeps>, p: RunStatusParams) -> Value {
    match deps.orchestrator_run_service.get_detail(&p.run_id).await {
        Ok(detail) => ok(project_status(&detail)),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

async fn result(deps: Arc<GatewayDeps>, p: RunResultParams) -> Value {
    match deps.orchestrator_run_service.get_detail(&p.run_id).await {
        Ok(detail) => ok(project_result(&detail)),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

// ── result projections (RunDetail → compact LLM-friendly shape) ───────────

/// Run status + per-task {id, title, status}.
fn project_status(detail: &RunDetail) -> Value {
    json!({
        "run_id": detail.run.id,
        "status": detail.run.status,
        "tasks": detail
            .tasks
            .iter()
            .map(|t| json!({ "id": t.id, "title": t.title, "status": t.status }))
            .collect::<Vec<_>>(),
    })
}

/// Run status + summary + per-task {title, output_summary}. When the run is not
/// yet terminal, `status` reflects the in-flight state (e.g. "running"); the
/// summary / output fields are simply whatever has been persisted so far.
fn project_result(detail: &RunDetail) -> Value {
    json!({
        "run_id": detail.run.id,
        "status": detail.run.status,
        "summary": detail.run.summary,
        "tasks": detail
            .tasks
            .iter()
            .map(|t| json!({ "title": t.title, "output_summary": t.output_summary }))
            .collect::<Vec<_>>(),
    })
}

// ── registration ─────────────────────────────────────────────────────────

/// Register the orchestration-domain capabilities.
pub(crate) fn register(out: &mut Vec<Capability>) {
    // 1. Create + kick off a run (write).
    out.push(Capability::new::<RunCreateParams, _, _>(
        CapabilityMeta::new(
            "nomi_run_create",
            "orchestrator",
            "Create and start an orchestration run from THIS conversation's context: decompose the goal into a task DAG over the conversation's chosen model range and drive it to completion. Only works in an orchestration-lead conversation (one with a model_range). Returns the run id and status.",
            DangerTier::Write,
        ),
        |deps, ctx, p| create(deps, ctx, p),
    ));

    // 2. Run status (read).
    out.push(Capability::new::<RunStatusParams, _, _>(
        CapabilityMeta::new(
            "nomi_run_status",
            "orchestrator",
            "Get an orchestration run's current status and each task's id, title, and status.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| status(deps, p),
    ));

    // 3. Run result (read).
    out.push(Capability::new::<RunResultParams, _, _>(
        CapabilityMeta::new(
            "nomi_run_result",
            "orchestrator",
            "Read an orchestration run's aggregated result: the run summary and each task's output summary. While still running, status reflects the in-flight state.",
            DangerTier::Read,
        ),
        |deps, _ctx, p| result(deps, p),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Registry, Surface};

    fn summary(id: &str, enabled: bool, models: &[&str]) -> ProviderSummary {
        ProviderSummary {
            id: id.to_owned(),
            name: format!("name-{id}"),
            platform: "openai".to_owned(),
            enabled,
            models: models.iter().map(|m| m.to_string()).collect(),
        }
    }

    // ── parse_lead_extra: reads work_dir + model_range from a lead conv's extra ──

    #[test]
    fn parse_lead_extra_reads_workspace_and_range() {
        let extra = json!({
            "workspace": "/x/proj",
            "model_range": {"mode": "range", "models": [
                {"provider_id": "p1", "model": "m1"},
                {"provider_id": "p2", "model": "m2"}
            ]}
        });
        let (work_dir, range) = parse_lead_extra(&extra).expect("parses");
        assert_eq!(work_dir.as_deref(), Some("/x/proj"));
        match range {
            ModelRange::Range { models } => {
                assert_eq!(models.len(), 2);
                assert_eq!(models[0].provider_id, "p1");
                assert_eq!(models[1].model, "m2");
            }
            other => panic!("expected range, got {other:?}"),
        }
    }

    #[test]
    fn parse_lead_extra_single_range_and_no_workspace() {
        // No `workspace` key → work_dir None; single model range parses.
        let extra = json!({
            "model_range": {"mode": "single", "model": {"provider_id": "ps", "model": "ms"}}
        });
        let (work_dir, range) = parse_lead_extra(&extra).expect("parses");
        assert!(work_dir.is_none(), "absent workspace → None");
        assert!(matches!(range, ModelRange::Single { .. }));
    }

    #[test]
    fn parse_lead_extra_blank_workspace_is_none() {
        let extra = json!({
            "workspace": "   ",
            "model_range": {"mode": "auto"}
        });
        let (work_dir, range) = parse_lead_extra(&extra).expect("parses");
        assert!(work_dir.is_none(), "blank workspace → None");
        assert!(matches!(range, ModelRange::Auto), "auto returned verbatim");
    }

    #[test]
    fn parse_lead_extra_missing_model_range_is_clean_error() {
        // A conversation that never picked a model range is not a lead → clean error.
        let extra = json!({ "workspace": "/x" });
        let err = parse_lead_extra(&extra).expect_err("must error without model_range");
        let msg = err["error"].as_str().unwrap_or("");
        assert!(
            msg.contains("not an orchestration lead"),
            "error must explain the conversation is not a lead, got: {msg}"
        );
    }

    #[test]
    fn parse_lead_extra_malformed_model_range_is_clean_error() {
        // Present but unparseable (bad tag) → a clear "malformed" error, not a panic.
        let extra = json!({ "model_range": {"mode": "nonsense"} });
        let err = parse_lead_extra(&extra).expect_err("must error on malformed range");
        let msg = err["error"].as_str().unwrap_or("");
        assert!(msg.contains("malformed"), "got: {msg}");
    }

    // ── expand_auto_range: Auto → concrete Range of enabled (provider, model) ──

    #[test]
    fn expand_auto_lists_enabled_models() {
        let summaries = vec![
            summary("p1", true, &["a", "b"]),
            summary("off", false, &["x"]), // disabled provider excluded
            summary("p2", true, &["c"]),
        ];
        let range = expand_auto_range(&summaries).expect("expands");
        match range {
            ModelRange::Range { models } => {
                // p1×{a,b} + p2×{c} = 3 pairs; the disabled provider is excluded.
                assert_eq!(models.len(), 3, "two enabled providers' models only");
                let pairs: Vec<(&str, &str)> = models
                    .iter()
                    .map(|m| (m.provider_id.as_str(), m.model.as_str()))
                    .collect();
                assert!(pairs.contains(&("p1", "a")));
                assert!(pairs.contains(&("p1", "b")));
                assert!(pairs.contains(&("p2", "c")));
                assert!(!pairs.iter().any(|(p, _)| *p == "off"), "disabled excluded");
            }
            other => panic!("expected range, got {other:?}"),
        }
    }

    #[test]
    fn expand_auto_empty_is_clean_error() {
        // Only a disabled provider (and an enabled-but-model-less one) → no models.
        let summaries = vec![summary("off", false, &["a"]), summary("empty", true, &[])];
        let err = expand_auto_range(&summaries).expect_err("must error with no enabled models");
        let msg = err["error"].as_str().unwrap_or("");
        assert!(msg.contains("no provider/model is enabled"), "got: {msg}");
    }

    /// The three orchestration tools are registered and visible on the Desktop
    /// surface (all are Read/Write — never hard-denied), with names within the
    /// 42-char style budget.
    #[test]
    fn orchestrator_tools_registered_and_visible_on_desktop() {
        let reg = Registry::global();
        for name in ["nomi_run_create", "nomi_run_status", "nomi_run_result"] {
            assert!(
                reg.contains(name),
                "orchestrator tool {name} is not registered"
            );
            assert!(
                reg.tool_visible(Surface::Desktop, name),
                "orchestrator tool {name} must be visible on the Desktop surface"
            );
            assert!(
                name.len() <= 42,
                "orchestrator tool name {name} exceeds the 42-char budget ({} chars)",
                name.len()
            );
        }
    }
}

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
//! single tool call kicks off a run end-to-end (the same sequence the REST
//! `POST /runs` route runs). The two read tools project the rich `RunDetail`
//! down to a compact, LLM-friendly shape (run status + per-task title/status,
//! and on result the per-task `output_summary`).

use std::sync::Arc;

use nomifun_api_types::{CreateRunRequest, RunDetail};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deps::GatewayDeps;
use crate::registry::{Capability, CapabilityMeta, DangerTier};
use crate::server::{ok, require_user};

// ── param structs (single source: schema + runtime) ──────────────────────

/// Create and kick off an orchestration run: snapshot the fleet, decompose the
/// goal into a task DAG, and start the serial execution loop.
#[derive(Deserialize, JsonSchema)]
struct RunCreateParams {
    /// The orchestration workspace id the run belongs to (from the workspace list).
    workspace_id: String,
    /// The high-level goal to decompose into tasks and execute.
    goal: String,
    /// The fleet (编队) id whose members are snapshotted and assigned to tasks.
    fleet_id: String,
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
        Ok(u) => u,
        Err(e) => return e,
    };
    let req = CreateRunRequest {
        workspace_id: p.workspace_id,
        goal: p.goal,
        fleet_id: p.fleet_id,
        autonomy: p.autonomy,
        // Serial loop (P1): parallelism is not yet a gateway-exposed knob.
        max_parallel: None,
    };
    // 1. Create: snapshot the fleet + park in `planning`.
    let run = match deps.orchestrator_run_service.create(user, req).await {
        Ok(run) => run,
        Err(e) => return json!({ "error": e.to_string() }),
    };
    // 2. Plan: decompose the goal → task DAG + assignments, flip to `running`.
    if let Err(e) = deps.orchestrator_run_service.plan(&run.id).await {
        return json!({ "error": format!("run {} created but planning failed: {e}", run.id) });
    }
    // 3. Start the serial execution loop (idempotent; restarts any existing loop).
    deps.orchestrator_run_engine.start(run.id.clone());
    // Re-read so the returned status reflects the post-plan state (`running`).
    let status = match deps.orchestrator_run_service.get_detail(&run.id).await {
        Ok(detail) => detail.run.status,
        // The run exists (we just created it); fall back to the create-time status.
        Err(_) => run.status,
    };
    ok(json!({ "run_id": run.id, "status": status }))
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
            "Create and start an orchestration run: decompose a goal into a task DAG against a fleet and drive it to completion. Returns the run id and status.",
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
    use crate::registry::{Registry, Surface};

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

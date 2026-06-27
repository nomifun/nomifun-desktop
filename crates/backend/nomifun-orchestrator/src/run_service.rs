//! [`RunService`]: create / plan / inspect / cancel orchestration runs.
//!
//! The service owns the *control-plane* of a run — everything that happens
//! synchronously around the [`RunEngine`](crate::engine::RunEngine) execution
//! loop:
//!
//! - [`RunService::create`] snapshots the chosen fleet into the run row (so the
//!   run is reproducible even if the fleet is later edited/deleted) and parks the
//!   run in `planning`.
//! - [`RunService::plan`] loads the run + its fleet snapshot, asks the
//!   [`PlanProducer`](crate::plan::PlanProducer) to decompose the goal into a
//!   [`PlannedDag`], then persists the tasks (status `pending`), the
//!   `depends_on` edges (planned index → minted task id), and the
//!   `member_index` → member-id assignments (`source = "auto"`). It emits
//!   `run.planUpdated` and flips the run to `running` — at which point the engine
//!   may pick it up.
//! - [`RunService::get_detail`] / [`RunService::list`] are the read paths
//!   (Row↔DTO mapping, JSON-as-TEXT decode of `task_profile` / `output_files`).
//! - [`RunService::cancel`] flips the run to `cancelled` and emits.
//!
//! Row↔DTO mapping note: `output_summary` is PROSE pass-through (the column is a
//! plain `Option<String>`, *not* JSON, despite a misleading Row comment), while
//! `output_files` is a JSON `Vec<String>` (decoded fail-soft to an empty vec).

use std::sync::Arc;

use nomifun_api_types::{
    Assignment, CreateAdhocRunRequest, CreateRunRequest, FleetMember, ModelRange, PlannedDag,
    ReassignRequest, Run, RunDetail, RunTask, RunTaskDep, TaskProfile,
};
use nomifun_common::AppError;
use nomifun_common::generate_prefixed_id;
use nomifun_db::models::{
    FleetMemberRow, OrchAssignmentRow, OrchRunRow, OrchRunTaskDepRow, OrchRunTaskRow,
};
use nomifun_db::{
    CreateAssignmentParams, CreateRunParams, CreateTaskParams, IFleetRepository,
    IOrchWorkspaceRepository, IRunRepository, UpdateRunParams,
};

use crate::error::OrchestratorError;
use crate::events::OrchestratorRunEventEmitter;
use crate::plan::PlanProducer;
use crate::router::rank_members;

/// Default autonomy when the create request omits it.
const DEFAULT_AUTONOMY: &str = "supervised";

/// How far down the Router ranking a planner's pre-assigned `member_index` may
/// sit and still be honored as the lead's deliberate judgment. Index 0 or 1
/// (top-2) counts as "a viable top candidate"; anything lower means the Router
/// found a materially better fit and overrides the planner.
const PLANNER_HONOR_TOP_K: usize = 2;

#[derive(Clone)]
pub struct RunService {
    run_repo: Arc<dyn IRunRepository>,
    fleet_repo: Arc<dyn IFleetRepository>,
    ws_repo: Arc<dyn IOrchWorkspaceRepository>,
    planner: Arc<dyn PlanProducer>,
    emitter: OrchestratorRunEventEmitter,
}

impl RunService {
    pub fn new(
        run_repo: Arc<dyn IRunRepository>,
        fleet_repo: Arc<dyn IFleetRepository>,
        ws_repo: Arc<dyn IOrchWorkspaceRepository>,
        planner: Arc<dyn PlanProducer>,
        emitter: OrchestratorRunEventEmitter,
    ) -> Self {
        Self {
            run_repo,
            fleet_repo,
            ws_repo,
            planner,
            emitter,
        }
    }

    /// Create a run: snapshot the chosen fleet's members into the run row and
    /// park it in `planning`. The snapshot makes the run reproducible even if
    /// the fleet is later edited or deleted (we never re-read `fleets` after
    /// this point — the engine resolves members from `fleet_snapshot`).
    pub async fn create(&self, user_id: &str, req: CreateRunRequest) -> Result<Run, AppError> {
        if req.goal.trim().is_empty() {
            return Err(OrchestratorError::BadRequest("goal must not be empty".into()).into());
        }
        // Confirm the workspace exists (clean 404 vs a later FK failure).
        if self
            .ws_repo
            .get(&req.workspace_id)
            .await
            .map_err(OrchestratorError::from)?
            .is_none()
        {
            return Err(OrchestratorError::NotFound(format!("workspace {}", req.workspace_id)).into());
        }
        // Load + snapshot the fleet's members.
        let members = self.load_fleet_members(&req.fleet_id).await?;
        let fleet_snapshot =
            serde_json::to_string(&members).unwrap_or_else(|_| "[]".to_string());

        let autonomy = req
            .autonomy
            .filter(|a| !a.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_AUTONOMY.to_string());

        let row = self
            .run_repo
            .create_run(CreateRunParams {
                workspace_id: Some(req.workspace_id),
                user_id: user_id.to_string(),
                goal: req.goal,
                fleet_snapshot,
                autonomy,
                max_parallel: req.max_parallel,
                // Workspace-backed run: no ad-hoc work_dir / pre-bound conversation.
                work_dir: None,
                lead_conv_id: None,
            })
            .await
            .map_err(OrchestratorError::from)?;

        let run = run_row_to_dto(row);
        // Status starts at `planning` (the repo INSERTs it); surface it on the bus.
        self.emitter.emit_run_status(&run.id, &run.status);
        Ok(run)
    }

    /// Create an **ad-hoc** run straight from a conversation: no workspace, no
    /// pre-built fleet. The fleet is synthesized on the fly from the request's
    /// [`ModelRange`] (one synthetic [`FleetMember`] per `provider+model`), and the
    /// run carries its own `work_dir` (the engine prefers it over a workspace dir).
    ///
    /// The run is parked in `planning` exactly like [`create`](Self::create) —
    /// the snapshot is opaque to the engine, which resolves members from
    /// `fleet_snapshot` by id. Synthetic member ids are minted unique + stable
    /// (`generate_prefixed_id("rmbr")`) so the engine's task→member resolution is
    /// deterministic.
    ///
    /// `ModelRange::Auto` is rejected here: its expansion to a concrete `range`
    /// requires provider access and is the caps_orchestrator layer's job (Task 3),
    /// which calls this with an already-expanded `Single`/`Range`.
    pub async fn create_adhoc(
        &self,
        user_id: &str,
        req: CreateAdhocRunRequest,
    ) -> Result<Run, AppError> {
        if req.goal.trim().is_empty() {
            return Err(OrchestratorError::BadRequest("goal must not be empty".into()).into());
        }
        // Synthesize the fleet from the model range (Single/Range only; Auto and
        // empty ranges are rejected — pinned_roles is parsed but ignored in P1).
        let members = build_members_from_range(&req.model_range)?;
        let fleet_snapshot =
            serde_json::to_string(&members).unwrap_or_else(|_| "[]".to_string());

        let autonomy = req
            .autonomy
            .filter(|a| !a.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_AUTONOMY.to_string());
        // An empty work_dir string is treated as absent (no dir).
        let work_dir = req
            .work_dir
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty());

        let row = self
            .run_repo
            .create_run(CreateRunParams {
                workspace_id: None,
                user_id: user_id.to_string(),
                goal: req.goal,
                fleet_snapshot,
                autonomy,
                max_parallel: req.max_parallel,
                work_dir,
                lead_conv_id: req.lead_conv_id,
            })
            .await
            .map_err(OrchestratorError::from)?;

        let run = run_row_to_dto(row);
        self.emitter.emit_run_status(&run.id, &run.status);
        Ok(run)
    }

    /// Full run detail: the run + its task DAG (tasks, dep edges, assignments)
    /// plus the run's frozen fleet snapshot decoded into `fleet_members`. The
    /// snapshot decode is fail-soft (parse error → empty vec + warn) so a
    /// corrupt snapshot never blocks reading the rest of the run.
    pub async fn get_detail(&self, id: &str) -> Result<RunDetail, AppError> {
        let row = self
            .run_repo
            .get_run(id)
            .await
            .map_err(OrchestratorError::from)?
            .ok_or_else(|| OrchestratorError::NotFound(format!("run {id}")))?;
        let tasks = self.run_repo.list_tasks(id).await.map_err(OrchestratorError::from)?;
        let deps = self.run_repo.list_deps(id).await.map_err(OrchestratorError::from)?;
        let assignments = self
            .run_repo
            .list_assignments(id)
            .await
            .map_err(OrchestratorError::from)?;
        let fleet_members = decode_fleet_snapshot(id, &row.fleet_snapshot);
        Ok(RunDetail {
            run: run_row_to_dto(row),
            tasks: tasks.into_iter().map(task_row_to_dto).collect(),
            deps: deps.into_iter().map(dep_row_to_dto).collect(),
            assignments: assignments.into_iter().map(assignment_row_to_dto).collect(),
            fleet_members,
        })
    }

    /// All runs in a workspace, newest first.
    pub async fn list(&self, workspace_id: &str) -> Result<Vec<Run>, AppError> {
        let rows = self
            .run_repo
            .list_runs(workspace_id)
            .await
            .map_err(OrchestratorError::from)?;
        Ok(rows.into_iter().map(run_row_to_dto).collect())
    }

    /// Plan a run: decompose the goal into a task DAG, persist tasks + deps +
    /// assignments, emit `planUpdated`, then apply the **autonomy gate** — an
    /// `interactive` run parks at `awaiting_plan_approval` (a human approves the
    /// plan before any worker dispatches); every other level flips to `running`.
    ///
    /// Edges are persisted AFTER all tasks are created so the planned `depends_on`
    /// indices can be resolved to the minted task ids. A planned task with no
    /// `member_index` (or an out-of-range one) defaults to member 0 — the engine
    /// requires an assignment to run a task, so defaulting is safer than skipping.
    pub async fn plan(&self, run_id: &str) -> Result<(), AppError> {
        let run = self
            .run_repo
            .get_run(run_id)
            .await
            .map_err(OrchestratorError::from)?
            .ok_or_else(|| OrchestratorError::NotFound(format!("run {run_id}")))?;

        let members: Vec<FleetMember> = decode_fleet_snapshot(run_id, &run.fleet_snapshot);

        let dag: PlannedDag = self.planner.produce(&run.goal, &members).await?;

        // 1. Create every task first (status `pending`); remember the minted ids
        //    in planned-index order so we can resolve dep edges + assignments.
        let mut task_ids: Vec<String> = Vec::with_capacity(dag.tasks.len());
        for planned in &dag.tasks {
            let task_profile = planned
                .task_profile
                .as_ref()
                .and_then(|p| serde_json::to_string(p).ok());
            let task = self
                .run_repo
                .create_task(CreateTaskParams {
                    run_id: run_id.to_string(),
                    title: planned.title.clone(),
                    spec: planned.spec.clone(),
                    task_profile,
                    status: "pending".to_string(),
                    graph_x: None,
                    graph_y: None,
                })
                .await
                .map_err(OrchestratorError::from)?;
            task_ids.push(task.id);
        }

        // 2. Dep edges: blocker (the depended-on, earlier task) → blocked (this).
        for (idx, planned) in dag.tasks.iter().enumerate() {
            let blocked_id = &task_ids[idx];
            for &dep_idx in &planned.depends_on {
                if let Some(blocker_id) = task_ids.get(dep_idx) {
                    self.run_repo
                        .add_dep(blocker_id, blocked_id)
                        .await
                        .map_err(OrchestratorError::from)?;
                } else {
                    tracing::warn!(
                        run_id,
                        task_idx = idx,
                        dep_idx,
                        "planner produced an out-of-range depends_on index; skipping edge"
                    );
                }
            }
        }

        // 3. Assignments via the capability Router. For each task we build a
        //    TaskProfile (the planner's, or a neutral default), rank the snapshot
        //    members, and pick:
        //    - the Router's top pick (`rank_members[0]`) by default;
        //    - the planner's `member_index` IF it survived the hard filters AND
        //      ranks in the top-K (we trust the lead's deliberate choice when the
        //      Router agrees it's a viable candidate);
        //    - a fallback to the planner's index / member 0 when every member was
        //      hard-filtered out (rank_members empty) — the engine needs an
        //      assignment to run the task, so leaving it unassigned would fail it.
        //    An existing *locked* assignment is never overwritten (re-plan must
        //    respect human overrides).
        for (idx, planned) in dag.tasks.iter().enumerate() {
            let task_id = &task_ids[idx];

            // Respect a locked assignment: re-plan must not touch it.
            if let Some(existing) = self
                .run_repo
                .get_assignment_for_task(task_id)
                .await
                .map_err(OrchestratorError::from)?
            {
                if existing.locked != 0 {
                    continue;
                }
            }

            let profile = planned.task_profile.clone().unwrap_or_else(default_profile);
            let ranked = rank_members(&members, &profile);

            // Decide the member index + the score/rationale to record.
            let pick = if ranked.is_empty() {
                // All hard-filtered: fall back so the task still gets assigned.
                resolve_member(&members, planned.member_index).map(|m| AssignmentPick {
                    member_id: m.id.clone(),
                    score: None,
                    rationale: planned.rationale.clone(),
                })
            } else {
                // Honor the planner's pre-assignment when it's a viable top
                // candidate (present in the ranking AND within the top-K).
                let planner_choice = planned.member_index.and_then(|mi| {
                    ranked
                        .iter()
                        .take(PLANNER_HONOR_TOP_K)
                        .find(|c| c.member_index == mi)
                });
                let chosen = planner_choice.unwrap_or(&ranked[0]);
                members.get(chosen.member_index).map(|m| AssignmentPick {
                    member_id: m.id.clone(),
                    score: Some(chosen.score),
                    rationale: Some(chosen.rationale.clone()),
                })
            };

            let Some(pick) = pick else {
                tracing::warn!(
                    run_id,
                    task_idx = idx,
                    "fleet snapshot has no members; cannot assign task (engine will fail it)"
                );
                continue;
            };

            // `plan` mints fresh tasks every call, so each task_id here is new and
            // has no prior assignment — `create_assignment` never stacks. The
            // locked-skip guard above is defensive (in case a future re-plan reuses
            // task ids): an existing locked assignment is honored, not overwritten.
            self.run_repo
                .create_assignment(CreateAssignmentParams {
                    task_id: task_id.clone(),
                    member_id: pick.member_id.clone(),
                    score: pick.score,
                    rationale: pick.rationale,
                    source: "auto".to_string(),
                    locked: false,
                })
                .await
                .map_err(OrchestratorError::from)?;
            self.emitter.emit_task_assigned(run_id, task_id, &pick.member_id);
        }

        self.emitter.emit_run_plan_updated(run_id);

        // Autonomy gate: `interactive` runs park at `awaiting_plan_approval` for a
        // human to confirm the plan before any worker dispatches (the human-in-the-
        // loop sits at the PLAN gate, not per-worker — workers always run yolo). All
        // other autonomy levels (`autonomous` / `supervised`) flip straight to
        // `running` so the engine may pick the run up. The route reads `run.autonomy`
        // to decide whether to `engine.start` (interactive: NOT until `approve_plan`).
        let next_status = if run.autonomy == "interactive" {
            "awaiting_plan_approval"
        } else {
            "running"
        };
        self.run_repo
            .update_run(
                run_id,
                UpdateRunParams {
                    status: Some(next_status.to_string()),
                    summary: None,
                    lead_conv_id: None,
                    total_tokens: None,
                },
            )
            .await
            .map_err(OrchestratorError::from)?;
        self.emitter.emit_run_status(run_id, next_status);
        Ok(())
    }

    /// Approve an `interactive` run's plan: `awaiting_plan_approval` → `running`
    /// + emit. The caller (route) then `engine.start`s the loop — `approve_plan`
    /// only mutates persisted state, mirroring how `create`/`plan` leave engine
    /// lifecycle to the route. A run not in `awaiting_plan_approval` is a 400
    /// (you cannot approve a plan that is already running / not awaiting approval).
    pub async fn approve_plan(&self, run_id: &str) -> Result<(), AppError> {
        self.transition(run_id, &["awaiting_plan_approval"], "running").await
    }

    /// Pause a `running` run: `running` → `paused` + emit. The engine's persistent
    /// loop keeps running but stops filling new workers (it re-reads the run status
    /// each iteration); any in-flight workers run to completion. Pause does NOT
    /// cancel in-flight work (that is `cancel`). A run not `running` is a 400.
    pub async fn pause(&self, run_id: &str) -> Result<(), AppError> {
        self.transition(run_id, &["running"], "paused").await
    }

    /// Resume a `paused` run: `paused` → `running` + emit. The caller (route) then
    /// `engine.start`s the loop (idempotent stop-then-start): if the loop was still
    /// alive idling on the paused gate it resumes filling on its next iteration; if
    /// it had exited, `start` respawns it. A run not `paused` is a 400.
    pub async fn resume(&self, run_id: &str) -> Result<(), AppError> {
        self.transition(run_id, &["paused"], "running").await
    }

    /// Shared status-transition helper for the run lifecycle controls
    /// (`approve_plan` / `pause` / `resume`): load the run (clean 404), require its
    /// current status to be one of `from` (else a 400 that names the actual state),
    /// then persist `to` + emit `run.statusChanged`.
    async fn transition(
        &self,
        run_id: &str,
        from: &[&str],
        to: &str,
    ) -> Result<(), AppError> {
        let run = self
            .run_repo
            .get_run(run_id)
            .await
            .map_err(OrchestratorError::from)?
            .ok_or_else(|| OrchestratorError::NotFound(format!("run {run_id}")))?;
        if !from.contains(&run.status.as_str()) {
            return Err(OrchestratorError::BadRequest(format!(
                "run {run_id} is `{}`, cannot transition to `{to}` (expected one of {from:?})",
                run.status
            ))
            .into());
        }
        self.run_repo
            .update_run(
                run_id,
                UpdateRunParams {
                    status: Some(to.to_string()),
                    summary: None,
                    lead_conv_id: None,
                    total_tokens: None,
                },
            )
            .await
            .map_err(OrchestratorError::from)?;
        self.emitter.emit_run_status(run_id, to);
        Ok(())
    }

    /// Override (or lock) the member assigned to a task. This is the human
    /// reassign path: it upserts the task's assignment to the requested member
    /// with `source = "override"`. `locked` defaults to `true` — a deliberate
    /// override should survive a later re-plan — unless the caller explicitly
    /// passes `false`.
    ///
    /// We verify the run + task exist (clean 404s) and that the member is part of
    /// the run's frozen fleet snapshot (a 400 otherwise — you cannot assign a
    /// member the run was never created with).
    pub async fn reassign(
        &self,
        run_id: &str,
        task_id: &str,
        req: ReassignRequest,
    ) -> Result<(), AppError> {
        let run = self
            .run_repo
            .get_run(run_id)
            .await
            .map_err(OrchestratorError::from)?
            .ok_or_else(|| OrchestratorError::NotFound(format!("run {run_id}")))?;

        // The task must exist and belong to this run.
        let task = self
            .run_repo
            .get_task(task_id)
            .await
            .map_err(OrchestratorError::from)?
            .ok_or_else(|| OrchestratorError::NotFound(format!("task {task_id}")))?;
        if task.run_id != run_id {
            return Err(OrchestratorError::NotFound(format!("task {task_id} in run {run_id}")).into());
        }

        // The member must be part of the run's frozen fleet snapshot.
        let members = decode_fleet_snapshot(run_id, &run.fleet_snapshot);
        if !members.iter().any(|m| m.id == req.member_id) {
            return Err(OrchestratorError::BadRequest(format!(
                "member {} is not in run {run_id}'s fleet snapshot",
                req.member_id
            ))
            .into());
        }

        let locked = req.locked.unwrap_or(true);
        self.run_repo
            .set_assignment(CreateAssignmentParams {
                task_id: task_id.to_string(),
                member_id: req.member_id.clone(),
                score: None,
                rationale: Some("人工指派".to_string()),
                source: "override".to_string(),
                locked,
            })
            .await
            .map_err(OrchestratorError::from)?;
        self.emitter.emit_task_assigned(run_id, task_id, &req.member_id);
        Ok(())
    }

    /// Cancel a run: flip it to `cancelled` and emit. The engine's cooperative
    /// cancel (set via [`RunEngine::stop`](crate::engine::RunEngine::stop)) is
    /// the runtime counterpart; this is the persisted state change.
    pub async fn cancel(&self, run_id: &str) -> Result<(), AppError> {
        // Confirm it exists for a clean 404.
        if self
            .run_repo
            .get_run(run_id)
            .await
            .map_err(OrchestratorError::from)?
            .is_none()
        {
            return Err(OrchestratorError::NotFound(format!("run {run_id}")).into());
        }
        self.run_repo
            .update_run(
                run_id,
                UpdateRunParams {
                    status: Some("cancelled".to_string()),
                    summary: None,
                    lead_conv_id: None,
                    total_tokens: None,
                },
            )
            .await
            .map_err(OrchestratorError::from)?;
        self.emitter.emit_run_status(run_id, "cancelled");
        self.emitter.emit_run_completed(run_id, "cancelled");
        Ok(())
    }

    /// Load the chosen fleet's members as DTOs (decoding JSON columns fail-soft).
    /// A missing fleet is a clean 404.
    async fn load_fleet_members(&self, fleet_id: &str) -> Result<Vec<FleetMember>, AppError> {
        if self
            .fleet_repo
            .get_fleet(fleet_id)
            .await
            .map_err(OrchestratorError::from)?
            .is_none()
        {
            return Err(OrchestratorError::NotFound(format!("fleet {fleet_id}")).into());
        }
        let rows = self
            .fleet_repo
            .list_members(fleet_id)
            .await
            .map_err(OrchestratorError::from)?;
        Ok(rows.into_iter().map(member_row_to_dto).collect())
    }
}

/// Resolve a planned `member_index` to a fleet-snapshot member, defaulting to
/// member 0 when the index is unset or out of range. Returns `None` only when
/// the snapshot is empty.
fn resolve_member(members: &[FleetMember], member_index: Option<usize>) -> Option<&FleetMember> {
    match member_index {
        Some(i) => members.get(i).or_else(|| members.first()),
        None => members.first(),
    }
}

/// Synthesize the ad-hoc fleet members for a [`ModelRange`]. Each `provider+model`
/// pair becomes one [`FleetMember`] with both `provider_id` and `model` set to
/// `Some` (the worker hard-requires both — `worker.rs:116-120`), a freshly minted
/// unique id (`generate_prefixed_id("rmbr")` — the engine resolves task→member by
/// id, so ids must be unique + stable), and `sort_order` = its snapshot index.
/// The member is otherwise bare (`agent_id` empty, no capability profile /
/// constraints / role hint) — capability routing in P1 falls back to the neutral
/// default profile.
///
/// **Contract (Task 3):** only `Single` and `Range` are handled here. `Auto` is a
/// `BadRequest` — its expansion to a concrete `Range` requires provider access and
/// belongs in the caps_orchestrator layer, which must pass an already-expanded
/// range. An empty `Range` is also a `BadRequest` (a run needs at least one model).
fn build_members_from_range(range: &ModelRange) -> Result<Vec<FleetMember>, AppError> {
    let model_refs: Vec<(&str, &str)> = match range {
        ModelRange::Single { model } => {
            vec![(model.provider_id.as_str(), model.model.as_str())]
        }
        ModelRange::Range { models } => models
            .iter()
            .map(|m| (m.provider_id.as_str(), m.model.as_str()))
            .collect(),
        ModelRange::Auto => {
            // Caller (caps_orchestrator, Task 3) must expand Auto to a concrete
            // Range before reaching the service — it has provider access; we don't.
            return Err(OrchestratorError::BadRequest(
                "auto range must be expanded by caller".into(),
            )
            .into());
        }
    };

    if model_refs.is_empty() {
        return Err(
            OrchestratorError::BadRequest("model_range 为空：无可用模型".into()).into(),
        );
    }

    let members = model_refs
        .into_iter()
        .enumerate()
        .map(|(i, (provider_id, model))| FleetMember {
            id: generate_prefixed_id("rmbr"),
            agent_id: String::new(),
            provider_id: Some(provider_id.to_string()),
            model: Some(model.to_string()),
            role_hint: None,
            capability_profile: None,
            constraints: None,
            sort_order: i as i64,
        })
        .collect();
    Ok(members)
}

/// The member + score/rationale chosen for a task during planning.
struct AssignmentPick {
    member_id: String,
    score: Option<f64>,
    rationale: Option<String>,
}

/// A neutral default profile for tasks the planner left unprofiled: a `general`
/// task with no special modality / reasoning / bulk requirements. The Router
/// scores every member against this without hard-filtering anyone out.
fn default_profile() -> TaskProfile {
    TaskProfile {
        kind: "general".to_string(),
        needs_vision: false,
        needs_long_context: false,
        needs_high_reasoning: false,
        bulk: false,
    }
}

/// Decode a run's `fleet_snapshot` JSON into its members, fail-soft: a parse
/// error logs a warning and yields an empty vec (never blocks the read path).
fn decode_fleet_snapshot(run_id: &str, snapshot: &str) -> Vec<FleetMember> {
    match serde_json::from_str::<Vec<FleetMember>>(snapshot) {
        Ok(members) => members,
        Err(err) => {
            tracing::warn!(run_id, error = %err, "failed to decode fleet_snapshot; using empty members");
            Vec::new()
        }
    }
}

// --- Row → DTO mapping ------------------------------------------------------

fn run_row_to_dto(row: OrchRunRow) -> Run {
    Run {
        id: row.id,
        workspace_id: row.workspace_id,
        goal: row.goal,
        autonomy: row.autonomy,
        max_parallel: row.max_parallel,
        status: row.status,
        summary: row.summary,
        lead_conv_id: row.lead_conv_id,
        total_tokens: row.total_tokens,
        work_dir: row.work_dir,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn task_row_to_dto(row: OrchRunTaskRow) -> RunTask {
    let task_profile = row
        .task_profile
        .as_deref()
        .and_then(|raw| serde_json::from_str::<TaskProfile>(raw).ok());
    // `output_files` is a JSON array of strings; decode fail-soft to empty.
    let output_files = row
        .output_files
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default();
    RunTask {
        id: row.id,
        run_id: row.run_id,
        title: row.title,
        spec: row.spec,
        task_profile,
        status: row.status,
        conversation_id: row.conversation_id,
        // PROSE pass-through — NOT JSON (the Row comment is misleading).
        output_summary: row.output_summary,
        output_files,
        attempt: row.attempt,
        tokens: row.tokens,
        graph_x: row.graph_x,
        graph_y: row.graph_y,
    }
}

fn dep_row_to_dto(row: OrchRunTaskDepRow) -> RunTaskDep {
    RunTaskDep {
        blocker_task_id: row.blocker_task_id,
        blocked_task_id: row.blocked_task_id,
    }
}

fn assignment_row_to_dto(row: OrchAssignmentRow) -> Assignment {
    Assignment {
        id: row.id,
        task_id: row.task_id,
        member_id: row.member_id,
        score: row.score,
        rationale: row.rationale,
        source: row.source,
        locked: row.locked != 0,
    }
}

/// Map a fleet member DB row to its DTO, decoding the JSON columns fail-soft
/// (mirrors `service::member_row_to_dto`). Kept local so `RunService` stays
/// self-contained over the raw `fleet_repo`.
fn member_row_to_dto(row: FleetMemberRow) -> FleetMember {
    let capability_profile = row
        .capability_profile
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok());
    let constraints = row
        .constraints
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok());
    FleetMember {
        id: row.id,
        agent_id: row.agent_id,
        provider_id: row.provider_id,
        model: row.model,
        role_hint: row.role_hint,
        capability_profile,
        constraints,
        sort_order: row.sort_order,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{FleetService, WorkspaceService};
    use nomifun_api_types::{
        CapabilityProfile, CreateAdhocRunRequest, CreateFleetRequest, CreateWorkspaceRequest,
        FleetMemberInput, ModelRange, ModelRef, PlannedTask, WebSocketMessage,
    };
    use nomifun_db::{
        SqliteFleetRepository, SqliteOrchWorkspaceRepository, SqliteRunRepository,
        init_database_memory,
    };
    use nomifun_realtime::EventBroadcaster;
    use std::sync::Mutex;

    /// No-op broadcaster: these tests assert persisted state, not the event trail.
    struct NoopBroadcaster;
    impl EventBroadcaster for NoopBroadcaster {
        fn broadcast(&self, _event: WebSocketMessage<serde_json::Value>) {}
    }

    /// A planner that returns a fixed [`PlannedDag`] regardless of goal/members,
    /// so each test controls the exact tasks (and their member_index / profile).
    struct FixedPlanProducer(Mutex<PlannedDag>);
    impl FixedPlanProducer {
        fn new(dag: PlannedDag) -> Self {
            Self(Mutex::new(dag))
        }
    }
    #[async_trait::async_trait]
    impl PlanProducer for FixedPlanProducer {
        async fn produce(
            &self,
            _goal: &str,
            _members: &[FleetMember],
        ) -> Result<PlannedDag, AppError> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    /// Member input with a specific capability profile (strengths/reasoning/cost).
    fn member_input(
        agent_id: &str,
        strengths: &[&str],
        reasoning: &str,
        cost_tier: &str,
    ) -> FleetMemberInput {
        FleetMemberInput {
            agent_id: agent_id.to_string(),
            provider_id: None,
            model: None,
            role_hint: None,
            capability_profile: Some(CapabilityProfile {
                strengths: strengths.iter().map(|s| s.to_string()).collect(),
                modalities: vec!["text".to_string()],
                tools: true,
                reasoning: reasoning.to_string(),
                cost_tier: cost_tier.to_string(),
                speed_tier: "standard".to_string(),
            }),
            constraints: None,
            sort_order: None,
        }
    }

    fn coding_profile() -> TaskProfile {
        TaskProfile {
            kind: "coding".to_string(),
            needs_vision: false,
            needs_long_context: false,
            needs_high_reasoning: true,
            bulk: false,
        }
    }

    /// Build a fully-wired RunService + repos, seed a workspace + a fleet with the
    /// given members, create a run (parked in `planning`), and return everything a
    /// test needs. The planner is `FixedPlanProducer(dag)`.
    async fn harness(
        members: Vec<FleetMemberInput>,
        dag: PlannedDag,
    ) -> (RunService, Arc<SqliteRunRepository>, Vec<FleetMember>, String) {
        let db = init_database_memory().await.expect("db init");
        let pool = db.pool().clone();
        let run_repo = Arc::new(SqliteRunRepository::new(pool.clone()));
        let fleet_repo = Arc::new(SqliteFleetRepository::new(pool.clone()));
        let ws_repo = Arc::new(SqliteOrchWorkspaceRepository::new(pool.clone()));
        let emitter = OrchestratorRunEventEmitter::new(Arc::new(NoopBroadcaster));
        let planner: Arc<dyn PlanProducer> = Arc::new(FixedPlanProducer::new(dag));
        let run_service = RunService::new(
            run_repo.clone(),
            fleet_repo.clone(),
            ws_repo.clone(),
            planner,
            emitter,
        );

        let fleet = FleetService::new(fleet_repo)
            .create(
                "u1",
                CreateFleetRequest {
                    name: "router fleet".to_string(),
                    description: None,
                    max_parallel: None,
                    members,
                },
            )
            .await
            .expect("fleet create");
        let ws = WorkspaceService::new(ws_repo)
            .create(
                "u1",
                CreateWorkspaceRequest {
                    name: "router ws".to_string(),
                    default_fleet_id: Some(fleet.id.clone()),
                    workspace_dir: None,
                },
            )
            .await
            .expect("ws create");
        let run = run_service
            .create(
                "u1",
                CreateRunRequest {
                    workspace_id: ws.id,
                    goal: "do the thing".to_string(),
                    fleet_id: fleet.id,
                    autonomy: None,
                    max_parallel: None,
                },
            )
            .await
            .expect("run create");

        // The run snapshotted the fleet; read it back so the test knows member ids.
        let snapshot = run_service.get_detail(&run.id).await.expect("detail");
        (run_service, run_repo, snapshot.fleet_members, run.id)
    }

    /// A single-task DAG with the given member_index / profile.
    fn single_task_dag(member_index: Option<usize>, profile: Option<TaskProfile>) -> PlannedDag {
        PlannedDag {
            tasks: vec![PlannedTask {
                title: "task".to_string(),
                spec: "spec".to_string(),
                task_profile: profile,
                depends_on: vec![],
                member_index,
                rationale: None,
            }],
        }
    }

    // (a) plan picks the Router's top member when it is clearly more capable than
    // the others (coding + high reasoning task → the coding/high member wins),
    // even though the planner left member_index unset. The assignment records a
    // non-empty rationale + a score.
    #[tokio::test]
    async fn plan_picks_router_top_member() {
        // index 0: generalist (no strengths, medium reasoning); index 1: coding +
        // high reasoning — a far better fit for the coding profile.
        let members = vec![
            member_input("agent_gen", &[], "medium", "standard"),
            member_input("agent_coder", &["coding"], "high", "standard"),
        ];
        let dag = single_task_dag(None, Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;

        svc.plan(&run_id).await.expect("plan");

        let detail = svc.get_detail(&run_id).await.expect("detail");
        assert_eq!(detail.assignments.len(), 1);
        let asg = &detail.assignments[0];
        // The coder (member index 1) must win.
        assert_eq!(asg.member_id, snapshot[1].id, "router must pick the coder");
        assert_eq!(asg.source, "auto");
        assert!(!asg.locked);
        assert!(asg.score.is_some(), "router records a score");
        let rationale = asg.rationale.as_deref().unwrap_or("");
        assert!(!rationale.is_empty(), "rationale must be non-empty");
        assert!(
            rationale.contains("强项匹配[coding]"),
            "rationale should explain the coding match, got: {rationale}"
        );
    }

    // (b) plan honors the planner's member_index when it is a viable top
    // candidate. Both members are coders (tied scores → both in top-2), so a
    // planner pre-assignment of the (otherwise tie-broken-second) member 1 must
    // be honored rather than overridden to member 0.
    #[tokio::test]
    async fn plan_honors_planner_member_index_when_viable() {
        let members = vec![
            member_input("agent_a", &["coding"], "high", "standard"),
            member_input("agent_b", &["coding"], "high", "standard"),
        ];
        // Planner deliberately chose member 1.
        let dag = single_task_dag(Some(1), Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;

        svc.plan(&run_id).await.expect("plan");

        let detail = svc.get_detail(&run_id).await.expect("detail");
        assert_eq!(detail.assignments.len(), 1);
        assert_eq!(
            detail.assignments[0].member_id, snapshot[1].id,
            "planner's viable choice (member 1) must be honored"
        );
    }

    // (b') the inverse: when the planner's member_index is NOT a top candidate
    // (member 0 is a weak generalist, the task strongly favors the coder at index
    // 1), the Router overrides the planner.
    #[tokio::test]
    async fn plan_overrides_planner_when_not_top_candidate() {
        // 3 members so member 0 falls outside the top-2 for a coding task.
        let members = vec![
            member_input("agent_gen", &[], "low", "standard"), // weak
            member_input("agent_c1", &["coding"], "high", "standard"),
            member_input("agent_c2", &["coding"], "high", "standard"),
        ];
        // Planner picked the weak generalist (index 0).
        let dag = single_task_dag(Some(0), Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;

        svc.plan(&run_id).await.expect("plan");

        let detail = svc.get_detail(&run_id).await.expect("detail");
        let chosen = &detail.assignments[0].member_id;
        assert_ne!(*chosen, snapshot[0].id, "weak generalist must be overridden");
        assert!(
            *chosen == snapshot[1].id || *chosen == snapshot[2].id,
            "router must pick one of the coders"
        );
    }

    // plan falls back to the planner's member_index when every member is
    // hard-filtered out (e.g. a vision task with no vision-capable members) so the
    // task still gets an assignment.
    #[tokio::test]
    async fn plan_falls_back_when_all_hard_filtered() {
        let members = vec![
            member_input("agent_a", &["coding"], "high", "standard"),
            member_input("agent_b", &["writing"], "medium", "standard"),
        ];
        // Vision task; neither member declares the "vision" modality → all excluded.
        let vision = TaskProfile {
            kind: "analysis".to_string(),
            needs_vision: true,
            needs_long_context: false,
            needs_high_reasoning: false,
            bulk: false,
        };
        let dag = single_task_dag(Some(1), Some(vision));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;

        svc.plan(&run_id).await.expect("plan");

        let detail = svc.get_detail(&run_id).await.expect("detail");
        assert_eq!(detail.assignments.len(), 1, "task still assigned on fallback");
        assert_eq!(
            detail.assignments[0].member_id, snapshot[1].id,
            "fallback honors the planner's member_index"
        );
    }

    // (c) reassign sets source='override' and locked.
    #[tokio::test]
    async fn reassign_sets_override_and_locked() {
        let members = vec![
            member_input("agent_a", &["coding"], "high", "standard"),
            member_input("agent_b", &["writing"], "medium", "standard"),
        ];
        let dag = single_task_dag(None, Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;
        svc.plan(&run_id).await.expect("plan");

        let detail = svc.get_detail(&run_id).await.expect("detail");
        let task_id = detail.tasks[0].id.clone();
        // Auto-pick is the coder (member 0). Override to member 1.
        assert_eq!(detail.assignments[0].member_id, snapshot[0].id);

        svc.reassign(
            &run_id,
            &task_id,
            ReassignRequest {
                member_id: snapshot[1].id.clone(),
                locked: None, // default → true
            },
        )
        .await
        .expect("reassign");

        let after = svc.get_detail(&run_id).await.expect("detail");
        // Exactly one assignment (upsert replaced, not stacked).
        assert_eq!(after.assignments.len(), 1, "reassign upserts (no stacking)");
        let asg = &after.assignments[0];
        assert_eq!(asg.member_id, snapshot[1].id, "member overridden");
        assert_eq!(asg.source, "override");
        assert!(asg.locked, "locked defaults to true on override");
    }

    // reassign with locked=false overrides without locking.
    #[tokio::test]
    async fn reassign_unlocked_does_not_lock() {
        let members = vec![
            member_input("agent_a", &["coding"], "high", "standard"),
            member_input("agent_b", &["writing"], "medium", "standard"),
        ];
        let dag = single_task_dag(None, Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;
        svc.plan(&run_id).await.expect("plan");
        let task_id = svc.get_detail(&run_id).await.unwrap().tasks[0].id.clone();

        svc.reassign(
            &run_id,
            &task_id,
            ReassignRequest {
                member_id: snapshot[1].id.clone(),
                locked: Some(false),
            },
        )
        .await
        .expect("reassign");

        let asg = &svc.get_detail(&run_id).await.unwrap().assignments[0];
        assert_eq!(asg.source, "override");
        assert!(!asg.locked, "explicit locked=false must not lock");
    }

    // (d) re-plan does NOT change a locked assignment. (The locked-skip guard in
    // `plan` is exercised here for the original task; in the current flow `plan`
    // also mints fresh tasks, so the locked task is preserved either way — this
    // asserts the user's override survives a re-plan regardless.)
    #[tokio::test]
    async fn replan_does_not_touch_locked_assignment() {
        let members = vec![
            member_input("agent_a", &["coding"], "high", "standard"),
            member_input("agent_b", &["writing"], "medium", "standard"),
        ];
        let dag = single_task_dag(None, Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;
        svc.plan(&run_id).await.expect("first plan");
        let task_id = svc.get_detail(&run_id).await.unwrap().tasks[0].id.clone();

        // Lock the task to member 1 (the writer — NOT what the router would pick).
        svc.reassign(
            &run_id,
            &task_id,
            ReassignRequest {
                member_id: snapshot[1].id.clone(),
                locked: Some(true),
            },
        )
        .await
        .expect("reassign");

        // Re-plan: the locked task must keep its override.
        svc.plan(&run_id).await.expect("re-plan");

        // Find the assignment for the original (locked) task.
        let detail = svc.get_detail(&run_id).await.expect("detail");
        let locked_asg = detail
            .assignments
            .iter()
            .find(|a| a.task_id == task_id)
            .expect("locked task assignment");
        assert_eq!(
            locked_asg.member_id, snapshot[1].id,
            "locked assignment must survive re-plan"
        );
        assert_eq!(locked_asg.source, "override");
        assert!(locked_asg.locked);
    }

    // re-plan appends a fresh task DAG (plan mints new task ids each call); the
    // prior plan's tasks + assignments are left intact. This asserts a re-plan
    // does not retroactively rewrite the earlier auto assignment.
    #[tokio::test]
    async fn replan_appends_without_rewriting_prior_assignment() {
        let members = vec![member_input("agent_a", &["coding"], "high", "standard")];
        let dag = single_task_dag(None, Some(coding_profile()));
        let (svc, _repo, snapshot, run_id) = harness(members, dag).await;
        svc.plan(&run_id).await.expect("first plan");
        let first = svc.get_detail(&run_id).await.expect("detail");
        let first_task = first.tasks[0].id.clone();
        assert_eq!(first.assignments.len(), 1);

        svc.plan(&run_id).await.expect("re-plan");

        let detail = svc.get_detail(&run_id).await.expect("detail");
        // Each task carries exactly one auto assignment (no per-task stacking).
        let first_assigns: Vec<_> = detail
            .assignments
            .iter()
            .filter(|a| a.task_id == first_task)
            .collect();
        assert_eq!(first_assigns.len(), 1, "prior task keeps a single assignment");
        assert_eq!(first_assigns[0].member_id, snapshot[0].id);
        assert_eq!(first_assigns[0].source, "auto");
    }

    // (e) get_detail returns fleet_members decoded from the run's snapshot.
    #[tokio::test]
    async fn get_detail_returns_fleet_members() {
        let members = vec![
            member_input("agent_a", &["coding"], "high", "premium"),
            member_input("agent_b", &["writing"], "medium", "economy"),
        ];
        let dag = single_task_dag(None, None);
        let (svc, _repo, _snapshot, run_id) = harness(members, dag).await;

        let detail = svc.get_detail(&run_id).await.expect("detail");
        assert_eq!(detail.fleet_members.len(), 2, "snapshot members surfaced");
        assert_eq!(detail.fleet_members[0].agent_id, "agent_a");
        assert_eq!(detail.fleet_members[1].agent_id, "agent_b");
        // Capability profile survived the snapshot round-trip.
        let cap0 = detail.fleet_members[0]
            .capability_profile
            .as_ref()
            .expect("cap profile");
        assert_eq!(cap0.strengths, vec!["coding"]);
    }

    // reassign rejects a member that is not in the run's fleet snapshot (400).
    #[tokio::test]
    async fn reassign_unknown_member_is_bad_request() {
        let members = vec![member_input("agent_a", &["coding"], "high", "standard")];
        let dag = single_task_dag(None, Some(coding_profile()));
        let (svc, _repo, _snapshot, run_id) = harness(members, dag).await;
        svc.plan(&run_id).await.expect("plan");
        let task_id = svc.get_detail(&run_id).await.unwrap().tasks[0].id.clone();

        let err = svc
            .reassign(
                &run_id,
                &task_id,
                ReassignRequest {
                    member_id: "fmem_not_real".to_string(),
                    locked: None,
                },
            )
            .await
            .expect_err("unknown member must error");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");
    }

    // -------------------------------------------------------------------------
    // P3b: autonomy gate (plan's next-status decision) + pause/resume/approve.
    // -------------------------------------------------------------------------

    /// Build a harness with an explicit `autonomy` on the created run (the default
    /// harness leaves autonomy None → "supervised"). Returns (svc, run_id).
    async fn harness_with_autonomy(autonomy: &str) -> (RunService, String) {
        let db = init_database_memory().await.expect("db init");
        let pool = db.pool().clone();
        let run_repo = Arc::new(SqliteRunRepository::new(pool.clone()));
        let fleet_repo = Arc::new(SqliteFleetRepository::new(pool.clone()));
        let ws_repo = Arc::new(SqliteOrchWorkspaceRepository::new(pool.clone()));
        let emitter = OrchestratorRunEventEmitter::new(Arc::new(NoopBroadcaster));
        let planner: Arc<dyn PlanProducer> =
            Arc::new(FixedPlanProducer::new(single_task_dag(None, Some(coding_profile()))));
        let svc = RunService::new(
            run_repo,
            fleet_repo.clone(),
            ws_repo.clone(),
            planner,
            emitter,
        );
        let fleet = FleetService::new(fleet_repo)
            .create(
                "u1",
                CreateFleetRequest {
                    name: "auto fleet".to_string(),
                    description: None,
                    max_parallel: None,
                    members: vec![member_input("agent_a", &["coding"], "high", "standard")],
                },
            )
            .await
            .expect("fleet create");
        let ws = WorkspaceService::new(ws_repo)
            .create(
                "u1",
                CreateWorkspaceRequest {
                    name: "auto ws".to_string(),
                    default_fleet_id: Some(fleet.id.clone()),
                    workspace_dir: None,
                },
            )
            .await
            .expect("ws create");
        let run = svc
            .create(
                "u1",
                CreateRunRequest {
                    workspace_id: ws.id,
                    goal: "do the thing".to_string(),
                    fleet_id: fleet.id,
                    autonomy: Some(autonomy.to_string()),
                    max_parallel: None,
                },
            )
            .await
            .expect("run create");
        (svc, run.id)
    }

    // (a) An `interactive` run parks at `awaiting_plan_approval` after plan (NOT
    // running), and `approve_plan` flips it to `running`.
    #[tokio::test]
    async fn interactive_run_parks_then_approves_to_running() {
        let (svc, run_id) = harness_with_autonomy("interactive").await;
        svc.plan(&run_id).await.expect("plan");
        let detail = svc.get_detail(&run_id).await.expect("detail");
        assert_eq!(
            detail.run.status, "awaiting_plan_approval",
            "interactive run must park at awaiting_plan_approval after plan"
        );

        svc.approve_plan(&run_id).await.expect("approve");
        let detail = svc.get_detail(&run_id).await.expect("detail");
        assert_eq!(detail.run.status, "running", "approve flips to running");
    }

    // A `supervised` (default) / `autonomous` run flips straight to `running`
    // after plan (no approval gate).
    #[tokio::test]
    async fn non_interactive_run_is_running_after_plan() {
        for autonomy in ["supervised", "autonomous"] {
            let (svc, run_id) = harness_with_autonomy(autonomy).await;
            svc.plan(&run_id).await.expect("plan");
            let detail = svc.get_detail(&run_id).await.expect("detail");
            assert_eq!(detail.run.status, "running", "{autonomy} run runs after plan");
        }
    }

    // approve_plan rejects a run that is NOT awaiting approval (e.g. already
    // running) with a 400.
    #[tokio::test]
    async fn approve_plan_rejects_non_awaiting_run() {
        let (svc, run_id) = harness_with_autonomy("supervised").await;
        svc.plan(&run_id).await.expect("plan"); // → running
        let err = svc.approve_plan(&run_id).await.expect_err("must reject");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");
    }

    // (b) pause: running → paused; resume: paused → running. Each emits.
    #[tokio::test]
    async fn pause_then_resume_round_trips_status() {
        let (svc, run_id) = harness_with_autonomy("supervised").await;
        svc.plan(&run_id).await.expect("plan"); // → running

        svc.pause(&run_id).await.expect("pause");
        assert_eq!(
            svc.get_detail(&run_id).await.unwrap().run.status,
            "paused",
            "pause sets paused"
        );

        svc.resume(&run_id).await.expect("resume");
        assert_eq!(
            svc.get_detail(&run_id).await.unwrap().run.status,
            "running",
            "resume sets running"
        );
    }

    // pause rejects a run that is not running (e.g. still planning); resume
    // rejects a run that is not paused. Both 400.
    #[tokio::test]
    async fn pause_resume_reject_wrong_state() {
        let (svc, run_id) = harness_with_autonomy("supervised").await;
        // Before plan the run is `planning` — pause must reject.
        let err = svc.pause(&run_id).await.expect_err("pause on planning must reject");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");

        svc.plan(&run_id).await.expect("plan"); // → running
        // running is not paused — resume must reject.
        let err = svc.resume(&run_id).await.expect_err("resume on running must reject");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");
    }

    // The lifecycle controls 404 on a missing run.
    #[tokio::test]
    async fn lifecycle_controls_404_on_missing_run() {
        let (svc, _run_id) = harness_with_autonomy("supervised").await;
        let missing = "run_does_not_exist";
        assert!(
            matches!(svc.approve_plan(missing).await, Err(AppError::NotFound(_))),
            "approve_plan must 404 on missing run"
        );
        assert!(
            matches!(svc.pause(missing).await, Err(AppError::NotFound(_))),
            "pause must 404 on missing run"
        );
        assert!(
            matches!(svc.resume(missing).await, Err(AppError::NotFound(_))),
            "resume must 404 on missing run"
        );
    }

    // -------------------------------------------------------------------------
    // Task 2: ad-hoc (workspace-less) run creation from a model range.
    // -------------------------------------------------------------------------

    /// Build a bare RunService (no fleet/workspace seeded) for the ad-hoc path:
    /// `create_adhoc` synthesizes its members from a model range, so it needs
    /// neither a fleet nor a workspace. Returns (svc, run_repo).
    async fn adhoc_service() -> (RunService, Arc<SqliteRunRepository>) {
        let db = init_database_memory().await.expect("db init");
        let pool = db.pool().clone();
        let run_repo = Arc::new(SqliteRunRepository::new(pool.clone()));
        let fleet_repo = Arc::new(SqliteFleetRepository::new(pool.clone()));
        let ws_repo = Arc::new(SqliteOrchWorkspaceRepository::new(pool.clone()));
        let emitter = OrchestratorRunEventEmitter::new(Arc::new(NoopBroadcaster));
        let planner: Arc<dyn PlanProducer> =
            Arc::new(FixedPlanProducer::new(single_task_dag(None, None)));
        let svc = RunService::new(run_repo.clone(), fleet_repo, ws_repo, planner, emitter);
        (svc, run_repo)
    }

    fn model_ref(provider_id: &str, model: &str) -> ModelRef {
        ModelRef {
            provider_id: provider_id.to_string(),
            model: model.to_string(),
        }
    }

    // (a) create_adhoc with a `range` of two models persists the run, snapshots
    // exactly two synthetic members (each with provider_id + model both Some),
    // and persists work_dir + lead_conv_id.
    #[tokio::test]
    async fn create_adhoc_range_snapshots_two_members() {
        let (svc, _repo) = adhoc_service().await;
        let req = CreateAdhocRunRequest {
            goal: "ship the feature".to_string(),
            work_dir: Some("/tmp/proj".to_string()),
            model_range: ModelRange::Range {
                models: vec![model_ref("prov_a", "model-a"), model_ref("prov_b", "model-b")],
            },
            pinned_roles: vec![],
            autonomy: None,
            max_parallel: Some(2),
            lead_conv_id: Some(909),
        };

        let run = svc.create_adhoc("u1", req).await.expect("create_adhoc");

        // Ad-hoc run is workspace-less, carries its own work_dir + lead_conv_id.
        assert!(run.workspace_id.is_none(), "ad-hoc run has no workspace");
        assert_eq!(run.work_dir.as_deref(), Some("/tmp/proj"), "work_dir persisted");
        assert_eq!(run.lead_conv_id, Some(909), "lead_conv_id persisted");
        assert_eq!(run.status, "planning", "starts in planning");
        // Autonomy defaulted (request omitted it).
        assert_eq!(run.autonomy, DEFAULT_AUTONOMY);
        assert_eq!(run.max_parallel, Some(2));

        // The fleet snapshot must decode to two members, each Nomi-runnable
        // (provider_id + model both Some — worker.rs:116-120 requires it).
        let detail = svc.get_detail(&run.id).await.expect("detail");
        assert_eq!(detail.fleet_members.len(), 2, "two synthetic members snapshotted");
        let m0 = &detail.fleet_members[0];
        let m1 = &detail.fleet_members[1];
        assert_eq!(m0.provider_id.as_deref(), Some("prov_a"));
        assert_eq!(m0.model.as_deref(), Some("model-a"));
        assert_eq!(m1.provider_id.as_deref(), Some("prov_b"));
        assert_eq!(m1.model.as_deref(), Some("model-b"));
        // Member ids must be unique + stable (the engine resolves task→member by id).
        assert_ne!(m0.id, m1.id, "synthetic member ids are unique");
        assert!(m0.id.starts_with("rmbr_"), "member id uses the rmbr prefix: {}", m0.id);
        // sort_order is the snapshot index.
        assert_eq!(m0.sort_order, 0);
        assert_eq!(m1.sort_order, 1);
    }

    // create_adhoc with a `single` model snapshots exactly one member.
    #[tokio::test]
    async fn create_adhoc_single_snapshots_one_member() {
        let (svc, _repo) = adhoc_service().await;
        let req = CreateAdhocRunRequest {
            goal: "single-model run".to_string(),
            work_dir: None,
            model_range: ModelRange::Single { model: model_ref("prov_solo", "model-solo") },
            pinned_roles: vec![],
            autonomy: Some("autonomous".to_string()),
            max_parallel: None,
            lead_conv_id: None,
        };

        let run = svc.create_adhoc("u1", req).await.expect("create_adhoc");
        assert!(run.work_dir.is_none(), "no work_dir given");
        assert_eq!(run.autonomy, "autonomous", "explicit autonomy honored");

        let detail = svc.get_detail(&run.id).await.expect("detail");
        assert_eq!(detail.fleet_members.len(), 1);
        assert_eq!(detail.fleet_members[0].provider_id.as_deref(), Some("prov_solo"));
        assert_eq!(detail.fleet_members[0].model.as_deref(), Some("model-solo"));
    }

    // (b) create_adhoc rejects an empty goal (400).
    #[tokio::test]
    async fn create_adhoc_empty_goal_is_bad_request() {
        let (svc, _repo) = adhoc_service().await;
        let req = CreateAdhocRunRequest {
            goal: "   ".to_string(),
            work_dir: None,
            model_range: ModelRange::Single { model: model_ref("p", "m") },
            pinned_roles: vec![],
            autonomy: None,
            max_parallel: None,
            lead_conv_id: None,
        };
        let err = svc.create_adhoc("u1", req).await.expect_err("empty goal must reject");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");
    }

    // (b) create_adhoc rejects an empty `range` (no models → 400, no run persisted).
    #[tokio::test]
    async fn create_adhoc_empty_range_is_bad_request() {
        let (svc, _repo) = adhoc_service().await;
        let req = CreateAdhocRunRequest {
            goal: "needs a model".to_string(),
            work_dir: None,
            model_range: ModelRange::Range { models: vec![] },
            pinned_roles: vec![],
            autonomy: None,
            max_parallel: None,
            lead_conv_id: None,
        };
        let err = svc.create_adhoc("u1", req).await.expect_err("empty range must reject");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");
    }

    // (b) create_adhoc rejects an unexpanded `auto` range — Auto must be expanded
    // to a concrete `range` by the caps_orchestrator layer (Task 3) before
    // reaching the service. This is the contract Task 3 relies on.
    #[tokio::test]
    async fn create_adhoc_auto_range_is_bad_request() {
        let (svc, _repo) = adhoc_service().await;
        let req = CreateAdhocRunRequest {
            goal: "auto picks".to_string(),
            work_dir: None,
            model_range: ModelRange::Auto,
            pinned_roles: vec![],
            autonomy: None,
            max_parallel: None,
            lead_conv_id: None,
        };
        let err = svc.create_adhoc("u1", req).await.expect_err("auto must reject at service");
        assert!(matches!(err, AppError::BadRequest(_)), "got: {err:?}");
    }

    // (c) build_members_from_range unit coverage: Range → one member per ModelRef
    // (provider+model both Some, unique ids, sort_order = index); Single → one
    // member; Auto / empty Range → BadRequest (must be expanded/non-empty).
    #[test]
    fn build_members_from_range_unit() {
        // Range → two members.
        let members = build_members_from_range(&ModelRange::Range {
            models: vec![model_ref("p1", "m1"), model_ref("p2", "m2")],
        })
        .expect("range builds members");
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].provider_id.as_deref(), Some("p1"));
        assert_eq!(members[0].model.as_deref(), Some("m1"));
        assert_eq!(members[0].sort_order, 0);
        assert_eq!(members[1].sort_order, 1);
        assert_ne!(members[0].id, members[1].id);
        assert!(members[0].agent_id.is_empty(), "synthetic member has no agent");

        // Single → one member.
        let single = build_members_from_range(&ModelRange::Single {
            model: model_ref("ps", "ms"),
        })
        .expect("single builds a member");
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].provider_id.as_deref(), Some("ps"));

        // Empty Range → BadRequest.
        let empty = build_members_from_range(&ModelRange::Range { models: vec![] });
        assert!(matches!(empty, Err(AppError::BadRequest(_))), "got: {empty:?}");

        // Auto → BadRequest (caller must expand it).
        let auto = build_members_from_range(&ModelRange::Auto);
        assert!(matches!(auto, Err(AppError::BadRequest(_))), "got: {auto:?}");
    }
}

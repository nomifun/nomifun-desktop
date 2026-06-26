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
    Assignment, CreateRunRequest, FleetMember, PlannedDag, ReassignRequest, Run, RunDetail,
    RunTask, RunTaskDep, TaskProfile,
};
use nomifun_common::AppError;
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
                workspace_id: req.workspace_id,
                user_id: user_id.to_string(),
                goal: req.goal,
                fleet_snapshot,
                autonomy,
                max_parallel: req.max_parallel,
            })
            .await
            .map_err(OrchestratorError::from)?;

        let run = run_row_to_dto(row);
        // Status starts at `planning` (the repo INSERTs it); surface it on the bus.
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
    /// assignments, emit `planUpdated`, and flip the run to `running`.
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

        // Flip to running so the engine may pick it up.
        self.run_repo
            .update_run(
                run_id,
                UpdateRunParams {
                    status: Some("running".to_string()),
                    summary: None,
                    lead_conv_id: None,
                    total_tokens: None,
                },
            )
            .await
            .map_err(OrchestratorError::from)?;
        self.emitter.emit_run_status(run_id, "running");
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
        CapabilityProfile, CreateFleetRequest, CreateWorkspaceRequest, FleetMemberInput, PlannedTask,
        WebSocketMessage,
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
}

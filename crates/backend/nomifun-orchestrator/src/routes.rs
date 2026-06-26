//! Orchestration (智能编排) HTTP routes. Handlers do request/response
//! transformation only; all logic lives in [`FleetService`] / [`WorkspaceService`].
//! Auth is layered externally in nomifun-app (mirrors the webhook / requirement
//! / idmm routes), so it is safe to extract [`CurrentUser`] here — these routes
//! mount UNDER the auth middleware, not as public routes.
//!
//! IDs are application strings (`fleet_…` / `ows_…`), so the `{id}` path segment
//! is passed straight to the service without parsing.

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};

use nomifun_api_types::{
    ApiResponse, CreateFleetRequest, CreateRunRequest, CreateWorkspaceRequest, Fleet, OrchWorkspace,
    ReassignRequest, Run, RunDetail, UpdateFleetRequest, UpdateWorkspaceRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::state::OrchestratorRouterState;

pub fn orchestrator_routes(state: OrchestratorRouterState) -> Router {
    Router::new()
        .route(
            "/api/orchestrator/fleets",
            get(list_fleets).post(create_fleet),
        )
        .route(
            "/api/orchestrator/fleets/{id}",
            get(get_fleet).put(update_fleet).delete(delete_fleet),
        )
        .route(
            "/api/orchestrator/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/api/orchestrator/workspaces/{id}",
            get(get_workspace).put(update_workspace).delete(delete_workspace),
        )
        .route("/api/orchestrator/runs", post(create_run))
        .route(
            "/api/orchestrator/workspaces/{ws}/runs",
            get(list_workspace_runs),
        )
        .route("/api/orchestrator/runs/{id}", get(get_run))
        .route("/api/orchestrator/runs/{id}/cancel", post(cancel_run))
        .route(
            "/api/orchestrator/runs/{run_id}/tasks/{task_id}/assignment",
            put(reassign_task),
        )
        .with_state(state)
}

// ── Fleets ──────────────────────────────────────────────────────────────────

async fn list_fleets(
    State(state): State<OrchestratorRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<Fleet>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.fleet.list(&user.id).await?)))
}

async fn get_fleet(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Fleet>>, AppError> {
    Ok(Json(ApiResponse::ok(state.fleet.get(&id).await?)))
}

async fn create_fleet(
    State(state): State<OrchestratorRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateFleetRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<Fleet>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let created = state.fleet.create(&user.id, req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(created))))
}

async fn update_fleet(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<UpdateFleetRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Fleet>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.fleet.update(&id, req).await?)))
}

async fn delete_fleet(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.fleet.delete(&id).await?;
    Ok(Json(ApiResponse::success()))
}

// ── Workspaces ───────────────────────────────────────────────────────────────

async fn list_workspaces(
    State(state): State<OrchestratorRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<OrchWorkspace>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.workspace.list(&user.id).await?)))
}

async fn get_workspace(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<OrchWorkspace>>, AppError> {
    Ok(Json(ApiResponse::ok(state.workspace.get(&id).await?)))
}

async fn create_workspace(
    State(state): State<OrchestratorRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateWorkspaceRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<OrchWorkspace>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let created = state.workspace.create(&user.id, req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(created))))
}

async fn update_workspace(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<UpdateWorkspaceRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<OrchWorkspace>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.workspace.update(&id, req).await?)))
}

async fn delete_workspace(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.workspace.delete(&id).await?;
    Ok(Json(ApiResponse::success()))
}

// ── Runs ─────────────────────────────────────────────────────────────────────

/// Create a run, then plan it, then hand it to the engine. The three steps are
/// deliberately separate (Task 6 contract): `create` parks the run in `planning`,
/// `plan` decomposes the goal + flips it to `running`, and `engine.start` spawns
/// the (synchronous, fire-and-forget) execution loop. If planning fails, the
/// error is surfaced — the run already exists in `planning` and can be re-planned
/// later, but the caller learns the goal could not be decomposed.
async fn create_run(
    State(state): State<OrchestratorRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateRunRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<Run>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let run = state.run_service.create(&user.id, req).await?;
    state.run_service.plan(&run.id).await?;
    // `start` is synchronous (spawns the loop internally) — do not await.
    state.engine.start(run.id.clone());
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(run))))
}

async fn list_workspace_runs(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(ws): Path<String>,
) -> Result<Json<ApiResponse<Vec<Run>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.run_service.list(&ws).await?)))
}

async fn get_run(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<RunDetail>>, AppError> {
    Ok(Json(ApiResponse::ok(state.run_service.get_detail(&id).await?)))
}

/// Cancel a run: stop the engine loop (cooperative cancel + abort) then persist
/// the `cancelled` status. `stop` is synchronous; `cancel` persists + emits.
async fn cancel_run(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.engine.stop(&id);
    state.run_service.cancel(&id).await?;
    Ok(Json(ApiResponse::success()))
}

/// Override (or lock) the member assigned to a task. The reassign path: upserts
/// the task's assignment to the requested member with `source = "override"`,
/// `locked = req.locked.unwrap_or(true)`. The service validates the run/task
/// exist and the member is in the run's fleet snapshot.
async fn reassign_task(
    State(state): State<OrchestratorRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path((run_id, task_id)): Path<(String, String)>,
    body: Result<Json<ReassignRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.run_service.reassign(&run_id, &task_id, req).await?;
    Ok(Json(ApiResponse::success()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{RunEngine, RunEngineDeps};
    use crate::events::OrchestratorRunEventEmitter;
    use crate::plan::PlanProducer;
    use crate::run_service::RunService;
    use crate::service::{FleetService, WorkspaceService};
    use crate::worker::{MockWorkerRunner, WorkerRunner};
    use axum::body::Body;
    use axum::http::Request;
    use nomifun_api_types::{
        CreateFleetRequest, CreateRunRequest, CreateWorkspaceRequest, FleetMember, FleetMemberInput,
        PlannedDag, WebSocketMessage,
    };
    use nomifun_common::AppError;
    use nomifun_db::{
        SqliteFleetRepository, SqliteOrchWorkspaceRepository, SqliteRunRepository,
        init_database_memory,
    };
    use nomifun_realtime::EventBroadcaster;
    use std::sync::Arc;
    use tower::ServiceExt; // for `oneshot`

    /// No-op broadcaster: the router-builds test never asserts the event trail.
    struct NoopBroadcaster;
    impl EventBroadcaster for NoopBroadcaster {
        fn broadcast(&self, _event: WebSocketMessage<serde_json::Value>) {}
    }

    /// Minimal planner so a RunService can be constructed for the state.
    struct EmptyPlanProducer;
    #[async_trait::async_trait]
    impl PlanProducer for EmptyPlanProducer {
        async fn produce(
            &self,
            _goal: &str,
            _members: &[FleetMember],
        ) -> Result<PlannedDag, AppError> {
            Ok(PlannedDag { tasks: vec![] })
        }
    }

    async fn build_state() -> OrchestratorRouterState {
        build_state_with_run().await.0
    }

    /// Build a fully-wired state AND seed one run (workspace + single-member fleet
    /// + a run parked in `planning`). Returns the state and the seeded run id so a
    /// oneshot can hit `GET /api/orchestrator/runs/{id}` against a real row. We
    /// build the services over the same repos the state holds so the seeded data
    /// is visible to the handlers.
    async fn build_state_with_run() -> (OrchestratorRouterState, String) {
        let db = init_database_memory().await.expect("db init");
        let pool = db.pool().clone();
        let fleet_repo = Arc::new(SqliteFleetRepository::new(pool.clone()));
        let ws_repo = Arc::new(SqliteOrchWorkspaceRepository::new(pool.clone()));
        let run_repo = Arc::new(SqliteRunRepository::new(pool));
        let fleet = FleetService::new(fleet_repo.clone());
        let workspace = WorkspaceService::new(ws_repo.clone());
        let emitter = OrchestratorRunEventEmitter::new(Arc::new(NoopBroadcaster));
        let planner: Arc<dyn PlanProducer> = Arc::new(EmptyPlanProducer);
        let run_service = Arc::new(RunService::new(
            run_repo.clone(),
            fleet_repo,
            ws_repo.clone(),
            planner,
            emitter.clone(),
        ));
        let worker: Arc<dyn WorkerRunner> = Arc::new(MockWorkerRunner::with_text(1, "x"));
        let engine = RunEngine::new(Arc::new(RunEngineDeps::new(run_repo, worker, emitter, ws_repo)));

        // Seed: fleet (one member) → workspace → run (parked in `planning`).
        let seeded_fleet = fleet
            .create(
                "u1",
                CreateFleetRequest {
                    name: "smoke fleet".to_string(),
                    description: None,
                    max_parallel: None,
                    members: vec![FleetMemberInput {
                        agent_id: "agent_a".to_string(),
                        provider_id: None,
                        model: None,
                        role_hint: None,
                        capability_profile: None,
                        constraints: None,
                        sort_order: None,
                    }],
                },
            )
            .await
            .expect("seed fleet");
        let seeded_ws = workspace
            .create(
                "u1",
                CreateWorkspaceRequest {
                    name: "smoke ws".to_string(),
                    default_fleet_id: Some(seeded_fleet.id.clone()),
                    workspace_dir: None,
                },
            )
            .await
            .expect("seed workspace");
        let run = run_service
            .create(
                "u1",
                CreateRunRequest {
                    workspace_id: seeded_ws.id,
                    goal: "smoke goal".to_string(),
                    fleet_id: seeded_fleet.id,
                    autonomy: None,
                    max_parallel: None,
                },
            )
            .await
            .expect("seed run");

        let state = OrchestratorRouterState::new(fleet, workspace, run_service, engine);
        (state, run.id)
    }

    /// The router builds without panicking.
    #[tokio::test]
    async fn router_builds() {
        let state = build_state().await;
        let _router = orchestrator_routes(state);
    }

    /// `GET /api/orchestrator/fleets` returns 200 once a `CurrentUser` extension
    /// is present. We inject it via a layer here, exactly as the auth middleware
    /// does in nomifun-app — so the handler's `Extension<CurrentUser>` requirement
    /// is exercised, not bypassed. (The full auth-wired path is covered by Task 8's
    /// app-level integration test.)
    #[tokio::test]
    async fn list_fleets_returns_ok_with_user() {
        let state = build_state().await;
        let app = orchestrator_routes(state).layer(axum::Extension(CurrentUser {
            id: "u1".to_string(),
            username: "tester".to_string(),
        }));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/orchestrator/fleets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Without the `CurrentUser` extension the handler cannot run — axum returns
    /// 500 (missing required extension). This guards that we did NOT weaken the
    /// handler by dropping the `Extension<CurrentUser>` requirement.
    #[tokio::test]
    async fn list_fleets_without_user_is_not_ok() {
        let state = build_state().await;
        let app = orchestrator_routes(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/orchestrator/fleets")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_ne!(resp.status(), StatusCode::OK);
    }

    /// `GET /api/orchestrator/runs/{id}` returns 200 for a seeded run once a
    /// `CurrentUser` extension is present. This exercises the new run route end to
    /// end through the router (path extraction → RunService::get_detail → 200) and
    /// confirms the route is actually mounted — before the route existed axum would
    /// have routed this to a 404. (Full HTTP behavior is covered by Task 9.)
    #[tokio::test]
    async fn get_run_returns_ok_with_user() {
        let (state, run_id) = build_state_with_run().await;
        let app = orchestrator_routes(state).layer(axum::Extension(CurrentUser {
            id: "u1".to_string(),
            username: "tester".to_string(),
        }));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/orchestrator/runs/{run_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// The `GET /api/orchestrator/runs/{id}` route still requires the
    /// `CurrentUser` extension — without it the handler cannot run (axum returns a
    /// non-200). Guards that the run route was not wired without auth.
    #[tokio::test]
    async fn get_run_without_user_is_not_ok() {
        let (state, run_id) = build_state_with_run().await;
        let app = orchestrator_routes(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/orchestrator/runs/{run_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_ne!(resp.status(), StatusCode::OK);
    }
}

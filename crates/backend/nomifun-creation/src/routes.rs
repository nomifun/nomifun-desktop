//! Canonical `/api/creative-studio/tasks` handlers. The retired unowned
//! `/api/creation/tasks` surface is deliberately not mounted.

use axum::Router;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::Value;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::dto::{
    CreativeCreationTask, CreativeCreationTaskPage, CreativeCreationTaskRetireResult,
};
#[cfg(test)]
use crate::dto::CreationTask;
use crate::service::{CreativeTaskOwner, NewCreationTask};
use crate::state::CreationRouterState;
use crate::types::{CreationInput, CreationInputKind, StandaloneWorkbenchKind};

pub fn creation_routes(state: CreationRouterState) -> Router {
    Router::new()
        .route(
            "/api/creative-studio/tasks",
            get(list_standalone_workbench_tasks).post(create_creative_task),
        )
        .route(
            "/api/creative-studio/tasks/{creation_task_id}",
            get(get_creative_task),
        )
        .route(
            "/api/creative-studio/tasks/retire",
            post(retire_standalone_workbench_tasks),
        )
        .route(
            "/api/creative-studio/tasks/{creation_task_id}/cancel",
            post(cancel_creative_task),
        )
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRef {
    asset_id: String,
    kind: CreationInputKind,
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "reference".to_string()
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CreativeTaskOwnerRequest {
    CanvasNode {
        canvas_id: String,
        node_id: String,
    },
    StandaloneWorkbench {
        workbench_kind: StandaloneWorkbenchKind,
    },
    TemplateStep {
        template_id: String,
        template_run_id: String,
        template_step_id: String,
    },
}

impl From<CreativeTaskOwnerRequest> for CreativeTaskOwner {
    fn from(owner: CreativeTaskOwnerRequest) -> Self {
        match owner {
            CreativeTaskOwnerRequest::CanvasNode {
                canvas_id,
                node_id,
            } => Self::CanvasNode {
                canvas_id,
                node_id,
            },
            CreativeTaskOwnerRequest::StandaloneWorkbench {
                workbench_kind,
            } => Self::StandaloneWorkbench {
                workbench_kind,
            },
            CreativeTaskOwnerRequest::TemplateStep {
                template_id,
                template_run_id,
                template_step_id,
            } => Self::TemplateStep {
                template_id,
                template_run_id,
                template_step_id,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCreativeTaskRequest {
    owner: CreativeTaskOwnerRequest,
    provider_id: String,
    model: String,
    capability: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    inputs: Vec<InputRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListStandaloneWorkbenchTasksQuery {
    workbench_kind: StandaloneWorkbenchKind,
    limit: Option<usize>,
    cursor: Option<String>,
    active_only: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetireStandaloneWorkbenchTasksRequest {
    workbench_kind: StandaloneWorkbenchKind,
    task_ids: Vec<String>,
}

fn required_idempotency_key(headers: &HeaderMap) -> Result<String, AppError> {
    let mut values = headers.get_all("idempotency-key").iter();
    let value = values.next().ok_or_else(|| {
        AppError::BadRequest("Creative Studio task creation requires Idempotency-Key".into())
    })?;
    if values.next().is_some() {
        return Err(AppError::BadRequest(
            "Idempotency-Key must be sent exactly once".into(),
        ));
    }
    value
        .to_str()
        .map(str::to_owned)
        .map_err(|_| AppError::BadRequest("Idempotency-Key must be visible ASCII".into()))
}

async fn create_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    headers: HeaderMap,
    body: Result<Json<CreateCreativeTaskRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let idempotency_key = required_idempotency_key(&headers)?;
    let task = state
        .service
        .create_creative_task(
            req.owner.into(),
            idempotency_key,
            NewCreationTask {
                provider_id: req.provider_id,
                model: req.model,
                capability: req.capability,
                params: req.params,
                inputs: req
                    .inputs
                    .into_iter()
                    .map(|input| CreationInput {
                        asset_id: input.asset_id,
                        kind: input.kind,
                        role: input.role,
                    })
                    .collect(),
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeCreationTask::try_from(task)?)),
    ))
}

async fn list_standalone_workbench_tasks(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    query: Result<Query<ListStandaloneWorkbenchTasksQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<CreativeCreationTaskPage>>, AppError> {
    let Query(query) = query.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let page = state
        .service
        .list_standalone_workbench_tasks(
            query.workbench_kind,
            query.active_only.unwrap_or(false),
            query.limit,
            query.cursor.as_deref(),
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeCreationTaskPage::try_new(
        page.items,
        page.next_cursor,
    )?)))
}

async fn retire_standalone_workbench_tasks(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<RetireStandaloneWorkbenchTasksRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<CreativeCreationTaskRetireResult>>, AppError> {
    let Json(request) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let retired_task_ids = state
        .service
        .retire_standalone_workbench_tasks(
            request.workbench_kind,
            &request.task_ids,
        )
        .await?;
    Ok(Json(ApiResponse::ok(CreativeCreationTaskRetireResult {
        retired_task_ids,
    })))
}

async fn get_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeCreationTask>>, AppError> {
    let task = state.service.get_task(&creation_task_id).await?;
    Ok(Json(ApiResponse::ok(CreativeCreationTask::try_from(task)?)))
}

async fn cancel_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeCreationTask>>, AppError> {
    let task = state.service.cancel_task(&creation_task_id).await?;
    Ok(Json(ApiResponse::ok(CreativeCreationTask::try_from(task)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    #[test]
    fn creative_task_request_accepts_only_the_tagged_owner_contract() {
        let canvas = serde_json::from_value::<CreateCreativeTaskRequest>(json!({
            "owner": {
                "kind": "canvas_node",
                "canvas_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "node_id": "0190f5fe-7c00-7a00-8000-000000000002"
            },
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
            "model": "image-model-v1",
            "capability": "t2i"
        }))
        .unwrap();
        assert!(matches!(
            canvas.owner,
            CreativeTaskOwnerRequest::CanvasNode { .. }
        ));

        let parsed = serde_json::from_value::<CreateCreativeTaskRequest>(json!({
            "owner": {
                "kind": "template_step",
                "template_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "template_run_id": "0190f5fe-7c00-7a00-8000-000000000002",
                "template_step_id": "0190f5fe-7c00-7a00-8000-000000000003"
            },
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
            "model": "image-model-v1",
            "capability": "t2i",
            "params": {"prompt": "Aurora"},
            "inputs": []
        }))
        .unwrap();
        assert!(matches!(
            &parsed.owner,
            CreativeTaskOwnerRequest::TemplateStep { .. }
        ));
        let owner = CreativeTaskOwner::from(parsed.owner);
        let owner_wire = serde_json::to_value(owner).unwrap();
        assert_eq!(owner_wire["kind"], "template_step");
        assert_eq!(
            owner_wire["template_id"],
            "0190f5fe-7c00-7a00-8000-000000000001"
        );
        assert_eq!(
            owner_wire["template_run_id"],
            "0190f5fe-7c00-7a00-8000-000000000002"
        );
        assert_eq!(
            owner_wire["template_step_id"],
            "0190f5fe-7c00-7a00-8000-000000000003"
        );

        let standalone = serde_json::from_value::<CreateCreativeTaskRequest>(json!({
            "owner": {
                "kind": "standalone_workbench",
                "workbench_kind": "video"
            },
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
            "model": "video-model-v1",
            "capability": "i2v",
            "params": {"prompt": "Aurora"},
            "inputs": [{
                "asset_id": "0190f5fe-7c00-7a00-8000-000000000006",
                "kind": "image",
                "role": "first_frame"
            }]
        }))
        .unwrap();
        assert!(matches!(
            standalone.owner,
            CreativeTaskOwnerRequest::StandaloneWorkbench {
                workbench_kind: StandaloneWorkbenchKind::Video,
                ..
            }
        ));

        for invalid in [
            json!({
                "canvas_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "node_id": "0190f5fe-7c00-7a00-8000-000000000002",
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
                "model": "image-model-v1",
                "capability": "t2i"
            }),
            json!({
                "owner": {
                    "kind": "canvas_node",
                    "project_id": "0190f5fe-7c00-7a00-8000-000000000001",
                    "node_id": "0190f5fe-7c00-7a00-8000-000000000002",
                    "canvas_id": "0190f5fe-7c00-7a00-8000-000000000005"
                },
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
                "model": "image-model-v1",
                "capability": "t2i"
            }),
            json!({
                "owner": {
                    "kind": "standalone_workbench",
                    "project_id": "0190f5fe-7c00-7a00-8000-000000000001",
                    "workbench_kind": "video"
                },
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
                "model": "video-model-v1",
                "capability": "i2v",
                "inputs": [{
                    "asset_id": "0190f5fe-7c00-7a00-8000-000000000006",
                    "role": "first_frame"
                }]
            }),
        ] {
            assert!(serde_json::from_value::<CreateCreativeTaskRequest>(invalid).is_err());
        }
    }

    #[test]
    fn creative_task_idempotency_key_is_required_exactly_once() {
        let mut headers = HeaderMap::new();
        assert!(required_idempotency_key(&headers).is_err());

        headers.append(
            "idempotency-key",
            HeaderValue::from_static("0190f5fe-7c00-7a00-8000-000000000001"),
        );
        assert_eq!(
            required_idempotency_key(&headers).unwrap(),
            "0190f5fe-7c00-7a00-8000-000000000001"
        );

        headers.append(
            "idempotency-key",
            HeaderValue::from_static("0190f5fe-7c00-7a00-8000-000000000002"),
        );
        assert!(required_idempotency_key(&headers).is_err());
    }

    #[test]
    fn standalone_list_query_is_exact_and_rejects_unknown_or_duplicate_fields() {
        let uri = "/api/creative-studio/tasks?workbench_kind=video&limit=30&cursor=1%3A0190f5fe-7c00-7a00-8000-000000000002&active_only=true"
            .parse()
            .unwrap();
        let Query(query) = Query::<ListStandaloneWorkbenchTasksQuery>::try_from_uri(&uri).unwrap();
        assert_eq!(query.workbench_kind, StandaloneWorkbenchKind::Video);
        assert_eq!(query.limit, Some(30));
        assert_eq!(query.active_only, Some(true));
        assert!(query.cursor.as_deref().unwrap().starts_with("1:"));

        for invalid in [
            "/api/creative-studio/tasks?workbench_kind=video&unknown=1",
            "/api/creative-studio/tasks?workbench_kind=video&workbench_kind=image",
            "/api/creative-studio/tasks?workbench_kind=canvas",
            "/api/creative-studio/tasks?workbench_kind=video&active_only=yes",
            "/api/creative-studio/tasks?workbench_kind=video&active_only=true&active_only=false",
        ] {
            let uri = invalid.parse().unwrap();
            assert!(
                Query::<ListStandaloneWorkbenchTasksQuery>::try_from_uri(&uri).is_err(),
                "query must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn standalone_retire_body_is_flat_exact_and_typed() {
        let request = serde_json::from_value::<RetireStandaloneWorkbenchTasksRequest>(json!({
            "workbench_kind": "image",
            "task_ids": ["0190f5fe-7c00-7a00-8000-000000000002"]
        }))
        .unwrap();
        assert_eq!(request.workbench_kind, StandaloneWorkbenchKind::Image);
        assert_eq!(request.task_ids.len(), 1);
        for invalid in [
            json!({
                "workbench_kind": "image",
                "task_ids": [],
                "owner": {"kind": "standalone_workbench"}
            }),
            json!({
                "workbench_kind": "canvas",
                "task_ids": []
            }),
        ] {
            assert!(serde_json::from_value::<RetireStandaloneWorkbenchTasksRequest>(invalid).is_err());
        }
    }

    #[test]
    fn creative_task_surface_rejects_invalid_owner_rows() {
        let invalid = CreationTask {
            creation_task_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            canvas_id: None,
            workbench_kind: None,
            template_id: None,
            template_run_id: None,
            template_step_id: None,
            node_id: None,
            provider_id: "0190f5fe-7c00-7a00-8000-000000000004".into(),
            model: "image-model-v1".into(),
            capability: "t2i".into(),
            params: json!({}),
            inputs: Some(Vec::new()),
            status: "queued".into(),
            error: None,
            result_asset_ids: Vec::new(),
            attempt: 0,
            submitted_at: 1,
            started_at: None,
            finished_at: None,
            deleted_at: None,
        };
        assert!(CreativeCreationTask::try_from(invalid).is_err());
    }
}

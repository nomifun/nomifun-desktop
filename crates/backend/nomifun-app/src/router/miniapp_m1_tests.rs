use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::{Extension, Router};
use http_body_util::BodyExt;
use nomifun_api_types::{
    ApiResponse, BuildMiniAppRequest, CreateMiniAppProjectRequest,
    DurableOperationKindDto, DurableOperationOwnerDto,
    DurableOperationStateDto, DurableOperationSummaryDto, ErrorResponse,
    MiniAppKindDto, MiniAppLibraryResponseDto, MiniAppWorkshopDto,
};
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, UserId};
use nomifun_db::{
    DbError, IMiniAppM1Repository, MiniAppM1ManagedSourceLineage,
    SqliteMiniAppM1Repository, StartMiniAppM1BuildOperationParams,
    init_database_memory, installation_owner_id,
};
use nomifun_miniapp_platform::MiniAppM1ApplicationService;
use serde::de::DeserializeOwned;
use serde_json::json;
use tower::ServiceExt;

use super::{
    MiniAppM1RouterState, application_error, miniapp_m1_read_routes,
    miniapp_m1_write_routes,
};

#[tokio::test]
async fn split_routes_preserve_owner_scope_and_api_envelopes() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let state = MiniAppM1RouterState::new(Arc::new(
        MiniAppM1ApplicationService::new_with_root(
            repository,
            store_root.path(),
        )
        .unwrap(),
    ));
    let owner = current_user(&owner_id, "owner");

    let read =
        miniapp_m1_read_routes(state.clone()).layer(Extension(owner.clone()));
    let write =
        miniapp_m1_write_routes(state.clone()).layer(Extension(owner));

    let response = send(&read, Method::POST, "/api/miniapps/projects", None)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send(&write, Method::GET, "/api/miniapps", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        &write,
        Method::POST,
        "/api/miniapps/projects",
        Some(json!({
            "expected_library_revision": 0,
            "display_name": "Route M1",
            "description": "owner-scoped route",
            "kind": "ui_only"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let created: MiniAppWorkshopDto = response_data(response).await;
    assert_eq!(created.miniapp.display_name, "Route M1");
    assert_eq!(created.miniapp.kind, MiniAppKindDto::UiOnly);

    let build_path = format!(
        "/api/miniapps/{}/build",
        created.miniapp.miniapp_id
    );
    let build_body = build_request(&created);
    let response = send(
        &read,
        Method::POST,
        &build_path,
        Some(serde_json::to_value(&build_body).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let operation_id = "0190f5fe-7c00-7000-8000-000000000991";
    let cancel_path = format!(
        "/api/miniapps/{}/operations/{operation_id}/cancel",
        created.miniapp.miniapp_id
    );
    let response = send(
        &read,
        Method::POST,
        &cancel_path,
        Some(json!({ "expected_operation_revision": 4 })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(&read, Method::GET, "/api/miniapps", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let library: MiniAppLibraryResponseDto = response_data(response).await;
    assert_eq!(library.library_revision, 1);
    assert_eq!(library.miniapps.len(), 1);
    assert_eq!(
        library.miniapps[0].miniapp_id,
        created.miniapp.miniapp_id
    );

    let workshop_path = format!(
        "/api/miniapps/{}/workshop",
        created.miniapp.miniapp_id
    );
    let response = send(&read, Method::GET, &workshop_path, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let workshop: MiniAppWorkshopDto = response_data(response).await;
    assert_eq!(workshop, created);

    let other = CurrentUser {
        id: UserId::new(),
        username: "other".to_owned(),
    };
    nomifun_db::sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(other.id.as_str())
    .bind(other.id.as_str())
    .execute(database.pool())
    .await
    .unwrap();
    let other_read =
        miniapp_m1_read_routes(state).layer(Extension(other));
    let response =
        send(&other_read, Method::GET, "/api/miniapps", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let library: MiniAppLibraryResponseDto = response_data(response).await;
    assert_eq!(library.library_revision, 0);
    assert!(library.miniapps.is_empty());

    let response =
        send(&other_read, Method::GET, &workshop_path, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, "NOT_FOUND");
}

#[tokio::test]
async fn build_and_cancel_routes_use_real_application_state() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let repository = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        MiniAppM1ApplicationService::new_with_root(
            repository.clone(),
            store_root.path(),
        )
        .unwrap(),
    );
    let workshop = application
        .create(
            &owner_id,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "Build Route".to_owned(),
                description: Some("exact request forwarding".to_owned()),
                kind: MiniAppKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let miniapp_id = workshop.miniapp.miniapp_id.clone();
    let state = MiniAppM1RouterState::new(application);
    let write = miniapp_m1_write_routes(state)
        .layer(Extension(current_user(&owner_id, "owner")));

    let build_request = build_request(&workshop);
    let build_path = format!("/api/miniapps/{miniapp_id}/build");
    let response = send(
        &write,
        Method::POST,
        &build_path,
        Some(serde_json::to_value(&build_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let built: MiniAppWorkshopDto = response_data(response).await;
    let ready = built.ready.as_ref().expect("Build must commit Ready");
    assert_eq!(ready.project_build_generation, workshop.build_generation);
    assert!(ready.created_at_ms > 0);
    assert_eq!(
        built.miniapp.releases.ready.as_ref(),
        Some(&ready.release)
    );
    let completed = repository
        .list_build_operations(&owner_id, &miniapp_id)
        .await
        .unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].state, "succeeded");
    assert_eq!(completed[0].owner_kind, "miniapp");
    assert_eq!(completed[0].owner_id, miniapp_id);

    let mismatched_request = BuildMiniAppRequest {
        miniapp_id: "0190f5fe-7c00-7000-8000-000000000993".to_owned(),
        ..build_request.clone()
    };
    let response = send(
        &write,
        Method::POST,
        &build_path,
        Some(serde_json::to_value(&mismatched_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, "BAD_REQUEST");
    assert_eq!(
        repository
            .list_build_operations(&owner_id, &miniapp_id)
            .await
            .unwrap()
            .len(),
        1,
        "path/body identity mismatch must fail before starting an operation"
    );

    let snapshot = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .expect("built MiniApp");
    let operation_id = "0190f5fe-7c00-7000-8000-000000000992";
    let started_at_ms = nomifun_common::now_ms()
        .max(snapshot.product.created_at)
        .max(snapshot.project.updated_at)
        .max(1);
    repository
        .start_build_operation(&StartMiniAppM1BuildOperationParams {
            owner_user_id: owner_id.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: snapshot.project.project_id.clone(),
            operation_id: operation_id.to_owned(),
            expected_project_revision: snapshot.project.project_revision,
            expected_source: MiniAppM1ManagedSourceLineage {
                managed_source_path: snapshot
                    .project
                    .managed_source_path
                    .clone()
                    .expect("managed source path"),
                source_head_digest: snapshot
                    .project
                    .source_head_digest
                    .clone()
                    .expect("source head digest"),
                dependency_lock_digest: snapshot
                    .project
                    .dependency_lock_digest
                    .clone()
                    .expect("dependency lock digest"),
                build_profile_version: snapshot
                    .project
                    .build_profile_version
                    .clone()
                    .expect("build profile version"),
                build_generation: snapshot.project.build_generation,
            },
            bounded_log_tail: vec!["Build started for route cancellation".to_owned()],
            started_at_ms,
        })
        .await
        .unwrap();
    let cancel_path = format!(
        "/api/miniapps/{miniapp_id}/operations/{operation_id}/cancel"
    );
    let response = send(
        &write,
        Method::POST,
        &cancel_path,
        Some(json!({ "expected_operation_revision": 1 })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let canceled: DurableOperationSummaryDto = response_data(response).await;
    assert_eq!(canceled.operation_id, operation_id);
    assert_eq!(canceled.operation_revision, 2);
    assert_eq!(canceled.kind, DurableOperationKindDto::Build);
    assert_eq!(
        canceled.owner,
        DurableOperationOwnerDto::Miniapp {
            miniapp_id: miniapp_id.clone(),
        }
    );
    assert_eq!(canceled.state, DurableOperationStateDto::Canceled);
    assert!(!canceled.cancelable);
    assert!(canceled.completed_at_ms.is_some());
    let persisted = repository
        .get_build_operation(&owner_id, &miniapp_id, operation_id)
        .await
        .unwrap()
        .expect("canceled operation");
    assert_eq!(persisted.state, "canceled");

    let response = send(
        &write,
        Method::POST,
        &cancel_path,
        Some(json!({ "expected_operation_revision": 1 })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, "CONFLICT");
}

#[test]
fn application_errors_map_to_app_error_semantics() {
    let invalid = application_error(
        nomifun_miniapp_platform::MiniAppM1ApplicationError::Invalid(
            "bad revision".to_owned(),
        ),
    );
    assert!(matches!(invalid, AppError::BadRequest(_)));

    let not_found = application_error(
        nomifun_miniapp_platform::MiniAppM1ApplicationError::NotFound,
    );
    assert!(matches!(not_found, AppError::NotFound(_)));

    let conflict = application_error(
        nomifun_miniapp_platform::MiniAppM1ApplicationError::Database(
            DbError::Conflict("stale library revision".to_owned()),
        ),
    );
    assert!(matches!(conflict, AppError::Conflict(_)));
}

fn build_request(workshop: &MiniAppWorkshopDto) -> BuildMiniAppRequest {
    BuildMiniAppRequest {
        miniapp_id: workshop.miniapp.miniapp_id.clone(),
        expected_product_revision: workshop.miniapp.product_revision,
        project_id: workshop.project_id.clone(),
        expected_project_revision: workshop.project_revision,
        expected_build_generation: workshop.build_generation,
        expected_source_snapshot_digest: workshop
            .source_snapshot_digest
            .clone()
            .expect("editable source snapshot"),
        expected_dependency_lock_digest: workshop
            .dependency_lock_digest
            .clone()
            .expect("dependency lock"),
    }
}

fn current_user(id: &str, username: &str) -> CurrentUser {
    CurrentUser {
        id: UserId::parse(id).unwrap(),
        username: username.to_owned(),
    }
}

async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn response_data<T>(response: axum::response::Response) -> T
where
    T: DeserializeOwned,
{
    let response: ApiResponse<T> = response_json(response).await;
    assert!(response.success);
    response.data.expect("success response must contain data")
}

async fn response_json<T>(response: axum::response::Response) -> T
where
    T: DeserializeOwned,
{
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&body).unwrap()
}

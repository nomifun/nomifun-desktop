use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::{Extension, Router};
use http_body_util::BodyExt;
use nomifun_api_types::{
    ApiResponse, ErrorResponse, MiniAppKindDto, MiniAppLibraryResponseDto,
    MiniAppWorkshopDto,
};
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, UserId};
use nomifun_db::{
    DbError, IMiniAppM1Repository, SqliteMiniAppM1Repository,
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
    let state = MiniAppM1RouterState::new(Arc::new(
        MiniAppM1ApplicationService::new(repository),
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

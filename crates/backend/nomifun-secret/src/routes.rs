//! `/api/browser-secrets/*` route handlers (P3-X2).
//!
//! Global browser-use credential CRUD. Handlers do request/response transformation
//! only; all logic lives in [`SecretService`](crate::service::SecretService). Auth is
//! layered externally in nomifun-app (mirrors the knowledge / webhook routes).
//!
//! **安全红线**：the secret *value* is write-only — accepted on `POST` (register) and
//! then encrypted into the shared vault. **No endpoint ever returns it.** `GET`
//! (list) returns only name + bound origins (the [`SecretListItem`] metadata).

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};

use nomifun_api_types::{ApiResponse, RegisterSecretRequest, SecretListItem};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::state::SecretRouterState;

pub fn secret_routes(state: SecretRouterState) -> Router {
    Router::new()
        // List (metadata only — NEVER the value) + register globally.
        .route("/api/browser-secrets", get(list_secrets).post(register_secret))
        // Remove a single secret by name.
        .route("/api/browser-secrets/{name}", delete(remove_secret))
        .with_state(state)
}

async fn list_secrets(
    State(state): State<SecretRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<SecretListItem>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list())))
}

async fn register_secret(
    State(state): State<SecretRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<RegisterSecretRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<()>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .service
        .register(&req.name, &req.value, req.allowed_origins)?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(()))))
}

async fn remove_secret(
    State(state): State<SecretRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(name): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.remove(&name)?;
    Ok(Json(ApiResponse::ok(())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KEY_SIZE;
    use crate::service::SecretService;
    use axum::body::Body;
    use axum::http::Request;
    use nomifun_auth::CurrentUser;
    use tower::ServiceExt;

    fn router_with_user(dir: &std::path::Path) -> Router {
        let svc = SecretService::new(dir.to_path_buf(), [0x42; KEY_SIZE]);
        secret_routes(SecretRouterState::new(svc))
            // Inject a CurrentUser directly (the real auth middleware is layered in
            // nomifun-app; tests attach the extension so the Extension extractor resolves).
            .layer(axum::Extension(CurrentUser {
                id: nomifun_common::UserId::new(),
                username: "tester".into(),
            }))
    }

    async fn body_string(resp: axum::response::Response) -> String {
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn register_then_list_never_returns_value() {
        let dir = tempfile::tempdir().unwrap();
        let app = router_with_user(dir.path());

        // Register.
        let reg = Request::builder()
            .method("POST")
            .uri("/api/browser-secrets")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"github","value":"ghp_supersecret","allowed_origins":["github.com"]}"#,
            ))
            .unwrap();
        let resp = app.clone().oneshot(reg).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // List — must carry name + origins, NEVER the value.
        let list = Request::builder()
            .method("GET")
            .uri("/api/browser-secrets")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(list).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let text = body_string(resp).await;
        assert!(text.contains("github"), "list should carry the name: {text}");
        assert!(text.contains("github.com"), "list should carry the bound origin: {text}");
        assert!(!text.contains("ghp_supersecret"), "list MUST NOT leak the value: {text}");
    }

    #[tokio::test]
    async fn register_rejects_bad_origin_with_400() {
        let dir = tempfile::tempdir().unwrap();
        let app = router_with_user(dir.path());
        let reg = Request::builder()
            .method("POST")
            .uri("/api/browser-secrets")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"n","value":"v","allowed_origins":["co.uk"]}"#))
            .unwrap();
        let resp = app.oneshot(reg).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn remove_then_list_empty() {
        let dir = tempfile::tempdir().unwrap();
        let app = router_with_user(dir.path());
        let reg = Request::builder()
            .method("POST")
            .uri("/api/browser-secrets")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"pw","value":"v","allowed_origins":["x.com"]}"#))
            .unwrap();
        app.clone().oneshot(reg).await.unwrap();

        let del = Request::builder()
            .method("DELETE")
            .uri("/api/browser-secrets/pw")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(del).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let list = Request::builder()
            .method("GET")
            .uri("/api/browser-secrets")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(list).await.unwrap();
        let text = body_string(resp).await;
        assert!(!text.contains("\"pw\""), "removed secret must be gone: {text}");
    }
}

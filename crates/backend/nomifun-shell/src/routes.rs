use axum::extract::{Multipart, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};

use nomifun_api_types::{
    ApiResponse, CheckToolInstalledRequest, CheckToolInstalledResponse, OpenExternalRequest, OpenFileRequest,
    OpenFolderWithRequest, ShowItemInFolderRequest, SpeechToTextConfig,
};
use nomifun_common::AppError;

use crate::error::SttError;
use crate::state::ShellRouterState;

pub fn shell_routes(state: ShellRouterState) -> Router {
    Router::new()
        .route("/api/shell/open-file", post(open_file))
        .route("/api/shell/show-item-in-folder", post(show_item_in_folder))
        .route("/api/shell/open-external", post(open_external))
        .route("/api/shell/check-tool-installed", post(check_tool_installed))
        .route("/api/shell/open-folder-with", post(open_folder_with))
        .route("/api/stt", post(speech_to_text))
        .with_state(state)
}

async fn open_file(
    State(state): State<ShellRouterState>,
    body: Result<Json<OpenFileRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.shell_service.open_file(&req.file_path).await?;
    Ok(Json(ApiResponse::success()))
}

async fn show_item_in_folder(
    State(state): State<ShellRouterState>,
    body: Result<Json<ShowItemInFolderRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.shell_service.show_item_in_folder(&req.file_path).await?;
    Ok(Json(ApiResponse::success()))
}

async fn open_external(
    State(state): State<ShellRouterState>,
    body: Result<Json<OpenExternalRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.shell_service.open_external(&req.url).await?;
    Ok(Json(ApiResponse::success()))
}

async fn check_tool_installed(
    State(state): State<ShellRouterState>,
    body: Result<Json<CheckToolInstalledRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ApiResponse<CheckToolInstalledResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let installed = state.shell_service.check_tool_installed(req.tool).await;
    Ok(Json(ApiResponse::ok(CheckToolInstalledResponse { installed })))
}

async fn open_folder_with(
    State(state): State<ShellRouterState>,
    body: Result<Json<OpenFolderWithRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.shell_service.open_folder_with(&req.folder_path, req.tool).await?;
    Ok(Json(ApiResponse::success()))
}

struct SttMultipartFields {
    file_data: Vec<u8>,
    file_name: String,
    mime_type: String,
    language_hint: Option<String>,
}

async fn extract_stt_multipart(mut multipart: Multipart) -> Result<SttMultipartFields, AppError> {
    let mut file_data: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut language_hint: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_owned();
        match name.as_str() {
            "file" => {
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("failed to read file: {e}")))?
                        .to_vec(),
                );
            }
            "fileName" => {
                file_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("failed to read fileName: {e}")))?,
                );
            }
            "mimeType" => {
                mime_type = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("failed to read mimeType: {e}")))?,
                );
            }
            "languageHint" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("failed to read languageHint: {e}")))?;
                if !text.is_empty() {
                    language_hint = Some(text);
                }
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or_else(|| AppError::BadRequest("missing 'file' field".to_owned()))?;
    let file_name = file_name.ok_or_else(|| AppError::BadRequest("missing 'fileName' field".to_owned()))?;
    let mime_type = mime_type.ok_or_else(|| AppError::BadRequest("missing 'mimeType' field".to_owned()))?;

    Ok(SttMultipartFields {
        file_data,
        file_name,
        mime_type,
        language_hint,
    })
}

async fn speech_to_text(
    State(state): State<ShellRouterState>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    let fields = extract_stt_multipart(multipart).await.map_err(|e| {
        let status = e.status_code();
        let body = serde_json::json!({
            "success": false,
            "error": e.to_string(),
            "code": e.error_code(),
        });
        (status, Json(body))
    })?;

    let prefs = state
        .client_pref_service
        .get_preferences(Some(&["speechToText"]))
        .await
        .map_err(|e| {
            let status = e.status_code();
            let body = serde_json::json!({
                "success": false,
                "error": e.to_string(),
                "code": e.error_code(),
            });
            (status, Json(body))
        })?;

    let config: SpeechToTextConfig = prefs
        .get("speechToText")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(SpeechToTextConfig {
            enabled: false,
            provider: nomifun_api_types::SpeechToTextProvider::Openai,
            auto_send: None,
            openai: None,
            deepgram: None,
        });

    let result = state
        .stt_service
        .transcribe(
            fields.file_data,
            &fields.file_name,
            &fields.mime_type,
            fields.language_hint.as_deref(),
            &config,
        )
        .await
        .map_err(|e| stt_error_response(&e))?;

    let body = serde_json::json!({
        "success": true,
        "data": result,
    });
    Ok((StatusCode::OK, Json(body)))
}

fn stt_error_response(err: &SttError) -> (StatusCode, Json<serde_json::Value>) {
    let status = StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = serde_json::json!({
        "success": false,
        "error": err.to_string(),
        "code": err.error_code(),
    });
    (status, Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn make_state() -> ShellRouterState {
        use crate::opener::NoopSystemOpener;
        use crate::shell::ShellService;
        use crate::stt::SttService;

        let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        let repo = Arc::new(nomifun_db::SqliteClientPreferenceRepository::new(pool));
        let client_pref_service = nomifun_system::ClientPrefService::new(repo);

        ShellRouterState {
            shell_service: Arc::new(ShellService::new(Arc::new(NoopSystemOpener))),
            stt_service: Arc::new(SttService::new(reqwest::Client::new())),
            client_pref_service,
        }
    }

    fn make_router() -> Router {
        shell_routes(make_state())
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn open_file_missing_body_returns_400() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/open-file")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn open_file_nonexistent_returns_400() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/open-file")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"filePath":"/nonexistent/file.txt"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = body_json(resp).await;
        assert_eq!(json["success"], false);
    }

    #[tokio::test]
    async fn open_external_invalid_url_returns_400() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/open-external")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"url":"; rm -rf /"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn open_external_file_scheme_returns_400() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/open-external")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"url":"file:///etc/passwd"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn check_tool_terminal_returns_installed_true() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/check-tool-installed")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"tool":"terminal"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["success"], true);
        assert_eq!(json["data"]["installed"], true);
    }

    #[tokio::test]
    async fn check_tool_explorer_returns_installed_true() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/check-tool-installed")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"tool":"explorer"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["success"], true);
        assert_eq!(json["data"]["installed"], true);
    }

    #[tokio::test]
    async fn open_folder_with_nonexistent_dir_returns_400() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/open-folder-with")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"folderPath":"/nonexistent/dir","tool":"explorer"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn show_item_in_folder_nonexistent_returns_400() {
        let app = make_router();
        let req = Request::builder()
            .method("POST")
            .uri("/api/shell/show-item-in-folder")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"filePath":"/nonexistent/path"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}

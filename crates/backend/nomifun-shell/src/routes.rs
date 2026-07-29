use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use tower_http::limit::RequestBodyLimitLayer;

use nomifun_api_types::{
    ApiResponse, CheckToolInstalledRequest, CheckToolInstalledResponse, ClientPreferencesResponse,
    OpenExternalRequest, OpenFileRequest, OpenFolderWithRequest, ShowItemInFolderRequest,
    SpeechToTextConfig, SpeechToTextProvider, TtsApiRequest,
};
use nomifun_common::AppError;
use nomifun_model_invoke::{ModelRef, ProducedData, TaskOutcome, TaskRequest, TaskResult, TtsRequest};

use crate::error::SttError;
use crate::state::ShellRouterState;
use crate::stt::CloudSttRoute;

/// Hard ceiling on `/api/tts` input length (characters). Mirrors the OpenAI
/// `/audio/speech` contract's own 4096-character input cap.
const MAX_TTS_TEXT_CHARS: usize = 4096;

pub fn shell_routes(state: ShellRouterState) -> Router {
    let shell = Router::new()
        .route("/api/shell/open-file", post(open_file))
        .route("/api/shell/show-item-in-folder", post(show_item_in_folder))
        .route("/api/shell/open-external", post(open_external))
        .route("/api/shell/check-tool-installed", post(check_tool_installed))
        .route("/api/shell/open-folder-with", post(open_folder_with))
        .route("/api/tts", post(text_to_speech));
    let stt = Router::new()
        .route("/api/stt", post(speech_to_text))
        // Disable the application's 10 MiB extractor default, then make the
        // transport layer the sole cap: 30 MiB audio plus multipart overhead.
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(31 * 1024 * 1024));
    shell.merge(stt).with_state(state)
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

/// `POST /api/tts` — synthesize speech through the unified invoke layer.
///
/// A BINARY endpoint (like the office preview routes, not the `ApiResponse`
/// envelope): a successful synthesis answers `200` with the audio bytes and
/// the asset's MIME as `Content-Type`. Errors ride the standard `AppError`
/// JSON body via the invoke crate's `From<InvokeError>` mapping.
async fn text_to_speech(
    State(state): State<ShellRouterState>,
    body: Result<Json<TtsApiRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if req.text.trim().is_empty() {
        return Err(AppError::BadRequest("text must not be empty".to_owned()));
    }
    let char_count = req.text.chars().count();
    if char_count > MAX_TTS_TEXT_CHARS {
        return Err(AppError::BadRequest(format!(
            "text is {char_count} characters; the limit is {MAX_TTS_TEXT_CHARS}"
        )));
    }
    let Some(invoke) = state.model_invoke_service.as_ref() else {
        return Err(AppError::Internal(
            "model invoke service is unavailable for speech synthesis".to_owned(),
        ));
    };

    let model_ref = ModelRef { provider_id: req.provider_id, model: req.model };
    let request = TaskRequest::SpeechSynthesis(TtsRequest {
        text: req.text,
        voice: req.voice,
        format: req.format,
        extra: serde_json::json!({}),
    });
    let outcome = invoke.invoke(&model_ref, request).await.map_err(AppError::from)?;

    let TaskOutcome::Done(result) = outcome else {
        return Err(AppError::Internal(
            "speech synthesis returned an async job unexpectedly".to_owned(),
        ));
    };
    let TaskResult::Assets(assets) = result else {
        return Err(AppError::Internal(
            "speech synthesis returned a non-audio result".to_owned(),
        ));
    };
    let Some(asset) = assets.into_iter().next() else {
        return Err(AppError::BadGateway("provider returned no audio asset".to_owned()));
    };
    let ProducedData::Bytes(bytes) = asset.data else {
        return Err(AppError::BadGateway(
            "provider returned an audio URL instead of inline bytes".to_owned(),
        ));
    };
    let mime = asset.mime.unwrap_or_else(|| "audio/mpeg".to_owned());
    Ok((StatusCode::OK, [(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response())
}

struct SttMultipartFields {
    file_data: Vec<u8>,
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
    // `fileName` stays a required wire field for compatibility, but the invoke
    // layer derives the upload filename from the MIME type, so only presence
    // is validated here.
    file_name.ok_or_else(|| AppError::BadRequest("missing 'fileName' field".to_owned()))?;
    let mime_type = mime_type.ok_or_else(|| AppError::BadRequest("missing 'mimeType' field".to_owned()))?;

    Ok(SttMultipartFields {
        file_data,
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
        .get_preferences(Some(&["tools.speechToText", "speechToText"]))
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

    let config = speech_to_text_config_from_preferences(&prefs);

    let route = resolve_cloud_speech_to_text_config(&state, config)
        .await
        .map_err(|error| stt_error_response(&error))?;

    let result = state
        .stt_service
        .transcribe(
            fields.file_data,
            &fields.mime_type,
            fields.language_hint.as_deref(),
            &route,
        )
        .await
        .map_err(|e| stt_error_response(&e))?;

    let body = serde_json::json!({
        "success": true,
        "data": result,
    });
    Ok((StatusCode::OK, Json(body)))
}

fn speech_to_text_config_from_preferences(prefs: &ClientPreferencesResponse) -> SpeechToTextConfig {
    ["tools.speechToText", "speechToText"]
        .into_iter()
        .filter_map(|key| prefs.get(key))
        .find_map(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or(SpeechToTextConfig {
            enabled: false,
            provider: SpeechToTextProvider::Openai,
            provider_id: None,
            model: None,
            language: None,
            auto_send: None,
            openai: None,
            deepgram: None,
        })
}

/// Validate the stored speech preference against the provider catalog and
/// produce the invoke-layer coordinates ([`CloudSttRoute`]). The execution
/// protocol is NOT chosen here — the invoke layer's platform routing (plus any
/// model-row protocol override) decides it; the config's `provider` enum only
/// picks which legacy "not configured" error to surface.
async fn resolve_cloud_speech_to_text_config(
    state: &ShellRouterState,
    config: SpeechToTextConfig,
) -> Result<CloudSttRoute, SttError> {
    if !config.enabled {
        return Err(SttError::Disabled);
    }
    let Some(provider_id) = config.provider_id.as_deref() else {
        // Legacy embedded-credential configs (openai:/deepgram: blocks without
        // a provider_id) predate the provider catalog. The invoke layer only
        // executes catalog-backed models and the current UI always writes
        // provider_id mode, so this form is retired rather than emulated.
        if config.openai.is_some() || config.deepgram.is_some() {
            return Err(SttError::Unknown(
                "embedded-credential speech config is no longer supported; re-select your speech provider in Settings → 模型 → 语音识别".into(),
            ));
        }
        return Err(match config.provider {
            SpeechToTextProvider::Openai => SttError::OpenaiNotConfigured,
            SpeechToTextProvider::Deepgram => SttError::DeepgramNotConfigured,
        });
    };
    let Some(provider_service) = state.provider_service.as_ref() else {
        return Err(SttError::Unknown(
            "provider service is unavailable for speech recognition".into(),
        ));
    };
    let provider = provider_service
        .list()
        .await
        .map_err(|error| SttError::Unknown(error.to_string()))?
        .into_iter()
        .find(|provider| provider.provider_id == provider_id && provider.enabled)
        .ok_or_else(|| SttError::Unknown("selected speech provider was not found or is disabled".into()))?;
    if provider.api_key.trim().is_empty() {
        return Err(match config.provider {
            SpeechToTextProvider::Openai => SttError::OpenaiNotConfigured,
            SpeechToTextProvider::Deepgram => SttError::DeepgramNotConfigured,
        });
    }

    let model = config
        .model
        .clone()
        .ok_or_else(|| SttError::Unknown("selected speech provider has no selected speech model".into()))?;
    let model_is_enabled = provider
        .model_enabled
        .as_ref()
        .and_then(|models| models.get(&model))
        .copied()
        .unwrap_or(true);
    if !provider.models.contains(&model) || !model_is_enabled {
        return Err(SttError::Unknown(
            "selected speech model was not found or is disabled".into(),
        ));
    }
    let language = config.language.clone().filter(|value| !value.trim().is_empty());

    Ok(CloudSttRoute {
        provider_id: provider.provider_id,
        model,
        platform: provider.platform,
        language,
    })
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
    use bytes::Bytes;
    use http_body_util::BodyExt;
    use serde_json::json;
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
            stt_service: Arc::new(SttService::new(None)),
            client_pref_service,
            provider_service: None,
            model_invoke_service: None,
        }
    }

    fn make_router() -> Router {
        shell_routes(make_state())
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn multipart_request(audio_len: usize) -> Request<Body> {
        multipart_request_with_format(audio_len, "audio.wav", "audio/wav")
    }

    fn multipart_request_with_format(
        audio_len: usize,
        file_name: &str,
        mime_type: &str,
    ) -> Request<Body> {
        const BOUNDARY: &str = "nomifun-stt-limit-test";
        let prefix = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: {mime_type}\r\n\r\n"
        );
        let suffix = format!(
            "\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"fileName\"\r\n\r\n{file_name}\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"mimeType\"\r\n\r\n{mime_type}\r\n--{BOUNDARY}--\r\n"
        );
        let stream = futures_util::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from(prefix)),
            Ok(Bytes::from(vec![0_u8; audio_len])),
            Ok(Bytes::from(suffix)),
        ]);
        Request::builder()
            .method("POST")
            .uri("/api/stt")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from_stream(stream))
            .unwrap()
    }

    #[test]
    fn speech_to_text_config_prefers_tools_key_and_supports_legacy_key() {
        let legacy = json!({
            "enabled": true,
            "provider": "openai",
            "openai": {
                "api_key": "legacy-key",
                "model": "legacy-model"
            }
        });
        let current = json!({
            "enabled": true,
            "provider": "deepgram",
            "deepgram": {
                "api_key": "current-key",
                "model": "nova-2"
            }
        });

        let legacy_only = ClientPreferencesResponse::from([("speechToText".into(), legacy.clone())]);
        let config = speech_to_text_config_from_preferences(&legacy_only);
        assert!(matches!(
            config.provider,
            nomifun_api_types::SpeechToTextProvider::Openai
        ));
        assert_eq!(
            config.openai.as_ref().map(|value| value.api_key.as_str()),
            Some("legacy-key")
        );

        let both = ClientPreferencesResponse::from([
            ("speechToText".into(), legacy),
            ("tools.speechToText".into(), current),
        ]);
        let config = speech_to_text_config_from_preferences(&both);
        assert!(matches!(
            config.provider,
            nomifun_api_types::SpeechToTextProvider::Deepgram
        ));
        assert_eq!(
            config.deepgram.as_ref().map(|value| value.api_key.as_str()),
            Some("current-key")
        );
    }

    #[test]
    fn invalid_current_speech_to_text_config_falls_back_to_legacy_key() {
        let prefs = ClientPreferencesResponse::from([
            ("tools.speechToText".into(), json!({"enabled": true})),
            (
                "speechToText".into(),
                json!({
                    "enabled": true,
                    "provider": "openai",
                    "openai": {
                        "api_key": "legacy-key",
                        "model": "whisper-1"
                    }
                }),
            ),
        ]);

        let config = speech_to_text_config_from_preferences(&prefs);
        assert!(matches!(
            config.provider,
            nomifun_api_types::SpeechToTextProvider::Openai
        ));
        assert_eq!(
            config.openai.as_ref().map(|value| value.api_key.as_str()),
            Some("legacy-key")
        );
    }

    #[tokio::test]
    async fn stt_route_accepts_body_larger_than_global_ten_mib_limit() {
        let response = make_router()
            .oneshot(multipart_request(10 * 1024 * 1024 + 1))
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        // The lazy in-memory preference repository used by this unit test can
        // return 500 before configuration lookup; reaching that handler is the
        // contract under test (the transport did not reject at 10 MiB).
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::INTERNAL_SERVER_ERROR
        ));
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

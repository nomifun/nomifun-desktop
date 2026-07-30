//! Black-box tests for `POST /api/stt`: real in-memory catalog + client
//! preferences + wiremock provider behind the unified invoke layer.
//!
//! The execution protocol is decided by the invoke layer's platform routing —
//! a deepgram-platform provider is hit at `/v1/listen` with `Authorization:
//! Token …` even when the stored preference's `provider` enum guesses
//! "openai" (the legacy name-guess is display-only now); everything else
//! rides the OpenAI-compatible multipart `/v1/audio/transcriptions`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use nomifun_db::{
    CreateProviderParams, IProviderModelRepository, IProviderRepository, NewProviderModel,
    SqliteProviderConnectionRepository, SqliteProviderModelRepository, SqliteProviderRepository,
    init_database_memory,
};
use nomifun_model_invoke::{AdapterRegistry, ModelInvokeService, default_adapters};
use nomifun_shell::{NoopSystemOpener, ShellRouterState, ShellService, SttService, shell_routes};
use nomifun_system::{ClientPrefService, ProviderService};

const TEST_KEY: [u8; 32] = [0x42; 32];

/// Real in-memory DB + production adapters + provider/preference services
/// behind the shell router. The `Database` handle is forgotten (not dropped)
/// so the in-memory pool stays alive for the test.
async fn setup() -> (axum::Router, nomifun_db::SqlitePool) {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    std::mem::forget(db);

    let invoke = Arc::new(ModelInvokeService::new(
        Arc::new(SqliteProviderRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
        TEST_KEY,
        reqwest::Client::new(),
        AdapterRegistry::new(default_adapters()),
    ));

    let state = ShellRouterState {
        shell_service: Arc::new(ShellService::new(Arc::new(NoopSystemOpener))),
        stt_service: Arc::new(SttService::new(Some(invoke.clone()))),
        client_pref_service: ClientPrefService::new(Arc::new(
            nomifun_db::SqliteClientPreferenceRepository::new(pool.clone()),
        )),
        provider_service: Some(ProviderService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            TEST_KEY,
        )),
        model_invoke_service: Some(invoke),
    };
    (shell_routes(state), pool)
}

/// Seed an enabled provider on `platform` whose base_url is the mock server
/// (key decrypts to `sk-test`).
async fn seed_provider(pool: &nomifun_db::SqlitePool, platform: &str, base_url: &str) -> String {
    let repo = SqliteProviderRepository::new(pool.clone());
    let encrypted = nomifun_common::encrypt_string("sk-test", &TEST_KEY).unwrap();
    repo.create(CreateProviderParams {
        provider_id: None,
        platform,
        name: "Wiremock Provider",
        base_url,
        api_key_encrypted: &encrypted,
        models: "[]",
        enabled: true,
        capabilities: "[]",
        model_context_limits: None,
        model_protocols: None,
        model_descriptions: None,
        model_enabled: None,
        model_health: None,
        bedrock_config: None,
        is_full_url: false,
        sort_order: None,
    })
    .await
    .unwrap()
    .provider_id
}

async fn seed_model(pool: &nomifun_db::SqlitePool, provider_id: &str, model: &str, enabled: bool) {
    let repo = SqliteProviderModelRepository::new(pool.clone());
    repo.create(
        provider_id,
        &NewProviderModel {
            model,
            enabled,
            sort_order: 0,
            tasks: r#"["speech_recognition"]"#,
            traits: "[]",
            protocol: None,
            params: "{}",
            context_limit: None,
            description: None,
            source: "user",
            health: None,
        },
    )
    .await
    .unwrap();
}

/// Store the `tools.speechToText` preference the route reads.
async fn set_speech_pref(pool: &nomifun_db::SqlitePool, value: serde_json::Value) {
    let service = ClientPrefService::new(Arc::new(
        nomifun_db::SqliteClientPreferenceRepository::new(pool.clone()),
    ));
    let mut req = nomifun_api_types::UpdateClientPreferencesRequest::new();
    req.insert("tools.speechToText".to_owned(), value);
    service.update_preferences(req).await.unwrap();
}

fn stt_request() -> Request<Body> {
    const BOUNDARY: &str = "nomifun-stt-invoke-test";
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"clip.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"fileName\"\r\n\r\nclip.wav\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"mimeType\"\r\n\r\naudio/wav\r\n--{BOUNDARY}--\r\n"
    );
    Request::builder()
        .method("POST")
        .uri("/api/stt")
        .header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
        .body(Body::from(body))
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// ---------------------------------------------------------------------------
// openai-compatible platform → multipart /v1/audio/transcriptions
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_openai_platform_rides_invoke_multipart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_string_contains("name=\"file\""))
        .and(body_string_contains("name=\"model\""))
        .and(body_string_contains("whisper-1"))
        .and(body_string_contains("name=\"response_format\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "hello world"})))
        .expect(1)
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "openai", &server.uri()).await;
    seed_model(&pool, &pid, "whisper-1", true).await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider": "openai", "provider_id": pid, "model": "whisper-1"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["text"], "hello world");
    assert_eq!(body["data"]["model"], "whisper-1");
    assert_eq!(body["data"]["provider"], "openai");
}

// ---------------------------------------------------------------------------
// deepgram platform → /v1/listen with Token auth, frontend guess ignored
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_deepgram_platform_uses_token_header_despite_openai_guess() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/listen"))
        .and(header("authorization", "Token sk-test"))
        .and(header("content-type", "audio/wav"))
        .and(query_param("model", "nova-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "metadata": {"model_info": {"uuid-1": {"name": "2-general-nova"}}},
            "results": {"channels": [{
                "detected_language": "en",
                "alternatives": [{"transcript": "hello deepgram"}]
            }]}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "deepgram", &server.uri()).await;
    seed_model(&pool, &pid, "nova-2", true).await;
    // The stored `provider` enum deliberately guesses WRONG ("openai"): the
    // invoke layer's platform routing must ignore it and speak deepgram.
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider": "openai", "provider_id": pid, "model": "nova-2"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["text"], "hello deepgram");
    assert_eq!(body["data"]["model"], "2-general-nova");
    // The wire enum reports the executed platform, not the stored guess.
    assert_eq!(body["data"]["provider"], "deepgram");
    assert_eq!(body["data"]["language"], "en");
}

// ---------------------------------------------------------------------------
// stored config language wins and rides the request
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_config_language_is_forwarded() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .and(body_string_contains("name=\"language\""))
        .and(body_string_contains("zh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "你好"})))
        .expect(1)
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "openai", &server.uri()).await;
    seed_model(&pool, &pid, "whisper-1", true).await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider": "openai", "provider_id": pid, "model": "whisper-1", "language": "zh"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["data"]["text"], "你好");
    assert_eq!(body["data"]["language"], "zh");
}

// ---------------------------------------------------------------------------
// catalog gating: disabled model keeps the legacy error shape
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_disabled_model_error_unchanged() {
    let server = MockServer::start().await;
    // No mock mounted: the catalog check must refuse before the wire.
    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "openai", &server.uri()).await;
    seed_model(&pool, &pid, "whisper-1", false).await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider": "openai", "provider_id": pid, "model": "whisper-1"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(resp).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], "STT_UNKNOWN");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("selected speech model was not found or is disabled"),
        "error: {}",
        body["error"]
    );
    assert!(server.received_requests().await.unwrap().is_empty(), "must not reach the wire");
}

// ---------------------------------------------------------------------------
// legacy embedded-credential configs are retired
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_legacy_embedded_config_is_rejected() {
    let (app, pool) = setup().await;
    set_speech_pref(
        &pool,
        json!({
            "enabled": true,
            "provider": "openai",
            "openai": {"api_key": "sk-legacy", "model": "whisper-1"}
        }),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(resp).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], "STT_UNKNOWN");
    assert!(
        body["error"].as_str().unwrap().contains("no longer supported"),
        "error: {}",
        body["error"]
    );
}

// ---------------------------------------------------------------------------
// enabled without any provider selection → legacy "not configured" errors
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_enabled_without_selection_is_not_configured() {
    let (app, pool) = setup().await;
    set_speech_pref(&pool, json!({"enabled": true, "provider": "deepgram"})).await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["code"], "STT_DEEPGRAM_NOT_CONFIGURED");
}

// ---------------------------------------------------------------------------
// an empty-key embedded shell is "nothing configured", NOT a retired legacy
// credential — the NOT_CONFIGURED 400 family keeps firing
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_empty_key_embedded_shell_is_not_configured() {
    let (app, pool) = setup().await;
    set_speech_pref(
        &pool,
        json!({
            "enabled": true,
            "provider": "openai",
            "openai": {"api_key": "", "model": "whisper-1"}
        }),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["code"], "STT_OPENAI_NOT_CONFIGURED");
}

// ---------------------------------------------------------------------------
// disabled STT keeps the STT_DISABLED wire error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_disabled_returns_stt_disabled() {
    let (app, pool) = setup().await;
    set_speech_pref(&pool, json!({"enabled": false, "provider": "openai"})).await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], "STT_DISABLED");
}

// ---------------------------------------------------------------------------
// upstream failure maps to the legacy 502 STT_REQUEST_FAILED
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_upstream_401_maps_to_request_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}
        })))
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "openai", &server.uri()).await;
    seed_model(&pool, &pid, "whisper-1", true).await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider": "openai", "provider_id": pid, "model": "whisper-1"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = body_json(resp).await;
    assert_eq!(body["code"], "STT_REQUEST_FAILED");
    assert!(body["error"].as_str().unwrap().contains("401"), "error: {}", body["error"]);
}

// ---------------------------------------------------------------------------
// SttError → AppError conversion (unchanged legacy mapping)
// ---------------------------------------------------------------------------
#[test]
fn stt_error_to_app_error_mapping() {
    use nomifun_common::AppError;
    use nomifun_shell::SttError;

    let err: AppError = SttError::Disabled.into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = SttError::OpenaiNotConfigured.into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = SttError::DeepgramNotConfigured.into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = SttError::RequestFailed("upstream".into()).into();
    assert!(matches!(err, AppError::BadGateway(_)));

    let err: AppError = SttError::Unknown("bug".into()).into();
    assert!(matches!(err, AppError::Internal(_)));
}

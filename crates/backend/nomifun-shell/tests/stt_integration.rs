//! Black-box tests for `POST /api/stt`: real in-memory catalog + client
//! preferences + wiremock provider behind the unified invoke layer.
//!
//! Every fixture declares its speech-recognition protocol and auth scheme
//! explicitly; provider identity never selects either one.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use nomifun_db::{
    CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
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
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
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
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
        )),
        model_invoke_service: Some(invoke),
    };
    (shell_routes(state), pool)
}

/// Seed a provider whose speech-recognition capability explicitly owns its
/// protocol. The key decrypts to `sk-test`.
async fn seed_provider(
    pool: &nomifun_db::SqlitePool,
    platform: &str,
    base_url: &str,
    auth_scheme: &str,
    model: &str,
    protocol: &str,
    enabled: bool,
) -> String {
    let repo = SqliteProviderRepository::new(pool.clone());
    let encrypted =
        nomifun_common::encrypt_string(r#"{"api_keys":["sk-test"]}"#, &TEST_KEY).unwrap();
    let capabilities = [NewProviderModelCapability {
        task: "speech_recognition",
        traits: "[]",
        protocol,
        connection_role: "default",
        base_url_override: None,
        endpoint: None,
        poll_endpoint: None,
        content_endpoint: None,
        realtime_endpoint: None,
        allow_cross_origin_credentials: false,
        provider_params: "{}",
        context_limit: None,
        output_limit: None,
    }];
    let initial_model = NewProviderModel {
        model,
        enabled,
        sort_order: 0,
        description: None,
        capabilities: &capabilities,
    };
    repo.create(
        CreateProviderParams {
            provider_id: None,
            platform,
            name: "Wiremock Provider",
            base_url,
            auth_scheme,
            credentials_encrypted: &encrypted,
            enabled: true,
            bedrock_config: None,
            sort_order: None,
        },
        &initial_model,
        &[],
    )
    .await
    .unwrap()
    .0
    .provider_id
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
// Explicit OpenAI-compatible capability → multipart /v1/audio/transcriptions
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_openai_capability_rides_invoke_multipart() {
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
    let base_url = format!("{}/v1", server.uri());
    let pid = seed_provider(
        &pool,
        "openai",
        &base_url,
        "bearer",
        "whisper-1",
        "openai.audio_transcriptions",
        true,
    )
    .await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider_id": pid, "model": "whisper-1"}),
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
// Explicit Deepgram capability → /v1/listen with Token auth.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_deepgram_capability_uses_listen_with_token_header() {
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
    let pid = seed_provider(
        &pool,
        "deepgram",
        &server.uri(),
        "token",
        "nova-2",
        "deepgram.listen",
        true,
    )
    .await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider_id": pid, "model": "nova-2"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["text"], "hello deepgram");
    assert_eq!(body["data"]["model"], "2-general-nova");
    // The result reports the actual provider platform without enum coercion.
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
    let base_url = format!("{}/v1", server.uri());
    let pid = seed_provider(
        &pool,
        "openai",
        &base_url,
        "bearer",
        "whisper-1",
        "openai.audio_transcriptions",
        true,
    )
    .await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider_id": pid, "model": "whisper-1", "language": "zh"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["data"]["text"], "你好");
    assert_eq!(body["data"]["language"], "zh");
}

// ---------------------------------------------------------------------------
// catalog gating: disabled model is rejected before invocation
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_disabled_model_is_rejected_before_invocation() {
    let server = MockServer::start().await;
    // No mock mounted: the catalog check must refuse before the wire.
    let (app, pool) = setup().await;
    let base_url = format!("{}/v1", server.uri());
    let pid = seed_provider(
        &pool,
        "openai",
        &base_url,
        "bearer",
        "whisper-1",
        "openai.audio_transcriptions",
        false,
    )
    .await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider_id": pid, "model": "whisper-1"}),
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
            .contains("has no speech-recognition capability"),
        "error: {}",
        body["error"]
    );
    assert!(server.received_requests().await.unwrap().is_empty(), "must not reach the wire");
}

// ---------------------------------------------------------------------------
// enabled without any provider selection → one provider-agnostic error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_enabled_without_selection_is_not_configured() {
    let (app, pool) = setup().await;
    set_speech_pref(&pool, json!({"enabled": true})).await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["code"], "STT_NOT_CONFIGURED");
}

// ---------------------------------------------------------------------------
// disabled STT keeps the STT_DISABLED wire error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn stt_disabled_returns_stt_disabled() {
    let (app, pool) = setup().await;
    set_speech_pref(&pool, json!({"enabled": false})).await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], "STT_DISABLED");
}

// ---------------------------------------------------------------------------
// upstream failure maps to the 502 STT_REQUEST_FAILED
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
    let base_url = format!("{}/v1", server.uri());
    let pid = seed_provider(
        &pool,
        "openai",
        &base_url,
        "bearer",
        "whisper-1",
        "openai.audio_transcriptions",
        true,
    )
    .await;
    set_speech_pref(
        &pool,
        json!({"enabled": true, "provider_id": pid, "model": "whisper-1"}),
    )
    .await;

    let resp = app.oneshot(stt_request()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = body_json(resp).await;
    assert_eq!(body["code"], "STT_REQUEST_FAILED");
    assert!(body["error"].as_str().unwrap().contains("401"), "error: {}", body["error"]);
}

// ---------------------------------------------------------------------------
// SttError → AppError conversion
// ---------------------------------------------------------------------------
#[test]
fn stt_error_to_app_error_mapping() {
    use nomifun_common::AppError;
    use nomifun_shell::SttError;

    let err: AppError = SttError::Disabled.into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = SttError::NotConfigured.into();
    assert!(matches!(err, AppError::BadRequest(_)));

    let err: AppError = SttError::RequestFailed("upstream".into()).into();
    assert!(matches!(err, AppError::BadGateway(_)));

    let err: AppError = SttError::Unknown("bug".into()).into();
    assert!(matches!(err, AppError::Internal(_)));
}

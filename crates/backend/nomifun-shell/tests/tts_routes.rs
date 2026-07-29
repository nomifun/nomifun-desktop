//! Black-box tests for `POST /api/tts`: real in-memory catalog + wiremock
//! provider behind the unified invoke layer. Covers the binary happy path
//! (audio bytes + Content-Type, no ApiResponse envelope), the local input
//! validation (empty / oversized text), and catalog gating (unprofiled model).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use nomifun_db::{
    CreateProviderParams, IProviderModelRepository, IProviderRepository, NewProviderModel,
    SqliteProviderConnectionRepository, SqliteProviderModelRepository, SqliteProviderRepository,
    init_database_memory,
};
use nomifun_model_invoke::{AdapterRegistry, ModelInvokeService, default_adapters};
use nomifun_shell::{NoopSystemOpener, ShellRouterState, ShellService, SttService, shell_routes};

const TEST_KEY: [u8; 32] = [0x42; 32];

/// Real in-memory DB + the production adapter set behind the shell router.
/// Returns the router and the pool for seeding. The `Database` handle is
/// forgotten (not dropped) so the in-memory pool stays alive for the test.
async fn setup() -> (axum::Router, nomifun_db::SqlitePool) {
    let db = init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    std::mem::forget(db);

    let invoke = ModelInvokeService::new(
        Arc::new(SqliteProviderRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
        TEST_KEY,
        reqwest::Client::new(),
        AdapterRegistry::new(default_adapters()),
    );

    let state = ShellRouterState {
        shell_service: Arc::new(ShellService::new(Arc::new(NoopSystemOpener))),
        stt_service: Arc::new(SttService::new(reqwest::Client::new())),
        client_pref_service: nomifun_system::ClientPrefService::new(Arc::new(
            nomifun_db::SqliteClientPreferenceRepository::new(pool.clone()),
        )),
        provider_service: None,
        model_invoke_service: Some(Arc::new(invoke)),
    };
    (shell_routes(state), pool)
}

/// Seed an enabled openai-platform provider whose base_url is the mock server
/// (key decrypts to `sk-test` → `Authorization: Bearer sk-test`).
async fn seed_provider(pool: &nomifun_db::SqlitePool, base_url: &str) -> String {
    let repo = SqliteProviderRepository::new(pool.clone());
    let encrypted = nomifun_common::encrypt_string("sk-test", &TEST_KEY).unwrap();
    repo.create(CreateProviderParams {
        provider_id: None,
        platform: "openai",
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

async fn seed_model(pool: &nomifun_db::SqlitePool, provider_id: &str, model: &str, tasks: &str) {
    let repo = SqliteProviderModelRepository::new(pool.clone());
    repo.create(
        provider_id,
        &NewProviderModel {
            model,
            enabled: true,
            sort_order: 0,
            tasks,
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

fn tts_request(body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/tts")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn tts_returns_audio_bytes_with_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_partial_json(json!({
            "model": "tts-1",
            "input": "hello world",
            "voice": "alloy",
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/mpeg")
                .set_body_bytes(b"ID3fake-mp3".to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, &server.uri()).await;
    seed_model(&pool, &pid, "tts-1", r#"["speech_synthesis"]"#).await;

    let resp = app
        .oneshot(tts_request(json!({
            "provider_id": pid,
            "model": "tts-1",
            "text": "hello world",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("audio/mpeg"),
        "binary endpoint must answer with the asset MIME"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"ID3fake-mp3", "body must be the raw audio bytes");
}

#[tokio::test]
async fn tts_passes_voice_and_format_through() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .and(body_partial_json(json!({
            "model": "tts-1",
            "input": "hi",
            "voice": "nova",
            "response_format": "wav",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"RIFFwav".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, &server.uri()).await;
    seed_model(&pool, &pid, "tts-1", r#"["speech_synthesis"]"#).await;

    let resp = app
        .oneshot(tts_request(json!({
            "provider_id": pid,
            "model": "tts-1",
            "text": "hi",
            "voice": "nova",
            "format": "wav",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // The requested format pins the response MIME.
    assert_eq!(
        resp.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("audio/wav")
    );
}

#[tokio::test]
async fn tts_unprofiled_model_is_400_without_network() {
    let server = MockServer::start().await;
    // No mock mounted: the catalog gate must refuse before the wire.
    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, &server.uri()).await;
    seed_model(&pool, &pid, "gpt-4o", r#"["chat"]"#).await;

    let resp = app
        .oneshot(tts_request(json!({
            "provider_id": pid,
            "model": "gpt-4o",
            "text": "hi",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "the task gate must fire before the wire"
    );
}

#[tokio::test]
async fn tts_empty_text_is_400() {
    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "https://unused.example").await;
    seed_model(&pool, &pid, "tts-1", r#"["speech_synthesis"]"#).await;

    let resp = app
        .oneshot(tts_request(json!({
            "provider_id": pid,
            "model": "tts-1",
            "text": "   ",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tts_text_over_4096_chars_is_400() {
    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, "https://unused.example").await;
    seed_model(&pool, &pid, "tts-1", r#"["speech_synthesis"]"#).await;

    let resp = app
        .oneshot(tts_request(json!({
            "provider_id": pid,
            "model": "tts-1",
            "text": "x".repeat(4097),
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn tts_upstream_401_maps_to_bad_gateway() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&server)
        .await;

    let (app, pool) = setup().await;
    let pid = seed_provider(&pool, &server.uri()).await;
    seed_model(&pool, &pid, "tts-1", r#"["speech_synthesis"]"#).await;

    let resp = app
        .oneshot(tts_request(json!({
            "provider_id": pid,
            "model": "tts-1",
            "text": "hi",
        })))
        .await
        .unwrap();
    // InvokeError::Auth (upstream rejected stored credentials) → BadGateway.
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn tts_unwired_invoke_service_is_500() {
    // Unit-style state without the invoke service: the route degrades to an
    // internal error rather than panicking (mirrors provider_service: None).
    let pool = sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap();
    let state = ShellRouterState {
        shell_service: Arc::new(ShellService::new(Arc::new(NoopSystemOpener))),
        stt_service: Arc::new(SttService::new(reqwest::Client::new())),
        client_pref_service: nomifun_system::ClientPrefService::new(Arc::new(
            nomifun_db::SqliteClientPreferenceRepository::new(pool),
        )),
        provider_service: None,
        model_invoke_service: None,
    };
    let resp = shell_routes(state)
        .oneshot(tts_request(json!({
            "provider_id": "018f0000-0000-7000-8000-000000000001",
            "model": "tts-1",
            "text": "hi",
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

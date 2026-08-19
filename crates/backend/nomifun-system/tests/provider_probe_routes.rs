//! Black-box tests for the provider-level connection probe.
//!
//! These cover the exact failure modes a real OpenAI-compatible gateway
//! produces and that the previous design could not distinguish:
//!
//! - a wrong path answering `200 OK` with the gateway's SPA (not a success);
//! - the correct path answering `401` (the address is confirmed, the key is not);
//! - a genuinely absent path answering `404`;
//! - and a bare root that works when the configured versioned root does not.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use nomifun_common::encrypt_string;
use nomifun_db::{
    CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderRepository, init_database_memory,
};
use nomifun_system::{SystemRouterState, VersionCheckService, system_routes};

const TEST_KEY: [u8; 32] = [0x42; 32];

fn build_state(db: &nomifun_db::Database) -> SystemRouterState {
    let http_client = reqwest::Client::builder().no_proxy().build().unwrap();
    common::build_system_state(
        db,
        TEST_KEY,
        http_client.clone(),
        VersionCheckService::new(http_client, "0.1.0".to_owned()),
        None,
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    )
}

async fn setup() -> (axum::Router, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let state = build_state(&db);
    (system_routes(state), db)
}

async fn create_provider(db: &nomifun_db::Database, base_url: &str) -> String {
    let repo = SqliteProviderRepository::new(db.pool().clone());
    let encrypted =
        encrypt_string(&json!({"api_keys": ["sk-test"]}).to_string(), &TEST_KEY).unwrap();
    let capabilities = [NewProviderModelCapability {
        task: "chat",
        traits: "[]",
        protocol: "openai.chat_text",
        connection_role: "default",
        provider_params: "{}",
        ..Default::default()
    }];
    let initial_model = NewProviderModel {
        model: "test-model",
        enabled: true,
        capabilities: &capabilities,
        ..Default::default()
    };
    let (row, _) = repo
        .create(
            CreateProviderParams {
                provider_id: None,
                platform: "custom",
                name: "Test Provider",
                base_url,
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
            },
            &initial_model,
            &[],
        )
        .await
        .unwrap();
    row.provider_id
}

async fn probe(router: &axum::Router, provider_id: &str, body: serde_json::Value) -> serde_json::Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/providers/{provider_id}/probe-connection"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "probe route must answer 200");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// The reported situation: the key is dead, so nothing can list models — but the
/// address is right, and the probe must say so instead of reporting a failure
/// indistinguishable from a wrong URL.
#[tokio::test]
async fn a_401_on_the_correct_path_confirms_the_address() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({"code": "INVALID_API_KEY"})),
        )
        .mount(&server)
        .await;

    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &format!("{}/v1", server.uri())).await;
    let body = probe(&router, &provider_id, json!({"protocol": "openai.chat_text"})).await;

    assert_eq!(body["data"]["reachability"], "credentials_rejected");
    assert_eq!(body["data"]["error_kind"], "unauthorized");
    assert_eq!(body["data"]["http_status"], 401);
    assert_eq!(
        body["data"]["attempted_url"],
        format!("{}/v1/chat/completions", server.uri())
    );
    assert_eq!(body["data"]["root_shape"], "versioned_root");
}

/// A gateway serving its SPA at a near-miss path answers `200 OK` with HTML.
/// Calling that reachable is how a wrong base URL survives configuration review.
#[tokio::test]
async fn a_200_html_body_is_reported_as_unreachable_not_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("<!doctype html><html><body>gateway</body></html>", "text/html"),
        )
        .mount(&server)
        .await;

    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &server.uri()).await;
    let body = probe(
        &router,
        &provider_id,
        json!({"protocol": "openai.chat_text", "probe_candidates": false}),
    )
    .await;

    assert_eq!(body["data"]["reachability"], "unreachable");
    assert_eq!(body["data"]["error_kind"], "non_api_response");
    // The status really is 200; the diagnosis must not rely on it.
    assert_eq!(body["data"]["http_status"], 200);
    assert!(
        body["data"]["message"]
            .as_str()
            .unwrap()
            .contains("web page"),
        "got: {}",
        body["data"]["message"]
    );
}

/// The repair mechanism the previous design computed and then discarded: when the
/// configured root has no API but another candidate does, offer it.
#[tokio::test]
async fn a_working_bare_root_is_offered_when_the_configured_root_has_no_api() {
    let server = MockServer::start().await;
    // The configured `/v1` root would resolve to `/v1/chat/completions`.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_string("404 page not found"))
        .mount(&server)
        .await;
    // The bare root works.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "no model"})))
        .mount(&server)
        .await;

    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &format!("{}/v1", server.uri())).await;
    let body = probe(&router, &provider_id, json!({"protocol": "openai.chat_text"})).await;

    assert_eq!(body["data"]["reachability"], "unreachable");
    assert_eq!(body["data"]["error_kind"], "not_found");
    assert_eq!(
        body["data"]["suggested_base_url"].as_str().unwrap(),
        server.uri(),
        "the bare root must be probed and offered; candidates: {}",
        body["data"]["candidates"]
    );
    // No candidate may ever be a doubled version segment.
    let candidates = body["data"]["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());
    for candidate in candidates {
        let url = candidate["attempted_url"].as_str().unwrap();
        assert!(!url.contains("/v1/v1"), "manufactured a doubled version: {url}");
    }
}

/// A custom provider has no protocol recommendation by design, so the probe must
/// say what it needs rather than guess a protocol and report a misleading result.
#[tokio::test]
async fn a_custom_provider_without_a_protocol_is_a_bad_request() {
    let (router, db) = setup().await;
    let provider_id = create_provider(&db, "https://example.invalid/v1").await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/providers/{provider_id}/probe-connection"))
                .header("content-type", "application/json")
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        body["error"].as_str().unwrap_or_default().contains("protocol"),
        "the error must name what is missing: {body}"
    );
}

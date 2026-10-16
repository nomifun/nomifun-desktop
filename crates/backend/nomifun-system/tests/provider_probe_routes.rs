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
use wiremock::matchers::{body_json, header, method, path};
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

#[tokio::test]
async fn responses_probe_uses_the_responses_wire_endpoint_without_generating_content() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/relay/v1/responses"))
        .and(header("authorization", "Bearer sk-test"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "Missing required parameter: model"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &format!("{}/relay/v1", server.uri())).await;
    let body = probe(&router, &provider_id, json!({"protocol":"openai.responses","probe_candidates":false})).await;
    assert_eq!(body["data"]["reachability"], "reachable");
    assert_eq!(body["data"]["attempted_url"], format!("{}/relay/v1/responses", server.uri()));
    assert_eq!(body["data"]["protocol"], "openai.responses");
    assert!(!server.received_requests().await.unwrap()[0].headers.contains_key("anthropic-version"));
}

#[tokio::test]
async fn anthropic_empty_request_sends_native_headers_but_only_confirms_endpoint_reachability() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "anthropic-probe-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "type": "error", "error": {"type": "invalid_request_error", "message": "messages is required"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (router, db) = setup().await;
    let response = router.oneshot(Request::builder().method("POST").uri("/api/providers/probe-connection")
        .header("content-type", "application/json")
        .body(Body::from(json!({
            "platform": "anthropic", "base_url": server.uri(), "auth_scheme": "header_key:x-api-key",
            "credentials": {"api_keys": ["anthropic-probe-key"]}, "protocol": "anthropic.messages",
            "task": "chat", "probe_candidates": false
        }).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["data"]["reachability"], "reachable");
    assert_eq!(body["data"]["http_status"], 400);
    assert_eq!(body["data"]["protocol"], "anthropic.messages");
    assert!(body["data"].get("status").is_none(), "a reachability probe must not claim model health");
    assert!(body["data"].get("healthy").is_none());
    assert!(!body.to_string().contains("anthropic-probe-key"));
    assert!(SqliteProviderRepository::new(db.pool().clone()).list().await.unwrap().is_empty());
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
}

/// A gateway that authorizes per model — a New API key with a model allowlist —
/// authenticates the key first and then refuses the model-less probe body with
/// a 403 permission error. That refusal must not be reported as a rejected key:
/// only a 401 carries that meaning.
#[tokio::test]
async fn a_per_model_403_confirms_the_endpoint_while_a_401_rejects_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "valid-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "type": "error",
            "error": {
                "type": "permission_error",
                "message": "该模型暂无权限访问 [trace_id=trace-fixture-403 code=40004]"
            },
            "request_id": "req-fixture-403"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "invalid-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "type": "error",
            "error": {
                "type": "authentication_error",
                "message": "Invalid API Key! [trace_id=trace-fixture-401 code=40001]"
            },
            "request_id": "req-fixture-401"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (router, db) = setup().await;
    let probe_anonymous = |key: &str| {
        let router = router.clone();
        let base_url = server.uri();
        let key = key.to_owned();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/providers/probe-connection")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({
                                "platform": "custom",
                                "base_url": base_url,
                                "auth_scheme": "header_key:x-api-key",
                                "credentials": {"api_keys": [key]},
                                "protocol": "anthropic.messages",
                                "task": "chat",
                                "probe_candidates": true
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "probe route must answer 200");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        }
    };

    let body = probe_anonymous("valid-key").await;
    assert_eq!(body["data"]["reachability"], "reachable");
    assert_eq!(body["data"]["error_kind"], "forbidden");
    assert_eq!(body["data"]["http_status"], 403);
    assert!(
        body["data"].get("suggested_base_url").is_none()
            || body["data"]["suggested_base_url"].is_null(),
        "a 403 is not a wrong-root signal: {}",
        body["data"]
    );
    assert!(
        body["data"].get("candidates").is_none()
            || body["data"]["candidates"].as_array().unwrap().is_empty(),
        "an authenticated endpoint must not fan out to root candidates: {}",
        body["data"]
    );
    assert!(!body.to_string().contains("valid-key"));

    let body = probe_anonymous("invalid-key").await;
    assert_eq!(body["data"]["reachability"], "credentials_rejected");
    assert_eq!(body["data"]["error_kind"], "unauthorized");
    assert_eq!(body["data"]["http_status"], 401);
    assert!(!body.to_string().contains("invalid-key"));

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2, "no root-candidate fan-out may occur");
    for (index, key) in ["valid-key", "invalid-key"].iter().enumerate() {
        assert_eq!(
            requests[index].headers["x-api-key"].to_str().unwrap(),
            *key
        );
    }
    assert!(
        SqliteProviderRepository::new(db.pool().clone())
            .list()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn missing_post_handlers_and_removed_endpoints_do_not_confirm_a_protocol() {
    for status in [405, 410] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({"error":"unavailable"})))
            .mount(&server)
            .await;
        let (router, db) = setup().await;
        let provider_id = create_provider(&db, &format!("{}/v1", server.uri())).await;
        let body = probe(&router, &provider_id, json!({"protocol":"openai.responses","probe_candidates":false})).await;
        assert_eq!(body["data"]["reachability"], "unreachable", "{status}");
        assert_eq!(body["data"]["http_status"], status);
    }
}

#[tokio::test]
async fn a_reachable_protocol_root_outranks_a_generic_credential_rejection() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":"unauthorized"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/responses"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"model is required"})))
        .mount(&server)
        .await;
    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &format!("{}/v1", server.uri())).await;
    let body = probe(&router, &provider_id, json!({"protocol":"openai.responses"})).await;
    assert_eq!(body["data"]["suggested_base_url"], format!("{}/api/v1", server.uri()));
}

#[tokio::test]
async fn html_sniffing_preserves_non_api_diagnosis_even_with_an_incorrect_content_type() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("<!doctype html><html>gateway</html>", "application/json"))
        .mount(&server)
        .await;
    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &format!("{}/v1", server.uri())).await;
    let body = probe(&router, &provider_id, json!({"protocol":"openai.responses","probe_candidates":false})).await;
    assert_eq!(body["data"]["reachability"], "unreachable");
    assert_eq!(body["data"]["error_kind"], "non_api_response");
}

/// Protocols such as Gemini embed the selected model in the request path. The
/// configuration probe must use the draft model instead of a placeholder that
/// can turn a valid endpoint into a misleading model-not-found 404.
#[tokio::test]
async fn a_concrete_model_is_interpolated_into_the_probe_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1beta/models/gemini-2.5-pro:streamGenerateContent",
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "request body is required"
        })))
        .mount(&server)
        .await;

    let (router, db) = setup().await;
    let provider_id = create_provider(&db, &server.uri()).await;
    let body = probe(
        &router,
        &provider_id,
        json!({
            "protocol": "gemini.generate_text",
            "task": "chat",
            "model": "gemini-2.5-pro",
            "probe_candidates": false
        }),
    )
    .await;

    assert_eq!(body["data"]["reachability"], "reachable");
    assert!(
        body["data"]["attempted_url"]
            .as_str()
            .unwrap()
            .contains("/v1beta/models/gemini-2.5-pro:streamGenerateContent")
    );
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

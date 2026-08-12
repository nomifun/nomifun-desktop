//! Save-time contract tests for the four Agent Chat protocol families.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use nomifun_db::init_database_memory;
use nomifun_system::{SystemRouterState, VersionCheckService, system_routes};

const TEST_KEY: [u8; 32] = [0x6A; 32];

fn build_state(db: &nomifun_db::Database) -> SystemRouterState {
    let http = reqwest::Client::new();
    common::build_system_state(
        db,
        TEST_KEY,
        http.clone(),
        VersionCheckService::new(http, "0.1.0".into()),
        None,
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    )
}

fn request(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/providers")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn http_provider(
    platform: &str,
    auth_scheme: &str,
    protocol: &str,
    model: &str,
    endpoint: &str,
) -> Value {
    json!({
        "platform": platform,
        "name": format!("{platform} contract"),
        "base_url": format!("https://api.{platform}.example"),
        "auth_scheme": auth_scheme,
        "credentials": {"api_keys":["sk-contract"]},
        "initial_model": {
            "model": model,
            "capabilities": [{
                "task": "chat",
                "protocol": protocol,
                "connection_role": "default",
                "endpoint": endpoint,
                "provider_params": {}
            }]
        },
        "connections": []
    })
}

fn bedrock_provider() -> Value {
    json!({
        "platform": "bedrock",
        "name": "Bedrock contract",
        "base_url": "",
        "auth_scheme": "bedrock",
        "credentials": {},
        "bedrock_config": {
            "auth_method": "profile",
            "region": "us-east-1",
            "profile": "default"
        },
        "initial_model": {
            "model": "anthropic.claude-contract-v1:0",
            "capabilities": [{
                "task": "chat",
                "protocol": "bedrock.anthropic_messages",
                "connection_role": "default",
                "provider_params": {}
            }]
        },
        "connections": []
    })
}

#[tokio::test]
async fn agent_chat_protocols_accept_only_their_executable_auth_scheme() {
    let valid = [
        http_provider(
            "openai",
            "bearer",
            "openai.chat_text",
            "gpt-contract",
            "/custom/chat?api-version=2026-08-11",
        ),
        http_provider(
            "anthropic",
            "header_key:x-api-key",
            "anthropic.messages",
            "claude-contract",
            "/v1/messages",
        ),
        http_provider(
            "gemini",
            "header_key:x-goog-api-key",
            "gemini.generate_text",
            "gemini-contract",
            "/v1beta/models/{model}:streamGenerateContent?alt=sse",
        ),
        bedrock_provider(),
    ];

    for body in valid {
        let platform = body["platform"].as_str().unwrap().to_owned();
        let db = init_database_memory().await.unwrap();
        let response = system_routes(build_state(&db))
            .oneshot(request(body))
            .await
            .unwrap();
        let status = response.status();
        let response_body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{platform}: {}",
            String::from_utf8_lossy(&response_body)
        );
    }

    let mut invalid = [
        http_provider(
            "openai",
            "header_key:x-api-key",
            "openai.chat_text",
            "gpt-contract",
            "/chat/completions",
        ),
        http_provider(
            "anthropic",
            "bearer",
            "anthropic.messages",
            "claude-contract",
            "/v1/messages",
        ),
        http_provider(
            "gemini",
            "bearer",
            "gemini.generate_text",
            "gemini-contract",
            "/v1beta/models/{model}:streamGenerateContent?alt=sse",
        ),
        bedrock_provider(),
    ];
    invalid[3]["auth_scheme"] = json!("bearer");
    invalid[3]["credentials"] = json!({"api_keys":["sk-contract"]});

    for body in invalid {
        let db = init_database_memory().await.unwrap();
        let response = system_routes(build_state(&db))
            .oneshot(request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn bedrock_sdk_save_rejects_every_http_transport_field() {
    let mut variants = Vec::new();

    let mut provider_base = bedrock_provider();
    provider_base["base_url"] = json!("https://bedrock-runtime.us-east-1.amazonaws.com");
    variants.push(provider_base);

    for (field, value) in [
        ("base_url_override", "https://runtime.example/v1"),
        ("endpoint", "/converse"),
        ("poll_endpoint", "/jobs/{id}"),
        ("content_endpoint", "/jobs/{id}/content"),
        ("realtime_endpoint", "wss://runtime.example/session"),
    ] {
        let mut body = bedrock_provider();
        body["initial_model"]["capabilities"][0][field] = json!(value);
        variants.push(body);
    }

    let mut named_connection = bedrock_provider();
    named_connection["initial_model"]["capabilities"][0]["connection_role"] = json!("runtime");
    named_connection["connections"] = json!([{
        "role": "runtime",
        "base_url": "https://runtime.example/v1",
        "auth_scheme": "bedrock",
        "credentials": {},
        "extra": {}
    }]);
    variants.push(named_connection);

    for body in variants {
        let db = init_database_memory().await.unwrap();
        let response = system_routes(build_state(&db))
            .oneshot(request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

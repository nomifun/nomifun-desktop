//! Integration tests for ConnectionTestService.
//!
//! Tests validate input checking, service construction, and error paths.
//! Real AWS calls are tested only with fake credentials to verify
//! proper error handling (no real accounts needed).

use nomifun_api_types::{BedrockAuthMethod, BedrockConfig};
use nomifun_system::ConnectionTestService;
use serde_json::json;

fn make_service() -> ConnectionTestService {
    ConnectionTestService::new()
}

// ── Bedrock validation ──────────────────────────────────────────────

#[tokio::test]
async fn bedrock_rejects_empty_region() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "".into(),
        profile: None,
    };
    let err = svc
        .test_bedrock_connection(
            config,
            json!({"access_key_id":"AKIA","secret_access_key":"secret"}),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("region"));
}

#[tokio::test]
async fn bedrock_rejects_missing_access_key_id() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "us-east-1".into(),
        profile: None,
    };
    let err = svc
        .test_bedrock_connection(config, json!({"secret_access_key":"secret"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("access_key_id"));
}

#[tokio::test]
async fn bedrock_rejects_missing_secret_access_key() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "us-east-1".into(),
        profile: None,
    };
    let err = svc
        .test_bedrock_connection(config, json!({"access_key_id":"AKIA"}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("secret_access_key"));
}

#[tokio::test]
async fn bedrock_rejects_empty_profile() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::Profile,
        region: "us-east-1".into(),
        profile: Some("".into()),
    };
    let err = svc
        .test_bedrock_connection(config, json!({}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("profile"));
}

#[tokio::test]
async fn bedrock_rejects_none_profile() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::Profile,
        region: "us-east-1".into(),
        profile: None,
    };
    let err = svc
        .test_bedrock_connection(config, json!({}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("profile"));
}

#[tokio::test]
async fn bedrock_fake_credentials_error() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "us-east-1".into(),
        profile: None,
    };
    // Should fail with credential error, not panic
    let err = svc
        .test_bedrock_connection(
            config,
            json!({
                "access_key_id":"AKIAFAKEKEY1234567890",
                "secret_access_key":"fakesecretkey1234567890abcdefgh"
            }),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("Bedrock credentials invalid"),
        "Expected credential error, got: {err}"
    );
}

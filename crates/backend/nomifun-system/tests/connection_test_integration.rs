//! Integration tests for ConnectionTestService.
//!
//! Tests validate input checking, service construction, and error paths.
//! Real AWS calls are tested only with fake credentials to verify
//! proper error handling (no real accounts needed).

use nomifun_api_types::{BedrockAuthMethod, BedrockConfig};
use nomifun_system::ConnectionTestService;

fn make_service() -> ConnectionTestService {
    ConnectionTestService::new(reqwest::Client::new())
}

// ── Bedrock validation ──────────────────────────────────────────────

#[tokio::test]
async fn bedrock_rejects_empty_region() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "".into(),
        access_key_id: Some("AKIA".into()),
        secret_access_key: Some("secret".into()),
        profile: None,
    };
    let err = svc.test_bedrock_connection(config).await.unwrap_err();
    assert!(err.to_string().contains("region"));
}

#[tokio::test]
async fn bedrock_rejects_missing_access_key_id() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "us-east-1".into(),
        access_key_id: None,
        secret_access_key: Some("secret".into()),
        profile: None,
    };
    let err = svc.test_bedrock_connection(config).await.unwrap_err();
    assert!(err.to_string().contains("accessKeyId"));
}

#[tokio::test]
async fn bedrock_rejects_missing_secret_access_key() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "us-east-1".into(),
        access_key_id: Some("AKIA".into()),
        secret_access_key: None,
        profile: None,
    };
    let err = svc.test_bedrock_connection(config).await.unwrap_err();
    assert!(err.to_string().contains("secretAccessKey"));
}

#[tokio::test]
async fn bedrock_rejects_empty_profile() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::Profile,
        region: "us-east-1".into(),
        access_key_id: None,
        secret_access_key: None,
        profile: Some("".into()),
    };
    let err = svc.test_bedrock_connection(config).await.unwrap_err();
    assert!(err.to_string().contains("profile"));
}

#[tokio::test]
async fn bedrock_rejects_none_profile() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::Profile,
        region: "us-east-1".into(),
        access_key_id: None,
        secret_access_key: None,
        profile: None,
    };
    let err = svc.test_bedrock_connection(config).await.unwrap_err();
    assert!(err.to_string().contains("profile"));
}

#[tokio::test]
async fn bedrock_fake_credentials_error() {
    let svc = make_service();
    let config = BedrockConfig {
        auth_method: BedrockAuthMethod::AccessKey,
        region: "us-east-1".into(),
        access_key_id: Some("AKIAFAKEKEY1234567890".into()),
        secret_access_key: Some("fakesecretkey1234567890abcdefgh".into()),
        profile: None,
    };
    // Should fail with credential error, not panic
    let err = svc.test_bedrock_connection(config).await.unwrap_err();
    assert!(
        err.to_string().contains("Bedrock credentials invalid"),
        "Expected credential error, got: {err}"
    );
}

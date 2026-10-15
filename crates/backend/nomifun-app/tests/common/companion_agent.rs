//! Shared companion binding fixtures for focused route tests.
use axum::{body::Body, http::{Request, StatusCode}};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const COMPANION_AGENT_TEST_TRUST: &str = "companion-product-agent";

pub async fn companion_agent_test_request(
    router: axum::Router,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", COMPANION_AGENT_TEST_TRUST).header("content-type", "application/json");
    if method == "PUT" && path.ends_with("/agent") {
        request = request.header("idempotency-key", uuid::Uuid::now_v7().to_string());
    }
    let response = router.oneshot(request
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_else(|error|
        panic!("{method} {path} returned {status}: {error}; {}", String::from_utf8_lossy(&bytes))))
}

// Save a core-only official configuration explicitly. Release defaults must not overwrite authored choices.
pub async fn save_companion_core_agent_for_fixture(router: axum::Router) {
    let call = companion_agent_test_request;
    let (status, original_library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{original_library}");
    let official_id = original_library["data"]["official_presets"]["companion.default"]["preset_id"].as_str().unwrap();
    let (status, original_editor) = call(router.clone(), "GET",
        &format!("/api/agent-presets/{official_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{original_editor}");
    let mut core_draft = original_editor["data"]["draft"].clone();
    core_draft["document"]["enabled_capabilities"] = json!([
        {"capability":{"id":"companion"},"action_allowlist":["companion/evolve","companion/learn"]},
        {"capability":{"id":"companion.memory"},"action_allowlist":["companion.memory/recall","companion.memory/write"]},
        {"capability":{"id":"knowledge"},"action_allowlist":["knowledge/read","knowledge/search"]},
        {"capability":{"id":"channel.messaging"},"action_allowlist":["channel.messaging/reply"]},
        {"capability":{"id":"robot"},"action_allowlist":["robot/vision"]},
        {"capability":{"id":"automation.schedule"},"action_allowlist":["automation.schedule/create","automation.schedule/delete","automation.schedule/list","automation.schedule/update"]}
    ]);
    let (status, saved_core) = call(router.clone(), "POST",
        &format!("/api/agent-presets/{official_id}/revisions"), json!({
            "expected_current_revision":original_editor["data"]["revision"]["reference"], "draft":core_draft,
        })).await;
    assert_eq!(status, StatusCode::OK, "{saved_core}");
}

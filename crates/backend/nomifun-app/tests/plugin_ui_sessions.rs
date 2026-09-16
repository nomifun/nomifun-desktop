//! Ordinary App surfaces retain storage while retired Session grants fail closed.
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

#[path = "common/mod.rs"]
mod common;

#[path = "plugin_ui_binding.rs"]
mod ui_binding;

#[path = "plugin_ui_admission.rs"]
mod admission;

const TRUST: &str = "plugin-ui-session-test";

async fn request(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

async fn post(router: &axum::Router, path: &str, body: Value) -> Value {
    let (status, result) = request(router, "POST", path, body).await;
    assert!(status.is_success(), "{path}: {status} {result}");
    result["data"].clone()
}

async fn install_ui(router: &axum::Router) -> Value {
    let draft = post(router, "/api/plugins/runtimes/import/inspect", json!({
        "filename": "ordinary-app.html",
        "content": "<!doctype html><html><head><title>Ordinary app</title></head><body><main>Agent</main></body></html>"
    })).await;
    post(
        router,
        &format!("/api/plugins/drafts/{}/save", draft["id"].as_str().unwrap()),
        json!({"expected_revision": draft["revision"]}),
    )
    .await["plugin"]
        .clone()
}

fn bridge_body(surface: &Value, command: Value) -> Value {
    json!({
        "surface_capability": surface["surface_capability"],
        "active_release_epoch": surface["active_release_epoch"],
        "expected_release_digest": surface["expected_release_digest"],
        "request": {"call_id": uuid::Uuid::now_v7().to_string(),
            "target": {"target": "agent_session", "request": command}}
    })
}

fn storage_body(surface: &Value, request: Value) -> Value {
    let mut value = bridge_body(surface, Value::Null);
    value["request"]["target"] = json!({"target": "host_kv", "request": request});
    value
}

async fn get(router: &axum::Router, path: &str) -> Value {
    let (status, body) = request(router, "GET", path, Value::Null).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body["data"].clone()
}

#[tokio::test]
async fn ordinary_app_surface_keeps_storage_without_any_session_grant() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let plugin = install_ui(&router).await;
    let id = plugin["plugin_id"].as_str().unwrap();
    let base = format!("/api/plugins/runtimes/{id}");
    let surface = post(
        &router,
        &format!("{base}/surface/open"),
        json!({"plugin_id":id}),
    )
    .await;
    let bridge = format!("{base}/surface/bridge");
    post(
        &router,
        &bridge,
        storage_body(
            &surface,
            json!({"operation":"set","key":"draft","value":"ordinary data"}),
        ),
    )
    .await;
    let stored = post(
        &router,
        &bridge,
        storage_body(&surface, json!({"operation":"get","key":"draft"})),
    )
    .await;
    assert_eq!(stored["value"], "ordinary data");
    for operation in [
        json!({"operation":"observe","after_seq":0,"limit":10}),
        json!({"operation":"turn","input":{"content":"must not run"},"idempotency_key":"retired"}),
        json!({"operation":"cancel"}),
    ] {
        let (status, error) =
            request(&router, "POST", &bridge, bridge_body(&surface, operation)).await;
        assert!(!status.is_success(), "{error}");
        assert!(error.to_string().contains("unsupported"), "{error}");
    }
    let (status, error) = request(
        &router,
        "POST",
        &format!("{base}/surface/open"),
        json!({
            "plugin_id":id,"agent_session":{"agent_session_id":uuid::Uuid::now_v7().to_string(),
            "expected_release_digest":plugin["releases"]["active"]["release_digest"]}
        }),
    )
    .await;
    assert!(!status.is_success(), "{error}");
    assert!(error.to_string().contains("unsupported"));
    let still_open = post(
        &router,
        &format!("{base}/surface/open"),
        json!({"plugin_id":id}),
    )
    .await;
    assert!(still_open["surface_capability"].is_string());
    services
        .plugin_runtime
        .shutdown_service_runtime(services.authoritative_user_id.as_ref())
        .await
        .unwrap();
}

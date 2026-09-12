//! Route-topology regression guard for the current Nomi-core composition.
//!
//! Route-topology and smoke guards for the current Nomi-core composition.
//!
//! The default router must expose the app-local Agent Settings/AgentSession/
//! Remote adapter while keeping the Fresh-v4/Codex router out of the product
//! path.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::fs;
use std::path::PathBuf;
use serde_json::{Value, json};
use tower::ServiceExt;

fn repo_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(relative)
        .canonicalize()
        .unwrap_or_else(|error| panic!("cannot resolve {relative}: {error}"));
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

#[test]
fn default_nomi_core_router_keeps_legacy_and_execution_surfaces_explicit() {
    let routes = repo_file("src/router/routes.rs");

    for required_merge in [
        ".merge(agent_authenticated)",
        ".merge(agent_execution_authenticated)",
        ".merge(agent_execution_template_authenticated)",
    ] {
        assert!(
            routes.contains(required_merge),
            "Nomi-core router must retain {required_merge}"
        );
    }

    assert!(
        routes.contains("build_nomi_core_agent_router("),
        "default Nomi-core router must mount the app-local Agent adapter"
    );
    assert!(
        routes.contains(".nest(\"/mcp\", nomi_core_remote_mcp)"),
        "default Nomi-core router must mount the canonical Remote MCP transport"
    );
    assert!(
        !routes.contains("remote_rest::build("),
        "default Nomi-core router must not mount the Fresh-v4 Remote adapter"
    );
}

#[test]
fn canonical_surfaces_are_traceable_to_the_app_local_route_definitions() {
    let control_plane = repo_file("../nomifun-agent-control-plane/src/routes.rs");
    let session = repo_file("src/router/nomi_core_session.rs");

    // These are the exact route groups consumed by ui/src/common/adapter/
    // ipcBridge.ts and must remain in the current Nomi-core adapter.
    for path in [
        "/api/agent-preset-templates",
        "/api/capabilities",
    ] {
        assert!(control_plane.contains(path), "control-plane definition must remain discoverable for {path}");
        assert!(
            session.contains("build_nomi_core_agent_router"),
            "Nomi-core route adapter must be mounted for {path}"
        );
    }
    for path in [
        "/api/agent-sessions",
        "/api/agent-sessions/{agent_session_id}",
        "/api/agent-sessions/{agent_session_id}/preset",
        "/api/remote/open",
        "/api/remote/turn",
        "/api/remote/observe",
        "/api/remote/cancel",
    ] {
        assert!(session.contains(path), "Nomi-core adapter must define {path}");
    }
}

#[test]
fn current_nomi_core_projection_is_not_a_runtime_or_route_authority() {
    let projection = repo_file("src/router/nomi_core_agent_projection.rs");

    assert!(
        projection.contains("pub fn project("),
        "the Nomi-core projection seam must remain available for the future adapter"
    );
    for forbidden in [
        "AgentPlatform::from_pool",
        "AgentSessionStore",
        "CodexRuntimeSupervisor",
        "RemoteRuntimeCoordinator",
        "Router::new()",
    ] {
        assert!(
            !projection.contains(forbidden),
            "projection seam must not become a second runtime/router authority: {forbidden}"
        );
    }
}

#[test]
fn nomi_core_projection_does_not_reintroduce_preset_resource_fields() {
    let projection = repo_file("src/router/nomi_core_agent_projection.rs");
    assert!(
        !projection.contains("payload.resource_bindings"),
        "Preset Revision projection must not own concrete resource bindings"
    );
    assert!(
        !projection.contains("resource_binding_refs"),
        "Capability selections must not carry target resource references"
    );
}

#[path = "common/mod.rs"]
mod common;

#[allow(dead_code)]
#[path = "../src/router/nomi_core_agent_projection.rs"]
mod projection;

#[tokio::test]
async fn default_nomi_core_router_answers_canonical_catalog_requests() {
    let (router, services) = common::build_local_trust_app("route-gap-local-trust").await;
    for path in [
        "/api/agent-preset-templates?source=official",
        "/api/capabilities",
        "/api/agent-catalog/skills",
        "/api/mcp-tool-mappings",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("x-nomi-local-trust", "route-gap-local-trust")
                    .body(Body::empty())
                    .expect("build catalog request"),
            )
            .await
            .expect("dispatch catalog request");
        assert_eq!(response.status(), StatusCode::OK, "catalog route {path}");
        let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read catalog response");
        let value: Value = serde_json::from_slice(&body).expect("catalog response JSON");
        assert!(value.get("data").is_some(), "catalog response must be wrapped: {path}");
    }
    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn installation_token_is_limited_to_headless_product_control_planes() {
    let trust_secret = "headless-product-local-trust";
    let installation_token = "headless-product-installation-token";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    services
        .instance_token_validator
        .set_token(nomifun_auth::token_sha256_hex(installation_token));

    for path in [
        "/api/javascript-runtime/status",
        "/api/plugins",
        "/api/miniapps",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(
                        "authorization",
                        format!("Bearer {installation_token}"),
                    )
                    .body(Body::empty())
                    .expect("build installation-token request"),
            )
            .await
            .expect("dispatch installation-token request");
        assert_eq!(response.status(), StatusCode::OK, "product route {path}");
    }

    let mutation = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/plugin-projects")
                .header(
                    "authorization",
                    format!("Bearer {installation_token}"),
                )
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("build installation-token mutation"),
        )
        .await
        .expect("dispatch installation-token mutation");
    assert_eq!(
        mutation.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Bearer mutation must pass CSRF/auth and reach JSON validation"
    );

    let owner_jwt = services
        .jwt_service
        .sign(services.authoritative_user_id.as_ref(), "owner")
        .expect("sign owner JWT");
    for (path, token) in [
        ("/api/plugins", "wrong-installation-token"),
        ("/api/plugins", owner_jwt.as_str()),
        ("/api/capabilities", installation_token),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .expect("build rejected product request"),
            )
            .await
            .expect("dispatch rejected product request");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "route {path}");
    }

    services
        .shutdown_browser_platform()
        .await
        .expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_remote_preserves_owner_jwt_and_rejects_selector_queries() {
    let trust_secret = "remote-jwt-query-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    let jwt = services
        .jwt_service
        .sign(services.authoritative_user_id.as_ref(), "admin")
        .expect("sign installation owner JWT");

    let jwt_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .expect("build JWT Remote request"),
        )
        .await
        .expect("dispatch JWT Remote request");
    assert_eq!(
        jwt_response.status(),
        StatusCode::NOT_FOUND,
        "the installation owner's JWT must still reach the owner-scoped Remote handler"
    );

    for uri in [
        "/api/remote/open?profile=agent",
        "/api/remote/turn?domains=agent",
        "/api/remote/cancel?selector=latest",
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("x-nomi-local-trust", trust_secret)
                    .body(Body::empty())
                    .expect("build selector query request"),
            )
            .await
            .expect("dispatch selector query request");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read selector query response");
        let value: Value = serde_json::from_slice(&body).expect("selector query JSON");
        assert_eq!(value["code"], "REMOTE_INVALID_REQUEST", "{uri}");
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_catalog_exposes_native_nomi_capabilities() {
    let (router, services) = common::build_local_trust_app("catalog-placement-local-trust").await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/capabilities")
                .header("x-nomi-local-trust", "catalog-placement-local-trust")
                .body(Body::empty())
                .expect("build capability catalog request"),
        )
        .await
        .expect("dispatch capability catalog request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read capability catalog response");
    let value: Value = serde_json::from_slice(&body).expect("capability catalog JSON");
    let capabilities = value["data"]
        .as_array()
        .expect("capability catalog array");

    for capability_id in ["fs.read", "vcs.stage", "vcs.commit"] {
        let capability = capabilities
            .iter()
            .find(|item| item["capability"]["id"] == capability_id)
            .unwrap_or_else(|| panic!("missing capability {capability_id}"));
        assert_eq!(
            capability["materialization_state"],
            "materialized",
            "{capability_id} must remain selectable as an initial capability"
        );
        assert_eq!(
            capability["required_resource_kinds"].as_array().map(Vec::len),
            Some(1),
            "{capability_id} must retain its target resource-kind requirement"
        );
    }
    for capability_id in [
        "web.fetch",
        "agent.delegate",
        "agent.execution.observe",
        "agent.execution.steer",
        "agent.fork",
        "schedule.store",
    ] {
        let capability = capabilities.iter()
            .find(|item| item["capability"]["id"] == capability_id)
            .unwrap_or_else(|| panic!("missing repaired builtin {capability_id}"));
        assert_eq!(capability["materialization_state"], "materialized", "{capability_id}");
        assert!(capability["unavailable_code"].is_null(), "{capability_id}");
        assert_eq!(capability["source_kind"], "bundled");
    }
    let browser = capabilities
        .iter()
        .find(|item| item["capability"]["id"] == "browser.navigate")
        .expect("browser capability remains visible in the shared catalog");
    let expected_browser_state = if cfg!(feature = "browser-use") {
        "materialized"
    } else {
        "unavailable"
    };
    assert_eq!(
        browser["materialization_state"],
        expected_browser_state,
        "Browser availability must match whether this host compiled the Nomi Browser owner"
    );
    if cfg!(feature = "browser-use") {
        assert!(browser["unavailable_code"].is_null());
    } else {
        assert_eq!(browser["unavailable_code"], "CAPABILITY_UNAVAILABLE");
    }

    let templates = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agent-preset-templates?source=official")
                .header("x-nomi-local-trust", "catalog-placement-local-trust")
                .body(Body::empty())
                .expect("build official template request"),
        )
        .await
        .expect("dispatch official template request");
    assert_eq!(templates.status(), StatusCode::OK);
    let template_body = axum::body::to_bytes(templates.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read official template response");
    let template_value: Value =
        serde_json::from_slice(&template_body).expect("official template JSON");
    let coding = template_value["data"]["official_templates"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["template_key"] == "coding.codex")
        })
        .expect("coding template");
    let on_demand = coding["seed"]["on_demand_capabilities"]
        .as_array()
        .expect("coding on-demand capabilities");
    for capability_id in ["vcs.stage", "vcs.commit"] {
        assert!(
            on_demand
                .iter()
                .any(|item| item["id"] == capability_id),
            "{capability_id} must remain an on-demand placement in the coding seed"
        );
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_accepts_on_demand_placement_without_widening_initial_tools() {
    let trust_secret = "on-demand-placement-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;
    let created = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", trust_secret)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "display_name": "On-demand placement smoke",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize placement template request"),
                ))
                .expect("build placement template request"),
        )
        .await
        .expect("dispatch placement template request");
    assert_eq!(created.status(), StatusCode::OK);
    let created_body = axum::body::to_bytes(created.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read placement template response");
    let mut created_value: Value =
        serde_json::from_slice(&created_body).expect("placement template JSON");
    let preset_id = created_value["data"]["preset"]["preset_id"]
        .as_str()
        .expect("placement preset id")
        .to_owned();
    let revision = created_value["data"]["revision"]["reference"].clone();
    let deferred_ids = [
        "vcs.stage",
        "web.fetch",
        "agent.delegate",
        "agent.execution.observe",
        "agent.execution.steer",
        "agent.fork",
        "schedule.store",
    ];
    created_value["data"]["draft"]["document"]["on_demand_capabilities"] = json!(
        deferred_ids.iter().map(|id| json!({
            "capability": {"id": id, "version": "1.0.0"},
            "action_allowlist": []
        })).collect::<Vec<_>>()
    );
    let preview = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-presets/{preset_id}/resolve-preview"))
                .header("x-nomi-local-trust", trust_secret)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "expected_current_revision": revision,
                        "draft": created_value["data"]["draft"],
                        "scene": "agent_settings",
                        "surface": "desktop",
                        "audience": "owner"
                    }))
                    .expect("serialize on-demand preview request"),
                ))
                .expect("build on-demand preview request"),
        )
        .await
        .expect("dispatch on-demand preview request");
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_body = axum::body::to_bytes(preview.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read on-demand preview response");
    let preview_value: Value = serde_json::from_slice(&preview_body).expect("preview JSON");
    assert_eq!(preview_value["data"]["status"], "ready");
    assert!(
        preview_value["data"]["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.is_empty()),
        "a supported on-demand capability must produce a clean preview"
    );
    assert_eq!(
        preview_value["data"]["summary"]["initial_count"],
        0,
        "moving a capability to on-demand must remove it from the initial set"
    );
    assert_eq!(
        preview_value["data"]["summary"]["on_demand_count"],
        deferred_ids.len(),
        "the preview must retain the on-demand placement"
    );
    for id in deferred_ids {
        assert!(
            preview_value["data"]["inspector"]["on_demand_capabilities"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["capability"]["id"] == id)),
            "the immutable preview must expose deferred capability {id}"
        );
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn configured_agent_creation_persists_adjusted_capabilities_and_keeps_official_seeds() {
    const TRUST: &str = "configured-agent-workbench-test";
    async fn call(router: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (_, before) = call(router.clone(), "GET", "/api/agent-preset-templates?source=official", json!({})).await;
    let before_count = before["data"]["user_presets"].as_array().unwrap().len();
    let official_before = before["data"]["official_templates"].clone();
    let document = json!({
        "schema_version":"1.0.0", "model_route_refs":{}, "chat_route_records":{},
        "initial_capabilities":[{"capability":{"id":"fs.read","version":"1.0.0"}}],
        "on_demand_capabilities":[{"capability":{"id":"web.fetch","version":"1.0.0"}},{"capability":{"id":"agent.execution.plan","version":"1.0.0"}}],
        "skill_bindings":[], "system_role_provider_overrides":{}, "persona":"Research helper",
        "instructions":"Use only the selected capabilities.", "starter_prompts":[]
    });
    let (status, created) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"My adjusted assistant", "description":"Custom capability scope", "document":document,
    })).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["data"]["preset"]["source"], "user");
    assert_eq!(created["data"]["revision"]["reference"]["revision"], 1);
    assert!(created["data"]["draft"]["source_template_key"].is_null());
    let id = created["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, reloaded) = call(router.clone(), "GET", &format!("/api/agent-presets/{id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reloaded["data"]["draft"]["document"]["initial_capabilities"], created["data"]["revision"]["document"]["initial_capabilities"]);
    assert_eq!(reloaded["data"]["draft"]["document"]["on_demand_capabilities"], created["data"]["revision"]["document"]["on_demand_capabilities"]);
    assert_eq!(reloaded["data"]["draft"]["document"]["initial_capabilities"].as_array().unwrap().len(), 1);
    assert_eq!(reloaded["data"]["draft"]["document"]["on_demand_capabilities"].as_array().unwrap().len(), 2);
    let mut invalid = document;
    invalid["initial_capabilities"] = json!([{"capability":{"id":"missing.capability","version":"1.0.0"}}]);
    let (status, _) = call(router.clone(), "POST", "/api/agent-presets", json!({"display_name":"Must not persist", "document":invalid})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, after) = call(router, "GET", "/api/agent-preset-templates?source=official", json!({})).await;
    assert_eq!(after["data"]["user_presets"].as_array().unwrap().len(), before_count + 1);
    assert_eq!(after["data"]["official_templates"], official_before);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn official_agent_direct_launch_reuses_configuration_and_creates_sessions() {
    const TRUST: &str = "official-agent-direct-launch";
    async fn call(router: axum::Router, path: &str, body: Value) -> Value {
        let response = router.oneshot(Request::builder().method("POST").uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status, StatusCode::OK, "{path}: {value}");
        value["data"].clone()
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let create = json!({ "display_name": "Minimal", "model_route_refs": {}, "chat_route_records": {}, "reuse_existing": true });
    let first = call(router.clone(), "/api/agent-presets/from-template/chat.minimal", create.clone()).await;
    let second = call(router.clone(), "/api/agent-presets/from-template/chat.minimal", create).await;
    assert_eq!(first["preset"]["preset_id"], second["preset"]["preset_id"]);
    assert_eq!(first["revision"]["reference"], second["revision"]["reference"]);
    let preset_id = first["preset"]["preset_id"].as_str().unwrap();
    let session_a = call(router.clone(), "/api/agent-sessions", json!({ "preset_id": preset_id, "title": "First conversation" })).await;
    let session_b = call(router.clone(), "/api/agent-sessions", json!({ "preset_id": preset_id, "title": "Second conversation" })).await;
    assert_ne!(session_a["agent_session_id"], session_b["agent_session_id"]);
    assert_eq!(session_a["agent_binding"], session_b["agent_binding"]);
    assert_eq!(session_a["agent_binding"]["preset_revision_ref"], first["revision"]["reference"]);
    let persisted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nomi_agent_preset_revisions WHERE preset_id = ?")
        .bind(preset_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(persisted, 1);
    let library_response = router.oneshot(Request::builder()
        .uri("/api/agent-preset-templates")
        .header("x-nomi-local-trust", TRUST)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(library_response.status(), StatusCode::OK);
    let library: Value = serde_json::from_slice(&axum::body::to_bytes(
        library_response.into_body(), 4 * 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(library["data"]["user_presets"], json!([]),
        "using an official Agent must not create personal Agents");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn agent_session_model_selection_is_exact_persistent_and_keeps_the_agent_unchanged() {
    const TRUST: &str = "agent-model-selection-test";
    async fn call(router: axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = router.oneshot(Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (status, original) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name": "Personal model test", "document": {
            "schema_version": "1.0.0", "model_route_refs": {}, "chat_route_records": {},
            "initial_capabilities": [{ "capability": { "id": "fs.read", "version": "1.0.0" } }],
            "on_demand_capabilities": [], "skill_bindings": [], "system_role_provider_overrides": {},
            "persona": "Research helper", "instructions": "Keep my working rules", "starter_prompts": []
        }
    })).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    let original = original["data"].clone();
    let preset_id = original["preset"]["preset_id"].as_str().unwrap();
    let alternate = original["revision"]["document"]["chat_route_records"]["agent_chat"]["failovers"][0].clone();
    assert!(alternate["model"].is_string());
    let selection = json!({ "provider_id": alternate["provider_id"], "model": alternate["model"] });
    let mut sessions = Vec::new();
    for _ in 0..2 {
        let (status, result) = call(router.clone(), "POST", "/api/agent-sessions", json!({
            "preset_id": preset_id, "title": "Chosen model", "model": selection
        })).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let id = result["data"]["agent_session_id"].as_str().unwrap();
        let (status, conversation) = call(router.clone(), "GET", &format!("/api/conversations/{id}"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(conversation["data"]["model"]["provider_id"], selection["provider_id"]);
        assert_eq!(conversation["data"]["model"]["model"], selection["model"]);
        let binding = result["data"]["agent_binding"].clone();
        let variant = binding["preset_revision_ref"]["preset_id"].as_str().unwrap();
        let (status, editor) = call(router.clone(), "GET", &format!("/api/agent-presets/{variant}/editor"), json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let record = &editor["data"]["revision"]["document"]["chat_route_records"]["agent_chat"];
        assert_eq!(record["primary"]["provider_id"], selection["provider_id"]);
        assert_eq!(record["primary"]["model"], selection["model"]);
        assert_eq!(record["failovers"], json!([]));
        for field in ["initial_capabilities", "on_demand_capabilities", "skill_bindings", "system_role_provider_overrides", "persona", "instructions"] {
            assert_eq!(editor["data"]["revision"]["document"][field], original["revision"]["document"][field], "must retain {field}");
        }
        sessions.push(binding);
    }
    assert_eq!(sessions[0], sessions[1], "same model reuses the frozen configuration");
    let (_, reloaded) = call(router.clone(), "GET", &format!("/api/agent-presets/{preset_id}/editor"), json!({})).await;
    assert_eq!(reloaded["data"]["revision"], original["revision"]);
    let (_, library) = call(router.clone(), "GET", "/api/agent-preset-templates", json!({})).await;
    assert_eq!(library["data"]["user_presets"].as_array().unwrap().len(), 1, "internal model variants stay out of the Agent library");
    let (status, official) = call(router.clone(), "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "display_name": "Official chosen model", "model_route_refs": {}, "chat_route_records": {},
        "model": selection, "reuse_existing": true
    })).await;
    assert_eq!(status, StatusCode::OK, "{official}");
    assert_eq!(official["data"]["revision"]["document"]["chat_route_records"]["agent_chat"]["primary"]["model"], selection["model"]);
    let (status, _) = call(router, "POST", "/api/agent-sessions", json!({
        "preset_id": preset_id, "model": { "provider_id": selection["provider_id"], "model": "nonexistent-model" }
    })).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "an unknown model cannot silently fall back");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_agent_settings_template_and_binding_surface_is_persistent() {
    let (router, services) = common::build_local_trust_app("agent-settings-local-trust").await;
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "display_name": "Nomi-core smoke preset",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize template request"),
                ))
                .expect("build template request"),
        )
        .await
        .expect("dispatch template request");
    let response_status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read template response");
    let value: Value = serde_json::from_slice(&body).expect("template response JSON");
    assert_eq!(
        response_status,
        StatusCode::OK,
        "template request failed: {value}"
    );
    let preset_id = value["data"]["preset"]["preset_id"]
        .as_str()
        .expect("template response preset id")
        .to_owned();
    let persisted_payload: String = sqlx::query_scalar(
        "SELECT payload_json FROM nomi_agent_preset_revisions \
         WHERE preset_id = ? AND revision_no = 1",
    )
    .bind(&preset_id)
    .fetch_one(services.database.pool())
    .await
    .expect("persisted AgentPreset revision payload");
    assert_eq!(
        serde_json::from_str::<Value>(&persisted_payload).expect("persisted payload JSON"),
        value["data"]["revision"]["document"],
        "Nomi-core must persist the canonical revision payload in payload_json"
    );
    assert_eq!(
        value["data"]["revision"]["reference"]["revision"],
        1,
        "template creation must persist its first immutable revision"
    );
    let route = &value["data"]["revision"]["document"]["chat_route_records"]["agent_chat"];
    assert_eq!(
        route["schema"],
        "nomifun.chat-route-record.v1",
        "the host must materialize a canonical Chat route record"
    );
    assert_eq!(
        route["primary"]["model"],
        "big-pickle",
        "the managed Chat catalog's stable first candidate should be selected"
    );
    assert!(
        route["failovers"].as_array().is_some_and(|items| !items.is_empty()),
        "the host route should expose the remaining enabled Chat models as failovers"
    );
    assert!(
        route["primary"]["credential_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("nomi-core-chat-credential-")),
        "the route must carry an opaque credential reference, never credential material"
    );
    assert!(
        route["primary"]["credential_ref"]
            .as_str()
            .is_some_and(|value| !value.contains("test-only")),
        "credential material must not cross the route record boundary"
    );
    let editor = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-presets/{preset_id}/editor"))
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build editor request"),
        )
        .await
        .expect("dispatch editor request");
    assert_eq!(editor.status(), StatusCode::OK);
    let editor_body = axum::body::to_bytes(editor.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read editor response");
    let editor_value: Value = serde_json::from_slice(&editor_body).expect("editor JSON");
    assert_eq!(
        editor_value["data"]["preset"]["preset_id"],
        preset_id,
        "editor must reload the same persisted Preset"
    );
    let binding_json = json!({
        "preset_revision_ref": { "preset_id": preset_id }
    })
    .to_string();
    sqlx::query(
        "INSERT INTO nomi_agent_bindings \
         (target_kind, target_id, owner_user_id, agent_binding_json) \
         VALUES ('conversation', 'retirement-target', ?, ?)",
    )
    .bind(services.authoritative_user_id.as_ref())
    .bind(&binding_json)
    .execute(services.database.pool())
    .await
    .expect("insert active AgentBinding");
    sqlx::query(
        "INSERT INTO remote_bindings \
         (remote_binding_id, owner_user_id, name, agent_binding_json, nomi_snapshot_json, \
          provenance_json, agent_binding_digest, binding_version, created_at, updated_at) \
         VALUES ('0190f5fe-7c00-7a00-8000-000000000099', ?, 'Retirement Remote', ?, \
                 '{}', '{}', ?, 1, 1, 1)",
    )
    .bind(services.authoritative_user_id.as_ref())
    .bind(&binding_json)
    .bind("d".repeat(64))
    .execute(services.database.pool())
    .await
    .expect("insert active RemoteBinding");

    let retired = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/agent-presets/{preset_id}"))
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build Preset retirement request"),
        )
        .await
        .expect("dispatch Preset retirement request");
    assert_eq!(retired.status(), StatusCode::OK);
    let retired_at_ms: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(&preset_id)
    .fetch_one(services.database.pool())
    .await
    .expect("retired AgentPreset row");
    assert!(retired_at_ms.is_some());
    let revision_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM nomi_agent_preset_revisions WHERE preset_id = ?",
    )
    .bind(&preset_id)
    .fetch_one(services.database.pool())
    .await
    .expect("retained AgentPreset revisions");
    assert_eq!(revision_count, 1);
    let agent_binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM nomi_agent_bindings")
            .fetch_one(services.database.pool())
            .await
            .expect("count active AgentBindings");
    let remote_binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM remote_bindings")
            .fetch_one(services.database.pool())
            .await
            .expect("count active RemoteBindings");
    assert_eq!(agent_binding_count, 0);
    assert_eq!(remote_binding_count, 0);

    let library = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/agent-preset-templates?source=official")
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build post-retirement library request"),
        )
        .await
        .expect("dispatch post-retirement library request");
    assert_eq!(library.status(), StatusCode::OK);
    let library_body = axum::body::to_bytes(library.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read post-retirement library");
    let library_value: Value =
        serde_json::from_slice(&library_body).expect("post-retirement library JSON");
    assert!(
        library_value["data"]["user_presets"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    let retired_editor = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-presets/{preset_id}/editor"))
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .body(Body::empty())
                .expect("build retired editor request"),
        )
        .await
        .expect("dispatch retired editor request");
    assert_eq!(retired_editor.status(), StatusCode::NOT_FOUND);
    let retired_editor_body =
        axum::body::to_bytes(retired_editor.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read retired editor response");
    let retired_editor_error: Value =
        serde_json::from_slice(&retired_editor_body).expect("retired editor error JSON");
    assert_eq!(retired_editor_error["code"], "AGENT_PRESET_NOT_FOUND");

    let retired_session = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-settings-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "preset_id": preset_id,
                        "title": "Must not start"
                    }))
                    .expect("serialize retired Session request"),
                ))
                .expect("build retired Session request"),
        )
        .await
        .expect("dispatch retired Session request");
    assert_eq!(retired_session.status(), StatusCode::NOT_FOUND);
    let retired_session_body =
        axum::body::to_bytes(retired_session.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read retired Session response");
    let retired_session_error: Value =
        serde_json::from_slice(&retired_session_body).expect("retired Session error JSON");
    assert_eq!(retired_session_error["code"], "AGENT_PRESET_NOT_FOUND");

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_agent_session_projects_saved_chat_binding_without_internal_inputs() {
    let (router, services) = common::build_local_trust_app("agent-session-local-trust").await;
    let template_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "display_name": "Nomi-core session smoke",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize session template request"),
                ))
                .expect("build session template request"),
        )
        .await
        .expect("dispatch session template request");
    assert_eq!(template_response.status(), StatusCode::OK);
    let template_body = axum::body::to_bytes(template_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session template response");
    let template: Value = serde_json::from_slice(&template_body).expect("session template JSON");
    let preset_id = template["data"]["preset"]["preset_id"]
        .as_str()
        .expect("session preset id");
    let revision = template["data"]["revision"]["reference"].clone();
    let draft = template["data"]["draft"].clone();

    let preview_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-presets/{preset_id}/resolve-preview"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "expected_current_revision": revision,
                        "draft": draft,
                        "scene": "agent_session",
                        "surface": "desktop",
                        "audience": "owner"
                    }))
                    .expect("serialize session preview request"),
                ))
                .expect("build session preview request"),
        )
        .await
        .expect("dispatch session preview request");
    assert_eq!(preview_response.status(), StatusCode::OK);
    let preview_body = axum::body::to_bytes(preview_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session preview response");
    let preview: Value = serde_json::from_slice(&preview_body).expect("session preview JSON");
    assert_eq!(preview["data"]["status"], "ready");
    let expected_snapshot = preview["data"]["resolved_snapshot_ref"].clone();

    let rejected = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "preset_id": preset_id,
                        "agent_binding": {
                            "preset_revision_ref": revision,
                            "resolved_snapshot_ref": expected_snapshot,
                            "typed_resource_bindings": [],
                            "binding_version": 1
                        }
                    }))
                    .expect("serialize rejected session request"),
                ))
                .expect("build rejected session request"),
        )
        .await
        .expect("dispatch rejected session request");
    assert_eq!(
        rejected.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "deny_unknown_fields must reject the removed agent_binding input"
    );

    let session_response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .header("idempotency-key", "agent-session-create-smoke")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "preset_id": preset_id,
                        "title": "Session smoke"
                    }))
                    .expect("serialize session create request"),
                ))
                .expect("build session create request"),
        )
        .await
        .expect("dispatch session create request");
    let session_status = session_response.status();
    let session_body = axum::body::to_bytes(session_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session response");
    let session: Value = serde_json::from_slice(&session_body).expect("session JSON");
    assert_eq!(
        session_status,
        StatusCode::OK,
        "saved Chat binding should project into Nomi-core Session: {session}"
    );
    let session_id = session["data"]["agent_session_id"]
        .as_str()
        .expect("session id");
    assert_eq!(session["data"]["state"], "ready");
    assert_eq!(session["data"]["agent_binding"]["binding_version"], 1);

    let get_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{session_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build session get request"),
        )
        .await
        .expect("dispatch session get request");
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = axum::body::to_bytes(get_response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read session get response");
    let observation: Value = serde_json::from_slice(&get_body).expect("session observation JSON");
    assert_eq!(
        observation["data"]["session"]["owner_ref"]["principal_id"],
        services.authoritative_user_id.as_ref()
    );
    assert_eq!(
        observation["data"]["session"]["agent_binding"]["resolved_snapshot_ref"],
        expected_snapshot
    );

    let fork_body = json!({
        "target_agent_binding": session["data"]["agent_binding"],
        "parent_through_seq": 0,
        "title": "Session smoke child"
    });
    let fork_once = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-sessions/{session_id}/forks"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .header("idempotency-key", "agent-session-fork-smoke")
                .body(Body::from(serde_json::to_vec(&fork_body).unwrap()))
                .expect("build Session fork request"),
        )
        .await
        .expect("dispatch Session fork request");
    assert_eq!(fork_once.status(), StatusCode::OK);
    let fork_once: Value = serde_json::from_slice(
        &axum::body::to_bytes(fork_once.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read Session fork response"),
    )
    .expect("Session fork JSON");
    let child_session_id = fork_once["data"]["child_agent_session_id"]
        .as_str()
        .expect("child Session id");
    assert_ne!(child_session_id, session_id);
    assert_eq!(fork_once["data"]["parent_agent_session_id"], session_id);
    assert_eq!(fork_once["data"]["child_base_is_self_contained"], true);
    assert_eq!(fork_once["data"]["migrates_runtime_private_handles"], false);
    assert_eq!(fork_once["data"]["replays_tool_or_effect"], false);

    let child_response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{child_session_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build child Session observation request"),
        )
        .await
        .expect("dispatch child Session observation request");
    assert_eq!(child_response.status(), StatusCode::OK);
    let child: Value = serde_json::from_slice(
        &axum::body::to_bytes(child_response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read child Session observation"),
    )
    .expect("child Session observation JSON");
    assert_eq!(
        child["data"]["session"]["parent_session_id"],
        session_id,
        "the child must record the exact host-owned parent"
    );
    assert!(
        child["data"]["session"]["fork_base_payload_id"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    let retired = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/agent-presets/{preset_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build post-Session Preset retirement request"),
        )
        .await
        .expect("dispatch post-Session Preset retirement request");
    assert_eq!(retired.status(), StatusCode::OK);

    let capabilities = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{session_id}/capabilities"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build retired-Preset Session capability request"),
        )
        .await
        .expect("dispatch retired-Preset Session capability request");
    assert_eq!(
        capabilities.status(),
        StatusCode::OK,
        "an existing Session must keep its frozen capability view after Preset retirement"
    );
    let capabilities_body =
        axum::body::to_bytes(capabilities.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read retired-Preset Session capabilities");
    let capabilities_value: Value =
        serde_json::from_slice(&capabilities_body).expect("Session capabilities JSON");
    assert_eq!(
        capabilities_value["data"]["resolved_snapshot_ref"],
        expected_snapshot
    );

    let historical = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/agent-sessions/{session_id}"))
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .body(Body::empty())
                .expect("build retired-Preset historical Session request"),
        )
        .await
        .expect("dispatch retired-Preset historical Session request");
    assert_eq!(historical.status(), StatusCode::OK);

    let new_session = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-sessions")
                .header("x-nomi-local-trust", "agent-session-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "preset_id": preset_id,
                        "title": "Retired Preset must not launch"
                    }))
                    .expect("serialize retired Preset Session request"),
                ))
                .expect("build retired Preset Session request"),
        )
        .await
        .expect("dispatch retired Preset Session request");
    assert_eq!(new_session.status(), StatusCode::NOT_FOUND);

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_remote_replays_frozen_binding_and_persists_event_cursor() {
    let (router, services) = common::build_local_trust_app("remote-local-trust").await;
    let create_preset = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent-presets/from-template/chat.minimal")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "display_name": "Nomi-core remote smoke",
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize remote preset request"),
                ))
                .expect("build remote preset request"),
        )
        .await
        .expect("dispatch remote preset request");
    assert_eq!(create_preset.status(), StatusCode::OK);
    let preset_body = axum::body::to_bytes(create_preset.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote preset response");
    let preset: Value = serde_json::from_slice(&preset_body).expect("remote preset JSON");
    // The creation response does not include a Preview object. Resolve the
    // saved revision once through the canonical API to obtain the exact
    // Snapshot reference used by RemoteBinding.
    let preset_id = preset["data"]["preset"]["preset_id"]
        .as_str()
        .expect("remote preset id");
    let revision = preset["data"]["revision"]["reference"].clone();
    let editor_draft = preset["data"]["draft"].clone();
    let preview = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent-presets/{preset_id}/resolve-preview"))
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "expected_current_revision": revision,
                        "draft": editor_draft,
                        "scene": "remote",
                        "surface": "remote",
                        "audience": "owner"
                    }))
                    .expect("serialize remote preview request"),
                ))
                .expect("build remote preview request"),
        )
        .await
        .expect("dispatch remote preview request");
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_body = axum::body::to_bytes(preview.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote preview response");
    let preview: Value = serde_json::from_slice(&preview_body).expect("remote preview JSON");
    let binding = serde_json::json!({
        "preset_revision_ref": revision,
        "resolved_snapshot_ref": preview["data"]["resolved_snapshot_ref"],
        "typed_resource_bindings": [],
        "binding_version": 1
    });

    let create_binding = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote-bindings")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "name": "Nomi-core remote smoke binding",
                        "agent_binding": binding
                    }))
                    .expect("serialize remote binding request"),
                ))
                .expect("build remote binding request"),
        )
        .await
        .expect("dispatch remote binding request");
    assert_eq!(create_binding.status(), StatusCode::OK);
    let binding_body = axum::body::to_bytes(create_binding.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote binding response");
    let binding_response: Value =
        serde_json::from_slice(&binding_body).expect("remote binding JSON");
    let binding_id = binding_response["data"]["remote_binding_id"]
        .as_str()
        .expect("remote binding id")
        .to_owned();

    let open_request = || {
        Request::builder()
            .method("POST")
            .uri("/api/remote/open")
            .header("x-nomi-local-trust", "remote-local-trust")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "binding_id": binding_id,
                    "idempotency_key": "remote-open-smoke"
                }))
                .expect("serialize remote open request"),
            ))
            .expect("build remote open request")
    };
    let open = router.clone().oneshot(open_request()).await.expect("open remote");
    assert_eq!(open.status(), StatusCode::OK);
    let open_body = axum::body::to_bytes(open.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote open response");
    let opened: Value = serde_json::from_slice(&open_body).expect("remote open JSON");
    assert_eq!(opened["open_state"], json!({ "state": "ready" }));
    assert_eq!(
        opened["cursor"]["seq"],
        2,
        "Remote open cursor must be the Remote event cursor, not the Conversation message cursor"
    );
    let session_id = opened["agent_session_id"]
        .as_str()
        .expect("remote session id")
        .to_owned();

    let replay = router.clone().oneshot(open_request()).await.expect("replay remote open");
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = axum::body::to_bytes(replay.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read replay response");
    let replayed: Value = serde_json::from_slice(&replay_body).expect("replay JSON");
    assert_eq!(replayed["agent_session_id"], session_id);
    assert_eq!(replayed["agent_binding"], opened["agent_binding"]);
    assert_eq!(
        replayed["cursor"]["seq"],
        opened["cursor"]["seq"],
        "idempotent Remote open must replay the same event cursor"
    );

    let observe = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/remote/observe?agent_session_id={session_id}&after_seq=0&limit=100"
                ))
                .header("x-nomi-local-trust", "remote-local-trust")
                .body(Body::empty())
                .expect("build remote observe request"),
        )
        .await
        .expect("observe remote");
    assert_eq!(observe.status(), StatusCode::OK);
    let observe_body = axum::body::to_bytes(observe.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote observe response");
    let observed: Value = serde_json::from_slice(&observe_body).expect("remote observe JSON");
    let events = observed["events"].as_array().expect("remote event array");
    assert!(events.iter().any(|event| event["kind"] == "session/opening"));
    assert!(events.iter().any(|event| event["kind"] == "session/ready"));
    let cursor = observed["next_cursor"]["seq"]
        .as_u64()
        .expect("remote next cursor");
    assert!(cursor >= 2);

    // Deleting the mutable binding does not invalidate the already-open
    // Session. Replaying the same open key must use the frozen projection.
    let delete_binding = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/remote-bindings/{binding_id}"))
                .header("x-nomi-local-trust", "remote-local-trust")
                .body(Body::empty())
                .expect("build remote binding delete request"),
        )
        .await
        .expect("delete remote binding");
    assert_eq!(delete_binding.status(), StatusCode::OK);
    let replay_after_delete = router
        .clone()
        .oneshot(open_request())
        .await
        .expect("replay remote open after binding delete");
    assert_eq!(replay_after_delete.status(), StatusCode::OK);
    let replay_after_delete_body =
        axum::body::to_bytes(replay_after_delete.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read replay after delete response");
    let replay_after_delete: Value =
        serde_json::from_slice(&replay_after_delete_body).expect("replay after delete JSON");
    assert_eq!(replay_after_delete["agent_session_id"], session_id);

    let cancel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote/cancel")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_session_id": session_id,
                        "idempotency_key": "remote-cancel-smoke"
                    }))
                    .expect("serialize remote cancel request"),
                ))
                .expect("build remote cancel request"),
        )
        .await
        .expect("cancel remote");
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancel_body = axum::body::to_bytes(cancel.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read remote cancel response");
    let cancelled: Value = serde_json::from_slice(&cancel_body).expect("remote cancel JSON");
    assert_eq!(cancelled["session_status"], "cancelled");
    assert!(
        cancelled["cursor"]["seq"].as_u64().is_some_and(|seq| seq >= 4),
        "Remote cancel cursor must include both cancellation facts"
    );

    // Replaying the same cancel key is an absorbing operation. It must not
    // issue another runtime command or append a second terminal fact.
    let cancel_replay = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote/cancel")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_session_id": session_id,
                        "idempotency_key": "remote-cancel-smoke"
                    }))
                    .expect("serialize cancel replay request"),
                ))
                .expect("build cancel replay request"),
        )
        .await
        .expect("replay remote cancel");
    assert_eq!(cancel_replay.status(), StatusCode::OK);
    let cancel_replay_body =
        axum::body::to_bytes(cancel_replay.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read cancel replay response");
    let cancel_replayed: Value =
        serde_json::from_slice(&cancel_replay_body).expect("cancel replay JSON");
    assert_eq!(cancel_replayed["session_status"], "cancelled");

    let turn_after_cancel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/remote/turn")
                .header("x-nomi-local-trust", "remote-local-trust")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_session_id": session_id,
                        "input": {"content": "must not run"},
                        "idempotency_key": "remote-turn-after-cancel"
                    }))
                    .expect("serialize post-cancel turn request"),
                ))
                .expect("build post-cancel turn request"),
    )
    .await
    .expect("dispatch post-cancel turn");
    assert_eq!(turn_after_cancel.status(), StatusCode::CONFLICT);

    let delete_session = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/agent-sessions/{session_id}"))
                .header("x-nomi-local-trust", "remote-local-trust")
                .body(Body::empty())
                .expect("build Remote Session delete request"),
        )
        .await
        .expect("delete Remote Session");
    assert_eq!(delete_session.status(), StatusCode::OK);

    let post_delete_requests = [
        Request::builder()
            .uri(format!(
                "/api/remote/observe?agent_session_id={session_id}&after_seq=0&limit=1"
            ))
            .header("x-nomi-local-trust", "remote-local-trust")
            .body(Body::empty())
            .expect("build post-delete observe request"),
        Request::builder()
            .method("POST")
            .uri("/api/remote/turn")
            .header("x-nomi-local-trust", "remote-local-trust")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "agent_session_id": session_id,
                    "input": {"content": "must not resurrect"},
                    "idempotency_key": "remote-turn-after-delete"
                }))
                .expect("serialize post-delete turn request"),
            ))
            .expect("build post-delete turn request"),
        Request::builder()
            .method("POST")
            .uri("/api/remote/cancel")
            .header("x-nomi-local-trust", "remote-local-trust")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "agent_session_id": session_id,
                    "idempotency_key": "remote-cancel-after-delete"
                }))
                .expect("serialize post-delete cancel request"),
            ))
            .expect("build post-delete cancel request"),
    ];
    for request in post_delete_requests {
        let response = router
            .clone()
            .oneshot(request)
            .await
            .expect("dispatch post-delete Remote request");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read post-delete Remote response");
        let value: Value = serde_json::from_slice(&body).expect("post-delete Remote JSON");
        assert!(
            matches!(
                value["code"].as_str(),
                Some("REMOTE_SESSION_NOT_FOUND")
                    | Some("NOMI_CORE_AGENT_SESSION_NOT_FOUND")
                    | Some("SESSION_NOT_FOUND")
                    | Some("NOT_FOUND")
            ),
            "deleted Remote Session must not be resurrected: {value}"
        );
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

#[tokio::test]
async fn nomi_core_remote_accepts_installation_bearer_without_local_trust() {
    let trust_secret = "remote-installation-token-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;

    let missing = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .body(Body::empty())
                .expect("build unauthenticated Remote request"),
        )
        .await
        .expect("dispatch unauthenticated Remote request");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    let missing_body = axum::body::to_bytes(missing.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read unauthenticated Remote response");
    let missing_value: Value =
        serde_json::from_slice(&missing_body).expect("unauthenticated Remote JSON");
    assert_eq!(missing_value["code"], "REMOTE_AUTH_REQUIRED");

    let minted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webui/access-token")
                .header("x-nomi-local-trust", trust_secret)
                .body(Body::empty())
                .expect("build installation token mint request"),
        )
        .await
        .expect("dispatch installation token mint request");
    assert_eq!(minted.status(), StatusCode::OK);
    let minted_body = axum::body::to_bytes(minted.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read installation token response");
    let minted_value: Value =
        serde_json::from_slice(&minted_body).expect("installation token JSON");
    let token = minted_value["data"]["token"]
        .as_str()
        .expect("installation token must be returned once")
        .to_owned();

    let authenticated = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build installation-token Remote request"),
        )
        .await
        .expect("dispatch installation-token Remote request");
    assert_eq!(
        authenticated.status(),
        StatusCode::NOT_FOUND,
        "valid installation token must reach the owner-scoped Remote handler"
    );
    let authenticated_body =
        axum::body::to_bytes(authenticated.into_body(), 4 * 1024 * 1024)
            .await
            .expect("read authenticated Remote response");
    let authenticated_value: Value =
        serde_json::from_slice(&authenticated_body).expect("authenticated Remote JSON");
    assert_eq!(authenticated_value["code"], "REMOTE_SESSION_NOT_FOUND");

    let revoked = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/webui/access-token")
                .header("x-nomi-local-trust", trust_secret)
                .body(Body::empty())
                .expect("build installation token revoke request"),
        )
        .await
        .expect("dispatch installation token revoke request");
    assert_eq!(revoked.status(), StatusCode::OK);
    let revoked_request = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build revoked-token Remote request"),
        )
        .await
        .expect("dispatch revoked-token Remote request");
    assert_eq!(revoked_request.status(), StatusCode::UNAUTHORIZED);
    let revoked_body = axum::body::to_bytes(revoked_request.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read revoked-token Remote response");
    let revoked_value: Value =
        serde_json::from_slice(&revoked_body).expect("revoked-token Remote JSON");
    assert_eq!(revoked_value["code"], "REMOTE_AUTH_REQUIRED");

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

async fn mcp_response(
    router: &axum::Router,
    token: &str,
    session_id: Option<&str>,
    request: Value,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "127.0.0.1")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(session_id) = session_id {
        builder = builder
            .header("mcp-session-id", session_id)
            .header("mcp-protocol-version", "2025-06-18");
    }
    let response = router
        .clone()
        .oneshot(
            builder
                .body(Body::from(
                    serde_json::to_vec(&request).expect("serialize MCP request"),
                ))
                .expect("build MCP request"),
        )
        .await
        .expect("dispatch MCP request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read MCP response body");
    (status, headers, body.to_vec())
}

fn parse_mcp_response(body: &[u8]) -> Value {
    if let Ok(value) = serde_json::from_slice(body) {
        return value;
    }
    let text = String::from_utf8_lossy(body);
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| panic!("MCP response must contain a JSON data event: {text:?}"));
    serde_json::from_str(data).expect("MCP data event must be JSON")
}

fn mcp_tool_error_code(response: &Value) -> String {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP tool result must contain text");
    let text = text.strip_prefix("Error: ").unwrap_or(text);
    let payload: Value = serde_json::from_str(text)
        .unwrap_or_else(|error| panic!("MCP tool text must be JSON ({error}): {text:?}"));
    payload
        .pointer("/error/code")
        .or_else(|| payload.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("MCP tool error must contain a typed code: {response:?}"))
        .to_owned()
}

#[tokio::test]
async fn nomi_core_remote_mcp_uses_installation_auth_and_nomi_core_operations() {
    let trust_secret = "remote-mcp-local-trust";
    let (router, services) = common::build_local_trust_app(trust_secret).await;

    let minted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webui/access-token")
                .header("x-nomi-local-trust", trust_secret)
                .body(Body::empty())
                .expect("build MCP token mint request"),
        )
        .await
        .expect("dispatch MCP token mint request");
    assert_eq!(minted.status(), StatusCode::OK);
    let minted_body = axum::body::to_bytes(minted.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read MCP token response");
    let token = serde_json::from_slice::<Value>(&minted_body)
        .expect("MCP token response JSON")["data"]["token"]
        .as_str()
        .expect("MCP token")
        .to_owned();

    let (status, _, unauthorized_body) = mcp_response(
        &router,
        "not-the-installation-token",
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "nomi-core-route-gap", "version": "1"}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let unauthorized = serde_json::from_slice::<Value>(&unauthorized_body)
        .expect("unauthorized MCP JSON");
    assert_eq!(unauthorized["code"], "REMOTE_AUTH_REQUIRED");

    let (status, headers, body) = mcp_response(
        &router,
        &token,
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "nomi-core-route-gap", "version": "1"}
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session_id = headers
        .get("mcp-session-id")
        .expect("MCP initialize must pin a transport session")
        .to_str()
        .expect("MCP session id must be valid UTF-8")
        .to_owned();
    assert!(parse_mcp_response(&body)["result"].is_object());

    let (status, _, _) = mcp_response(
        &router,
        &token,
        Some(&session_id),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, body) = mcp_response(
        &router,
        &token,
        Some(&session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = parse_mcp_response(&body);
    let names = list["result"]["tools"]
        .as_array()
        .expect("MCP tools/list array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("MCP tool name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["open", "turn", "observe", "cancel"]);

    let session = "0190f5fe-7c00-7a00-8000-000000000001";
    let calls = [
        (
            "open",
            json!({
                "binding_id": "missing-binding",
                "idempotency_key": "mcp-open-gap"
            }),
            "REMOTE_BINDING_NOT_FOUND",
        ),
        (
            "turn",
            json!({
                "agent_session_id": session,
                "input": {"content": "unused"},
                "idempotency_key": "mcp-turn-gap"
            }),
            "REMOTE_SESSION_NOT_FOUND",
        ),
        (
            "observe",
            json!({
                "agent_session_id": session,
                "after_cursor": {"agent_session_id": session, "seq": 0},
                "limit": 1
            }),
            "REMOTE_SESSION_NOT_FOUND",
        ),
        (
            "cancel",
            json!({
                "agent_session_id": session,
                "idempotency_key": "mcp-cancel-gap"
            }),
            "REMOTE_SESSION_NOT_FOUND",
        ),
    ];
    for (index, (name, arguments, expected_code)) in calls.into_iter().enumerate() {
        let (status, _, body) = mcp_response(
            &router,
            &token,
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": index + 3,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "MCP tool call {name}");
        assert_eq!(mcp_tool_error_code(&parse_mcp_response(&body)), expected_code, "{name}");
    }

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

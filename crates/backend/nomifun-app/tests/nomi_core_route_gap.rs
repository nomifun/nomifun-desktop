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

#[path = "common/mod.rs"]
mod common;

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
                        "resource_bindings": [],
                        "model_route_refs": {},
                        "chat_route_records": {}
                    }))
                    .expect("serialize template request"),
                ))
                .expect("build template request"),
        )
        .await
        .expect("dispatch template request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read template response");
    let value: Value = serde_json::from_slice(&body).expect("template response JSON");
    let preset_id = value["data"]["preset"]["preset_id"]
        .as_str()
        .expect("template response preset id")
        .to_owned();
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
                        "resource_bindings": [],
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
    let binding = serde_json::json!({
        "preset_revision_ref": revision,
        "resolved_snapshot_ref": preview["data"]["resolved_snapshot_ref"],
        "typed_resource_bindings": [],
        "binding_version": 1
    });

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
                        "agent_binding": binding,
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
        binding["resolved_snapshot_ref"]
    );

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
                        "resource_bindings": [],
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

    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

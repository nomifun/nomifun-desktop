//! Catalog deletion through the real authenticated App HTTP configuration API.
//! Configuration cleanup preserves canonical Session bindings and model history.

mod common;

use axum::{Router, http::StatusCode};
use http_body_util::BodyExt;
use nomifun_app::compatibility::AppServices;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::MockServer;

use common::{build_app, delete_with_token, get_with_token, json_with_token, setup_and_login};

async fn api(
    app: &Router,
    token: &str,
    csrf: &str,
    method: &str,
    uri: &str,
    body: Value,
    expected: StatusCode,
) -> Value {
    let request = match method {
        "GET" => get_with_token(uri, token),
        "DELETE" => delete_with_token(uri, token, csrf),
        _ => json_with_token(method, uri, body, token, csrf),
    };
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).expect("App API must return JSON");
    assert_eq!(status, expected, "{method} {uri}: {value}");
    value["data"].clone()
}

#[derive(Debug, PartialEq)]
struct HistoricalArtifacts {
    revision_response: Vec<u8>,
    payload_json: String,
    // Compare the exact stored bytes, including the original native envelope.
    snapshot: (String, String, String, String),
}

async fn historical_artifacts(
    app: &Router,
    services: &AppServices,
    token: &str,
    preset_id: &str,
    revision: u64,
) -> HistoricalArtifacts {
    let response = app.clone().oneshot(get_with_token(
        &format!("/api/agent-presets/{preset_id}/revisions/{revision}"), token,
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let revision_response = response.into_body().collect().await.unwrap().to_bytes().to_vec();
    let payload_json = nomifun_db::sqlx::query_scalar(
        "SELECT payload_json FROM agent_preset_revisions WHERE preset_id = ? AND revision_no = ?",
    ).bind(preset_id).bind(revision as i64).fetch_one(services.database.pool()).await.unwrap();
    let snapshot = nomifun_db::sqlx::query_as(
        "SELECT snapshot_id, snapshot_digest, content_json, envelope_json FROM agent_runtime_snapshots \
         WHERE json_extract(content_json, '$.preset_revision_ref.preset_id') = ? \
         AND json_extract(content_json, '$.preset_revision_ref.revision') = ?",
    ).bind(preset_id).bind(revision as i64).fetch_one(services.database.pool()).await.unwrap();
    HistoricalArtifacts { revision_response, payload_json, snapshot }
}

fn non_model_document(editor: &Value) -> Value {
    let mut document = editor["draft"]["document"].clone();
    let object = document.as_object_mut().unwrap();
    object.remove("model_route_refs");
    object.remove("chat_route_records");
    document
}

fn assert_empty_default(editor: &Value) {
    assert!(editor["draft"]["document"]["model_route_refs"].as_object().unwrap().is_empty());
    assert!(editor["draft"]["document"].get("chat_route_records")
        .is_none_or(|records| records.as_object().is_some_and(|records| records.is_empty())));
}

fn revision_number(editor: &Value) -> u64 {
    editor["revision"]["reference"]["revision"].as_u64().unwrap()
}

#[tokio::test]
async fn deleting_a_model_then_its_provider_detaches_current_defaults_without_rewriting_agents_or_history() {
    let provider_server = MockServer::start().await;
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let capability = json!({
        "task": "chat", "traits": [], "protocol": "openai.chat_text",
        "connection_role": "default", "provider_params": {}
    });
    let provider = api(&app, &token, &csrf, "POST", "/api/providers", json!({
        "platform": "custom", "name": "Deletion regression provider",
        "base_url": format!("{}/v1", provider_server.uri()), "auth_scheme": "bearer",
        "credentials": {"api_keys": ["fixture-only-never-used"]},
        "initial_model": {"model": "model-a", "capabilities": [capability]}, "connections": []
    }), StatusCode::CREATED).await;
    let provider_id = provider["provider_id"].as_str().unwrap();
    api(&app, &token, &csrf, "PUT", "/api/provider-models", json!({
        "provider_id": provider_id,
        "model": {"model": "model-b", "description": "Keep the sibling model",
            "capabilities": [capability]}
    }), StatusCode::OK).await;
    let selection_a = json!({"provider_id": provider_id, "model": "model-a"});
    let selection_b = json!({"provider_id": provider_id, "model": "model-b"});
    api(&app, &token, &csrf, "PUT", "/api/settings/client", json!({
        "nomi.defaultModel": selection_a,
        "agent.model_failover": {"queue": [selection_a, selection_b]},
        "theme": "preserve-deletion-fixture"
    }), StatusCode::OK).await;

    let created = api(&app, &token, &csrf, "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "reuse_existing": false, "display_name": "My explicitly configured Agent",
        "description": "Preserve this Agent and its preferences", "model": selection_a
    }), StatusCode::OK).await;
    let preset_id = created["preset"]["preset_id"].as_str().unwrap();
    assert_eq!(created["preset"]["source"], "user");
    assert_eq!(created["draft"]["document"]["chat_route_records"]["agent_chat"]["primary"]["model"], "model-a");
    let mut draft = created["draft"].clone();
    draft["document"]["persona"] = json!("Keep my chosen persona");
    draft["document"]["instructions"] = json!("Keep my explicit instructions");
    draft["document"]["starter_prompts"] = json!(["Keep my starter prompt"]);
    api(&app, &token, &csrf, "POST", &format!("/api/agent-presets/{preset_id}/revisions"), json!({
        "expected_current_revision": draft["current_revision"], "draft": draft,
        "reason": "Explicit personal configuration"
    }), StatusCode::OK).await;
    let editor_uri = format!("/api/agent-presets/{preset_id}/editor");
    let before_a = api(&app, &token, &csrf, "GET", &editor_uri, json!({}), StatusCode::OK).await;
    let old_a_revision = revision_number(&before_a);
    let old_a = historical_artifacts(&app, &services, &token, preset_id, old_a_revision).await;
    let personal_document = non_model_document(&before_a);

    // Existing-session schedules derive their model from the frozen binding,
    // even when the job has no direct model selection to clear.
    tokio::fs::create_dir_all(&services.work_dir).await.unwrap();
    let session = api(&app, &token, &csrf, "POST", "/api/agent-sessions", json!({
        "preset_id": preset_id, "title": "Keep the bound session",
        "session_capabilities": {"skill_names": [], "mcp_server_ids": []}
    }), StatusCode::OK).await;
    let session_id = session["agent_session_id"].as_str().unwrap();
    let binding_before: String = nomifun_db::sqlx::query_scalar("SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=?")
        .bind(session_id).fetch_one(services.database.pool()).await.unwrap();
    let job = api(&app, &token, &csrf, "POST", "/api/cron/jobs", json!({
        "name": "Bound-session schedule", "created_by": "user",
        "conversation_id": session_id, "execution_mode": "existing", "message": "Run later",
        "schedule": {"kind": "every", "every_ms": 600000, "description": "Every ten minutes"}
    }), StatusCode::CREATED).await;
    assert_eq!(job["enabled"], true);
    assert_eq!(job["metadata"]["model"], selection_a, "display uses the exact Session model");
    let job_id = job["cron_job_id"].as_str().unwrap();
    let stored_configuration: (Option<String>, Option<String>, Option<String>) = nomifun_db::sqlx::query_as(
        "SELECT agent_binding,agent_selection,model_selection FROM cron_jobs WHERE cron_job_id=?"
    ).bind(job_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(stored_configuration, (None, None, None), "a continuing schedule must not copy Session authority into its row");
    let job_uri = format!("/api/cron/jobs/{}", job["cron_job_id"].as_str().unwrap());

    api(&app, &token, &csrf, "DELETE", &format!(
        "/api/provider-models?provider_id={provider_id}&model=model-a"
    ), json!({}), StatusCode::OK).await;
    let models = api(&app, &token, &csrf, "GET", &format!("/api/provider-models?provider_id={provider_id}"), json!({}), StatusCode::OK).await;
    assert_eq!(models.as_array().unwrap().len(), 1);
    assert_eq!(models[0]["model"], "model-b");
    assert_eq!(models[0]["description"], "Keep the sibling model");
    let after_a = api(&app, &token, &csrf, "GET", &editor_uri, json!({}), StatusCode::OK).await;
    assert_empty_default(&after_a);
    assert!(revision_number(&after_a) > old_a_revision);
    assert_eq!(non_model_document(&after_a), personal_document);
    for key in ["preset_id", "source", "display_name", "description", "owner_user_id"] {
        assert_eq!(after_a["preset"][key], before_a["preset"][key], "Agent {key} must survive deletion");
    }
    assert_eq!(historical_artifacts(&app, &services, &token, preset_id, old_a_revision).await, old_a);
    let detached_job = api(&app, &token, &csrf, "GET", &job_uri, json!({}), StatusCode::OK).await;
    assert_eq!(detached_job["enabled"], false);
    assert!(detached_job["state"]["next_run_at_ms"].is_null());
    api(&app, &token, &csrf, "PUT", &job_uri, json!({"name": "Still editable after deletion"}), StatusCode::OK).await;
    let binding_after: String = nomifun_db::sqlx::query_scalar("SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=?")
        .bind(session_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(binding_after, binding_before, "disabling a schedule must not change its bound Session");
    let preferences = api(&app, &token, &csrf, "GET", "/api/settings/client", json!({}), StatusCode::OK).await;
    assert!(preferences.get("nomi.defaultModel").is_none(), "the sibling must not become the global default");
    assert_eq!(preferences["agent.model_failover"]["queue"], json!([selection_b]));
    assert_eq!(preferences["theme"], "preserve-deletion-fixture");

    // The user can explicitly choose B after the empty default was persisted.
    api(&app, &token, &csrf, "PUT", "/api/settings/client", json!({
        "nomi.defaultModel": selection_b, "agent.model_failover": {"queue": [selection_b]}
    }), StatusCode::OK).await;
    let selected_b = api(&app, &token, &csrf, "POST", &format!("/api/agent-presets/{preset_id}/chat-model-route"),
        selection_b.clone(), StatusCode::OK).await;
    assert_eq!(selected_b["primary"]["model"], "model-b");
    let mut draft = after_a["draft"].clone();
    draft["document"]["model_route_refs"] = json!({"agent_chat": selected_b["primary"]["model_route_id"]});
    draft["document"]["chat_route_records"] = json!({"agent_chat": selected_b});
    api(&app, &token, &csrf, "POST", &format!("/api/agent-presets/{preset_id}/revisions"), json!({
        "expected_current_revision": draft["current_revision"], "draft": draft,
        "reason": "User explicitly selected model B"
    }), StatusCode::OK).await;
    let before_b = api(&app, &token, &csrf, "GET", &editor_uri, json!({}), StatusCode::OK).await;
    let old_b_revision = revision_number(&before_b);
    let old_b = historical_artifacts(&app, &services, &token, preset_id, old_b_revision).await;
    assert_eq!(before_b["draft"]["document"]["chat_route_records"]["agent_chat"]["primary"]["model"], "model-b");

    api(&app, &token, &csrf, "DELETE", &format!("/api/providers/{provider_id}"), json!({}), StatusCode::OK).await;
    let providers = api(&app, &token, &csrf, "GET", "/api/providers", json!({}), StatusCode::OK).await;
    assert!(providers.as_array().unwrap().is_empty());
    let models = api(&app, &token, &csrf, "GET", "/api/provider-models", json!({}), StatusCode::OK).await;
    assert!(models.as_array().unwrap().is_empty());
    let after_b = api(&app, &token, &csrf, "GET", &editor_uri, json!({}), StatusCode::OK).await;
    assert_empty_default(&after_b);
    assert!(revision_number(&after_b) > old_b_revision);
    assert_eq!(after_b["preset"]["preset_id"], preset_id);
    assert_eq!(after_b["preset"]["display_name"], "My explicitly configured Agent");
    assert_eq!(after_b["preset"]["description"], "Preserve this Agent and its preferences");
    assert_eq!(non_model_document(&after_b), personal_document);
    let preferences = api(&app, &token, &csrf, "GET", "/api/settings/client", json!({}), StatusCode::OK).await;
    assert!(preferences.get("nomi.defaultModel").is_none());
    assert_eq!(preferences["agent.model_failover"]["queue"], json!([]));
    assert_eq!(preferences["theme"], "preserve-deletion-fixture");
    assert_eq!(historical_artifacts(&app, &services, &token, preset_id, old_a_revision).await, old_a);
    assert_eq!(historical_artifacts(&app, &services, &token, preset_id, old_b_revision).await, old_b);
    assert!(provider_server.received_requests().await.unwrap().is_empty(), "configuration deletion must not call a model");
}

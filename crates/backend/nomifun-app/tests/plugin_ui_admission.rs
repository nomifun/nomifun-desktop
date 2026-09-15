//! Agent pages are public choices, while built-in presentation remains the default.
use super::*;

#[tokio::test]
async fn agent_ui_is_available_without_opt_in_and_preserves_builtin_and_generic_surfaces() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    assert!(get(&router, "/api/system/info").await.get("experimental_agent_ui_available").is_none());
    let upstream = wiremock::MockServer::start().await;
    let provider = post(&router, "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Admission model",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": {"api_keys": ["test-only"]},
        "enabled": true, "initial_model": {"model": "step-3.7-flash", "enabled": true,
            "capabilities": [{"task": "chat", "traits": ["function_calling", "streaming"],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {}}]}
    })).await;
    let preset = post(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Builtin still works", "reuse_existing": false,
            "model": {"provider_id": provider["provider_id"], "model": "step-3.7-flash"}
        }),
    )
    .await;
    let preset_id = preset["preset"]["preset_id"].as_str().unwrap();
    let session = post(
        &router,
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id, "title": "Builtin Session"
        }),
    )
    .await;
    let session_id = session["agent_session_id"].as_str().unwrap();
    let session_path = format!("/api/agent-sessions/{session_id}");
    let original_session = get(&router, &session_path).await;
    let editor_path = format!("/api/agent-presets/{preset_id}/editor");
    let original_editor = get(&router, &editor_path).await;
    let binding_path = format!("/api/agent-presets/{preset_id}/ui-binding");
    assert!(get(&router, &binding_path).await["binding"]["selection"].is_null());

    let (status, body) = request(
        &router,
        "POST",
        "/api/plugins/drafts/from-template/agent-session-view",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["status"], "ready");
    assert_eq!(get(&router, "/api/plugins/drafts").await.as_array().unwrap().len(), 1);
    assert_eq!(get(&router, "/api/agent-catalog/ui/agent-session").await, json!([]));

    let plugin = install_ui(&router).await;
    let id = plugin["plugin_id"].as_str().unwrap();
    let plugin = publish_mixed_agent_view(&router, id).await;
    let views = get(&router, "/api/agent-catalog/ui/agent-session").await;
    assert_eq!(views.as_array().unwrap().len(), 1);
    assert_eq!(views[0]["plugin_id"], id);
    assert_eq!(views[0]["expected_release_digest"], plugin["releases"]["active"]["release_digest"]);
    assert!(get(&router, &binding_path).await["binding"]["selection"].is_null(),
        "publishing a page must not select it by default");
    let tool_id = format!("plugin.{id}.echo");
    assert!(
        get(&router, "/api/agent-catalog").await["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["capability"]["id"] == tool_id),
        "publishing the UI of a mixed plugin must not hide its Tool"
    );

    let open_path = format!("/api/plugins/runtimes/{id}/surface/open");
    let bridge_path = format!("/api/plugins/runtimes/{id}/surface/bridge");
    let surface = post(&router, &open_path, json!({"plugin_id": id})).await;
    let generic = post(
        &router,
        &bridge_path,
        storage_body(
            &surface,
            json!({"operation": "get", "key": "ordinary-surface"}),
        ),
    )
    .await;
    assert_eq!(generic["outcome"], "value");
    // Public discovery is not a Session grant: standalone bridges still fail.
    for command in [
        json!({"operation": "observe", "after_seq": 0, "limit": 50}),
        json!({"operation": "turn", "input": {"content": "must not run"}, "idempotency_key": "denied"}),
        json!({"operation": "cancel"}),
    ] {
        let (status, error) = request(&router, "POST", &bridge_path, bridge_body(&surface, command)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    }

    let selection = views[0].clone();
    let binding = json!({"binding_version": 1, "selection": selection});
    let (status, saved) = request(&router, "PUT", &binding_path, json!({
        "expected_binding_version": 0, "selection": selection
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["binding"], binding);
    assert_eq!(get(&router, &binding_path).await["binding"], binding);
    assert_eq!(
        get(&router, &format!("{session_path}/ui-binding")).await["binding"],
        binding
    );
    let surface = post(&router, &open_path, json!({
        "plugin_id": id, "agent_session": {"agent_session_id": session_id,
            "expected_release_digest": selection["expected_release_digest"],
            "ui_capability": selection["capability"]}
    })).await;
    let observed = post(&router, &bridge_path, bridge_body(&surface,
        json!({"operation": "observe", "after_seq": 0, "limit": 50}))).await;
    assert_eq!(observed["session"]["agent_session_id"], session_id);
    assert_eq!(observed["messages"], json!([]));
    let (status, cleared) = request(
        &router,
        "PUT",
        &binding_path,
        json!({
            "expected_binding_version": 1, "selection": null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(
        cleared["data"]["binding"],
        json!({"binding_version": 2, "selection": null})
    );

    assert_eq!(get(&router, &session_path).await, original_session);
    assert_eq!(get(&router, &editor_path).await, original_editor);
    assert!(upstream.received_requests().await.unwrap().is_empty());
    // Switching back to built-in presentation leaves execution unchanged.
    post(
        &router,
        &format!("/api/plugins/runtimes/{id}/surface/close"),
        json!({
            "plugin_id": id, "surface_session_id": surface["surface_session_id"],
            "surface_capability": surface["surface_capability"]
        }),
    )
    .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/step_plan/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": "BUILTIN_STILL_WORKS"}, "finish_reason": null}]}),
                json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}))))
        .expect(1).mount(&upstream).await;
    post(
        &router,
        &format!("{session_path}/turns"),
        json!({
            "input": {"content": "builtin turn"}, "idempotency_key": "builtin-not-gated"
        }),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let page = get(&router, &session_path).await;
            if page["messages"].to_string().contains("BUILTIN_STILL_WORKS") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("public plugin UI must not disable builtin Session turns");
    assert_mixed_tool_still_executes(&router, &services, preset_id, id, &tool_id).await;
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

async fn publish_mixed_agent_view(router: &axum::Router, id: &str) -> Value {
    let base = format!("/api/plugins/runtimes/{id}");
    let w = get(router, &format!("{base}/workshop")).await;
    post(router, &format!("{base}/source/edit"), json!({
        "plugin_id": id, "expected_product_revision": w["plugin"]["product_revision"],
        "project_id": w["project_id"], "expected_project_revision": w["project_revision"],
        "expected_build_generation": w["build_generation"],
        "expected_source_snapshot_digest": w["source_snapshot_digest"],
        "path": "service/main.mjs", "content":
            "export async function start() { return { async invoke({method, payload}) { if (method !== 'echo') throw new Error('unexpected method'); return payload; }, async dispose() {} }; }"
    })).await;
    publish_agent_view_manifest(router, id, json!({
        "agent_view": {"name": "My Agent view", "description": "Mixed UI and Tool"},
        "actions": [{"id": "echo", "name": "Echo", "description": "Echo input",
            "input_schema": {"type": "object"}, "output_schema": {"type": "object"}, "effect": "pure"}]
    }), true).await
}

async fn assert_mixed_tool_still_executes(
    router: &axum::Router,
    services: &nomifun_app::compatibility::AppServices,
    preset_id: &str,
    plugin_id: &str,
    tool_id: &str,
) {
    use nomifun_plugin_platform::runtime::{
        PluginRuntimeAgentCapabilityInvocation, PluginRuntimeAgentCapabilityPort,
    };
    let editor = get(router, &format!("/api/agent-presets/{preset_id}/editor")).await;
    let mut draft = editor["draft"].clone();
    draft["document"]["enabled_capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "capability": {"id": tool_id, "version": "1.0.0"}, "action_allowlist": ["echo"]
        }));
    let saved = post(
        router,
        &format!("/api/agent-presets/{preset_id}/revisions"),
        json!({
            "expected_current_revision": draft["current_revision"], "draft": draft
        }),
    )
    .await;
    let json: String = sqlx::query_scalar(
        "SELECT snapshot_json FROM nomi_agent_preset_revisions WHERE preset_id = ? AND revision_no = ?",
    ).bind(preset_id).bind(saved["revision"]["reference"]["revision"].as_i64().unwrap())
        .fetch_one(services.database.pool()).await.unwrap();
    let snapshot: nomifun_agent_contracts::ResolvedSnapshotEnvelope =
        serde_json::from_str(&json).unwrap();
    let tool = snapshot
        .content
        .enabled_capabilities
        .iter()
        .find(|capability| capability.capability.id.as_ref() == tool_id)
        .unwrap();
    let persisted_digest: String = sqlx::query_scalar(
        "SELECT materialized_catalog_digest FROM plugin_products WHERE plugin_product_id = ?",
    )
    .bind(plugin_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert_eq!(
        tool.catalog_digest.as_ref().unwrap().as_ref(),
        persisted_digest,
        "UI rollout must not rewrite the publication digest frozen into a sibling Tool"
    );
    let owner = nomifun_db::installation_owner_id(services.database.pool())
        .await
        .unwrap();
    let schema = services
        .plugin_runtime
        .resolve_agent_capability_schema(&owner, tool, &tool.actions[0].input_schema)
        .await
        .expect("mixed-plugin schema must still match the persisted Product digest");
    assert_eq!(schema.0["type"], "object");
    let payload = nomifun_agent_contracts::StrictJsonValue(json!({"mixed_tool": "still works"}));
    let result = services
        .plugin_runtime
        .invoke_agent_capability(PluginRuntimeAgentCapabilityInvocation {
            owner_user_id: owner,
            plugin_product_id: plugin_id.into(),
            capability: tool.capability.clone(),
            action_id: "echo".into(),
            action_allowlist: std::collections::BTreeSet::from(["echo".into()]),
            active_release: tool.active_release.clone().unwrap(),
            active_release_epoch: tool.active_release_epoch.unwrap(),
            catalog_digest: tool.catalog_digest.clone().unwrap(),
            operation_id: uuid::Uuid::now_v7().to_string().into(),
            call_id: uuid::Uuid::now_v7().to_string().into(),
            payload: payload.clone(),
        })
        .await
        .expect("mixed-plugin Tool invocation must not fail with a stale Catalog digest");
    assert_eq!(result, payload);
    let views = get(router, "/api/agent-catalog/ui/agent-session").await;
    assert_eq!(views.as_array().unwrap().len(), 1);
    assert_eq!(views[0]["plugin_id"], plugin_id);
}

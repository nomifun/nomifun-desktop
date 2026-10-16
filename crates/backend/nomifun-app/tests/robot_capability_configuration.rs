//! Robot capability edits refresh the existing canonical companion binding.
mod common;
use common::companion_agent::{COMPANION_AGENT_TEST_TRUST, companion_agent_test_request, save_companion_core_agent_for_fixture};
use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn companion_explicit_resource_selection_updates_the_existing_session_and_defaults() {
    let call = companion_agent_test_request;
    let (router, services) = common::build_local_trust_app(COMPANION_AGENT_TEST_TRUST).await;
    common::materialize_builtin_skills_for_fixture(&services).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Resource selection fixture",
        "base_url":format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash", "enabled":true,
            "capabilities":[{"task":"chat", "traits":[], "protocol":"openai.chat_text",
                "connection_role":"default", "provider_params":{}}]},
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({"provider_id":provider["data"]["provider_id"], "model":"step-3.7-flash"});
    let (status, companion) = call(router.clone(), "POST", "/api/companion/companions",
        json!({"name":"Resource selection companion", "character":"ink"})).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap();
    let (status, patched) = call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"), json!({"model":model})).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    save_companion_core_agent_for_fixture(router.clone()).await;
    let selection = json!({"kind":"template", "template_key":"companion.default"});
    let product_path = format!("/api/product-agent-bindings/companion/{companion_id}");
    let (status, initial) = call(router.clone(), "PUT", &product_path,
        json!({"selection":selection, "model":model, "resource_selections":[]})).await;
    assert_eq!(status, StatusCode::OK, "{initial}");
    let thread_path = format!("/api/companion/companions/{companion_id}/companion/threads");
    let (status, thread) = call(router.clone(), "POST", &thread_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    let session_id = thread["data"]["conversation_id"].as_str().unwrap();
    let projection_path = format!("/api/agent-sessions/{session_id}/projection");
    let (status, before) = call(router.clone(), "GET", &projection_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let before_binding = &before["data"]["agent_snapshot"]["canonical_binding"];
    assert!(!before_binding["typed_resource_bindings"].as_array().unwrap().iter()
        .any(|resource| resource["resource_kind"] == "robot"));
    let registry = &services.robot.as_ref().expect("robot fixture service").registry;
    let (robot, _) = registry.upsert_on_report(nomifun_robot::registry::RobotReport {
        robot_id:"resource-selection-robot".to_owned(), client_id:"fixture-client".to_owned(),
        board:"fixture-board".to_owned(), firmware_version:"test".to_owned(),
    }, 1).await.unwrap();
    registry.claim(robot.activation_code.as_deref().unwrap(), companion_id).await.unwrap();
    let request = json!({"selection":selection, "model":model, "conversation_id":session_id,
        "resource_selections":[{"resource_kind":"robot", "resource_id":robot.robot_id}]});
    let (status, applied) = call(router.clone(), "PUT", &product_path, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let (status, after) = call(router.clone(), "GET", &projection_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["data"]["conversation_id"], session_id);
    assert_eq!(after["data"]["extra"]["official_template_key"], "companion.default",
        "a resource-only transition must retain the verified official Agent identity");
    assert_eq!(after["data"]["agent_snapshot"]["resolved_model"], before["data"]["agent_snapshot"]["resolved_model"]);
    let after_binding = &after["data"]["agent_snapshot"]["canonical_binding"];
    assert_eq!(after_binding["binding_version"].as_u64(), Some(before_binding["binding_version"].as_u64().unwrap() + 1));
    let resources = after_binding["typed_resource_bindings"].as_array().unwrap();
    assert!(resources.iter().any(|resource| resource["resource_kind"] == "robot" && resource["resource_id"] == robot.robot_id));
    for kind in ["companion", "companion_memory"] {
        assert!(resources.iter().any(|resource| resource["resource_kind"] == kind && resource["resource_id"] == companion_id));
    }
    let selected_resources = |binding: &Value| binding["typed_resource_bindings"].as_array().unwrap().iter()
        .filter(|resource| resource["resource_kind"] != "workspace").cloned().collect::<Vec<_>>();
    assert_eq!(selected_resources(&applied["data"]["agent_binding"]), selected_resources(after_binding));
    let workspace = |binding: &Value| binding["typed_resource_bindings"].as_array().unwrap().iter()
        .find(|resource| resource["resource_kind"] == "workspace").cloned();
    assert_eq!(workspace(after_binding), workspace(before_binding), "the existing Session keeps its frozen workspace");
    let transitions = || sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'session/agent-binding-changed'")
        .bind(session_id).fetch_one(services.database.pool());
    assert_eq!(transitions().await.unwrap(), 1);
    let (status, noop) = call(router.clone(), "PUT", &product_path, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{noop}");
    assert_eq!(transitions().await.unwrap(), 1, "identical resources must not append a transition");
    let (status, unchanged) = call(router.clone(), "GET", &projection_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["data"]["agent_snapshot"]["canonical_binding"], *after_binding);
    let mut invalid = request.clone();
    invalid["resource_selections"][0]["resource_id"] = json!("missing-robot");
    let (status, rejected) = call(router.clone(), "PUT", &product_path, invalid).await;
    assert!(!status.is_success(), "an invalid resource cannot be saved: {rejected}");
    let (status, still_current) = call(router.clone(), "GET", &projection_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{still_current}");
    assert_eq!(still_current["data"]["agent_snapshot"]["canonical_binding"], *after_binding);
    let default_resources: String = sqlx::query_scalar("SELECT agent_binding_json FROM agent_bindings WHERE target_kind = 'companion' AND target_id = ?")
        .bind(companion_id).fetch_one(services.database.pool()).await.unwrap();
    let default_binding: Value = serde_json::from_str(&default_resources).unwrap();
    assert_eq!(selected_resources(&default_binding), selected_resources(after_binding));
    // Editing device metadata alone does not rewrite an immutable Session.
    // Explicitly applying the same robot freezes its new configuration revision.
    let robot_services = services.robot.as_ref().unwrap();
    let (device_tx, _device_rx) = tokio::sync::mpsc::channel(4);
    registry.connect(&robot.robot_id, companion_id, "profile-connection").await.unwrap();
    robot_services.tools.attach(&robot.robot_id, std::sync::Arc::new(
        nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx, "profile-connection".into())),
        vec![nomifun_robot::mcp_bridge::RobotToolDescriptor {
            device_name:"vendor.arm.pose".into(), exposed_name:"robot_vendor_arm_pose".into(),
            description:"Device gesture".into(), input_schema:json!({"type":"object"}),
        }]).await;
    let mut profile = nomifun_robot::capability_profile::RobotCapabilityProfile::default();
    profile.tool_overrides.insert("vendor.arm.pose".into(), nomifun_robot::capability_profile::RobotToolOverride {
        description:Some("A user-edited gesture description".into()), ..Default::default()
    });
    registry.set_capability_profile(&robot.robot_id, 0, profile, &robot_services.tools).await.unwrap();
    assert_eq!(after_binding["typed_resource_bindings"].as_array().unwrap().iter()
        .find(|resource| resource["resource_kind"] == "robot").unwrap()["typed_parameters"]["capability_revision"], "0");
    let (status, revised) = call(router.clone(), "PUT", &product_path, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{revised}");
    let (status, updated) = call(router.clone(), "GET", &projection_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["data"]["conversation_id"], session_id);
    assert_eq!(updated["data"]["agent_snapshot"]["resolved_model"], before["data"]["agent_snapshot"]["resolved_model"]);
    assert_eq!(updated["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap().iter()
        .find(|resource| resource["resource_kind"] == "robot").unwrap()["typed_parameters"]["capability_revision"], "1");
    assert_eq!(transitions().await.unwrap(), 2, "a changed robot capability revision must refresh the existing runtime");
    let (status, removed) = call(router.clone(), "PUT", &product_path,
        json!({"selection":selection, "model":model, "resource_selections":[]})).await;
    assert_eq!(status, StatusCode::OK, "{removed}");
    let (status, without_robot) = call(router.clone(), "GET", &projection_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{without_robot}");
    assert!(!without_robot["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"]
        .as_array().unwrap().iter().any(|resource| resource["resource_kind"] == "robot"));
    assert_eq!(transitions().await.unwrap(), 3, "explicit empty selection must remove optional resources");
    let (status, reopened) = call(router.clone(), "POST", &thread_path, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{reopened}");
    assert_eq!(reopened["data"]["conversation_id"], session_id);

    // Changing the Agent profile carries the explicitly selected device into
    // the same Session. Omitting resource_selections preserves that choice;
    // an explicit empty list above remains the separate removal operation.
    let (status, rebound) = call(router.clone(), "PUT", &product_path, request).await;
    assert_eq!(status, StatusCode::OK, "{rebound}");
    for key in ["companion.robot", "companion.default"] {
        let (status, switched) = call(router.clone(), "PUT", &product_path, json!({
            "selection":{"kind":"template","template_key":key}, "model":model, "conversation_id":session_id,
        })).await;
        assert_eq!(status, StatusCode::OK, "{key}: {switched}");
        let (status, projected) = call(router.clone(), "GET", &projection_path, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{projected}");
        assert_eq!(projected["data"]["conversation_id"], session_id);
        assert_eq!(projected["data"]["extra"]["official_template_key"], key);
        let carried = projected["data"]["agent_snapshot"]["canonical_binding"]["typed_resource_bindings"].as_array().unwrap();
        for kind in ["companion", "companion_memory"] {
            assert!(carried.iter().any(|resource| resource["resource_kind"] == kind && resource["resource_id"] == companion_id));
        }
        let device = carried.iter().find(|resource| resource["resource_kind"] == "robot").expect("selected Robot is preserved");
        assert_eq!(device["resource_id"], robot.robot_id);
        assert_eq!(device["typed_parameters"]["capability_revision"], "1");
        assert_eq!(projected["data"]["agent_snapshot"]["resolved_model"], before["data"]["agent_snapshot"]["resolved_model"]);
    }
    let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions WHERE state = 'live'")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(session_count, 1, "switching robot profiles preserves the one Companion Session");
    assert!(upstream.received_requests().await.unwrap().is_empty(), "configuration changes must not invoke the model");
    services.shutdown_browser_platform().await.expect("browser cleanup");
    services.database.close().await;
}

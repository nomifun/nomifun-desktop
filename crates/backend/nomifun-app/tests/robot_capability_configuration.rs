//! Independent Companion/Robot defaults, exact device Sessions and workspace ownership.
mod common;
use axum::http::StatusCode;
use common::companion_agent::{COMPANION_AGENT_TEST_TRUST, companion_agent_test_request, save_companion_core_agent_for_fixture};
use nomifun_agent_contracts::{AgentSessionId, CompanionSessionRole};
use nomifun_agent_session::AgentSessionStore;
use serde_json::{Value, json};

struct Fixture {
    router: axum::Router,
    services: nomifun_app::compatibility::AppServices,
    upstream: wiremock::MockServer,
    companion_id: String,
    model: Value,
}

async fn fixture() -> Fixture {
    let call = companion_agent_test_request;
    let (router, services) = common::build_local_trust_app(COMPANION_AGENT_TEST_TRUST).await;
    common::materialize_builtin_skills_for_fixture(&services).await;
    let upstream = wiremock::MockServer::start().await;
    let (status, provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Independent role configuration",
        "base_url":format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash", "enabled":true,
            "capabilities":[{"task":"chat", "traits":[], "protocol":"openai.chat_text",
                "connection_role":"default", "provider_params":{}}]},
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({"provider_id":provider["data"]["provider_id"], "model":"step-3.7-flash"});
    let (status, companion) = call(router.clone(), "POST", "/api/companion/companions",
        json!({"name":"Independent role companion", "character":"ink"})).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap().to_owned();
    let (status, patched) = call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"), json!({"model":model})).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    save_companion_core_agent_for_fixture(router.clone()).await;
    Fixture { router, services, upstream, companion_id, model }
}

async fn projection(router: &axum::Router, session: &str) -> Value {
    let (status, value) = companion_agent_test_request(router.clone(), "GET",
        &format!("/api/agent-sessions/{session}/projection"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "Session {session} projection failed: {value}");
    value["data"].clone()
}

fn binding(projection: &Value) -> &Value { &projection["agent_snapshot"]["canonical_binding"] }

fn device(projection: &Value) -> &Value {
    let devices = binding(projection)["typed_resource_bindings"].as_array().unwrap().iter()
        .filter(|resource| resource["resource_kind"] == "robot").collect::<Vec<_>>();
    assert_eq!(devices.len(), 1, "a Robot Session freezes exactly one device");
    devices[0]
}

async fn product_default(fixture: &Fixture, role: &str) -> String {
    sqlx::query_scalar("SELECT agent_binding_json FROM agent_bindings WHERE target_kind=? AND target_id=?")
        .bind(role).bind(&fixture.companion_id).fetch_one(fixture.services.database.pool()).await.unwrap()
}

fn assert_explicit_wide_actions(actual: &serde_json::Map<String, Value>, discovery: &Value) {
    assert_eq!(actual["workspace.files"], json!(["workspace.files/read", "workspace.files/write"]),
        "a Robot Agent keeps its explicitly selected local computer capabilities");
    assert!(actual["agent.collaboration"].as_array().unwrap().contains(&json!("agent/fork")),
        "the physical entry must not trim an explicitly selected collaboration action");
    if let Some(actions) = actual.get("agent.tool-discovery") {
        assert_eq!(discovery["source_package"]["id"], "nomifun.tool-discovery");
        assert_eq!(actions.as_array().unwrap().iter().map(|action| action.as_str().unwrap()).collect::<std::collections::BTreeSet<_>>(),
            discovery["actions"].as_array().unwrap().iter().map(|action| action["action_id"].as_str().unwrap()).collect(),
            "the host discovery module keeps its exact formal actions");
    }
}

fn assert_explicit_role_binding(current: &Value, discovery: &Value, expected_skills: &[&str]) {
    assert_explicit_wide_actions(current["agent_snapshot"]["enabled_capability_actions"].as_object().unwrap(), discovery);
    assert_eq!(current["agent_snapshot"]["included_skills"], json!(expected_skills),
        "Robot keeps the explicit source selection without selecting the global inventory");
    assert!(binding(current)["typed_resource_bindings"].as_array().unwrap().iter().any(|resource|
        resource["resource_kind"] == "workspace" && !resource["operations"].as_array().unwrap().is_empty()),
        "explicit local computer actions retain their admitted workspace grant");
}

async fn assert_physical_skill_locks(fixture: &Fixture, current: &Value, expected_skills: &[&str]) {
    let raw: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
        .bind(binding(current)["resolved_snapshot_ref"]["snapshot_id"].as_str().unwrap())
        .fetch_one(fixture.services.database.pool()).await.unwrap();
    let snapshot: Value = serde_json::from_str(&raw).unwrap();
    let locks = snapshot["content"]["skill_locks"].as_array().unwrap();
    assert_eq!(locks.len(), expected_skills.len(), "unselected library bodies are absent from the actual frozen Snapshot");
    for lock in locks {
        assert_eq!(lock["selected"], true);
        assert!(expected_skills.contains(&lock["skill"]["name"].as_str().unwrap()));
        assert!(lock["skill"]["body"].as_str().unwrap().contains("AUTHORED_ROBOT_SKILL_BODY"));
    }
}

async fn assert_explicit_wide_default(fixture: &Fixture, discovery: &Value) {
    let binding: Value = serde_json::from_str(&product_default(fixture, "robot").await).unwrap();
    let raw: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
        .bind(binding["resolved_snapshot_ref"]["snapshot_id"].as_str().unwrap())
        .fetch_one(fixture.services.database.pool()).await.unwrap();
    let snapshot: Value = serde_json::from_str(&raw).unwrap();
    let actual = snapshot["content"]["enabled_capabilities"].as_array().unwrap().iter()
        .map(|selection| (selection["capability"]["id"].as_str().unwrap().to_owned(), selection["action_allowlist"].clone()))
        .collect();
    assert_explicit_wide_actions(&actual, discovery);
    assert!(binding["typed_resource_bindings"].as_array().unwrap().iter().any(|resource|
        resource["resource_kind"] == "workspace" && !resource["operations"].as_array().unwrap().is_empty()),
        "future Robot defaults retain explicitly authorized workspace operations");
}

#[tokio::test]
async fn personal_robot_agent_keeps_explicit_capabilities_and_frozen_skills_across_create_switch_and_composer() {
    const AUTHORED_SKILL: &str = "physical-authored-guidance";
    const INVENTORY_SKILL: &str = "physical-unselected-inventory";
    let call = companion_agent_test_request;
    let f = fixture().await;
    let services = &f.services;
    let companion_id = &f.companion_id;
    let (status, catalog) = call(f.router.clone(), "GET", "/api/agent-catalog", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{catalog}");
    let discovery = catalog["data"]["modules"].as_array().unwrap().iter()
        .find(|module| module["module"]["id"] == "agent.tool-discovery").unwrap();
    let robot_path = format!("/api/product-agent-bindings/robot/{companion_id}");
    let registry = &services.robot.as_ref().unwrap().registry;
    let (robot, _) = registry.upsert_on_report(nomifun_robot::registry::RobotReport {
        robot_id:"personal-wide-role-device".into(), client_id:"personal-wide-role-client".into(),
        board:"fixture-board".into(), firmware_version:"test".into(),
    }, 1).await.unwrap();
    registry.claim(robot.activation_code.as_deref().unwrap(), companion_id).await.unwrap();
    let (device_tx, _device_rx) = tokio::sync::mpsc::channel(4);
    registry.connect(&robot.robot_id, companion_id, "personal-wide-role-connection").await.unwrap();
    services.robot.as_ref().unwrap().tools.attach(&robot.robot_id, std::sync::Arc::new(
        nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx, "personal-wide-role-connection".into())), Vec::new()).await;
    let (status, official) = call(f.router.clone(), "POST", "/api/agent-presets/from-template/companion.robot", json!({
        "display_name":"companion.robot", "reuse_existing":true, "model":f.model,
    })).await;
    assert_eq!(status, StatusCode::OK, "{official}");
    let mut document = official["data"]["draft"]["document"].clone();
    document["persona"] = json!("PERSONAL_PHYSICAL_ROLE_PERSONA");
    document["instructions"] = json!("PERSONAL_PHYSICAL_ROLE_RULE_V1");
    for name in [AUTHORED_SKILL, INVENTORY_SKILL] {
        let directory = services.skill_paths.user_skills_dir.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("SKILL.md"), format!(
            "---\nname: {name}\ndescription: Physical role fixture {name}\n---\nAUTHORED_ROBOT_SKILL_BODY {name}\n")).unwrap();
    }
    let capabilities = document["enabled_capabilities"].as_array_mut().unwrap();
    capabilities.iter_mut().find(|selection| selection["capability"]["id"] == "agent.collaboration").unwrap()
        ["action_allowlist"] = json!(["agent/delegate", "agent/fork", "agent/request_user_decision"]);
    capabilities.push(json!({"capability":{"id":"workspace.files"},
        "action_allowlist":["workspace.files/read", "workspace.files/write"]}));
    let (status, personal) = call(f.router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"Wide authored physical Agent", "document":document,
    })).await;
    assert_eq!(status, StatusCode::OK, "{personal}");
    let personal_id = personal["data"]["preset"]["preset_id"].as_str().unwrap();
    // Library bytes are captured only by the formal Session extension owner;
    // public Agent authoring accepts package references, never frozen bodies.
    // This source is a legal current-generation Session variant, not a public
    // author's Library binding. Its immutable selected intent must survive a
    // trusted product launch that starts from those saved artifacts.
    let (status, source) = call(f.router.clone(), "POST", "/api/agent-sessions", json!({
        "preset_id":personal_id, "model":f.model, "title":"Source frozen extension selection",
        "session_capabilities":{"skill_names":[AUTHORED_SKILL], "mcp_server_ids":[]},
        "resource_selections":[
            {"resource_kind":"companion", "resource_id":companion_id},
            {"resource_kind":"companion_memory", "resource_id":companion_id},
            {"resource_kind":"robot", "resource_id":robot.robot_id},
        ],
    })).await;
    assert_eq!(status, StatusCode::OK, "{source}");
    let source_binding = &source["data"]["agent_binding"];
    let source_raw: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
        .bind(source_binding["resolved_snapshot_ref"]["snapshot_id"].as_str().unwrap())
        .fetch_one(services.database.pool()).await.unwrap();
    let source_snapshot: Value = serde_json::from_str(&source_raw).unwrap();
    let source_skills = source_snapshot["content"]["skill_locks"].as_array().unwrap();
    assert!(source_skills.iter().any(|lock| lock["skill"]["name"] == AUTHORED_SKILL && lock["selected"] == true));
    assert!(source_skills.iter().any(|lock| lock["skill"]["name"] == INVENTORY_SKILL && lock["selected"] == false));
    let source_selection = json!({"kind":"preset", "preset_id":source_binding["preset_revision_ref"]["preset_id"]});
    let (status, configured) = call(f.router.clone(), "PUT", &robot_path, json!({
        "selection":source_selection, "model":f.model,
        "resource_selections":[{"resource_kind":"robot", "resource_id":robot.robot_id}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{configured}");
    assert_explicit_wide_default(&f, discovery).await;
    let desktop = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Companion, None).await.unwrap();
    let desktop_before = projection(&f.router, &desktop.conversation_id).await;
    let physical = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some(&robot.robot_id)).await.unwrap();
    assert_explicit_wide_default(&f, discovery).await;
    let before = projection(&f.router, &physical.conversation_id).await;
    assert_explicit_role_binding(&before, discovery, &[AUTHORED_SKILL]);
    assert_physical_skill_locks(&f, &before, &[AUTHORED_SKILL]).await;
    assert_eq!(device(&before)["resource_id"], robot.robot_id);
    assert_eq!(before["agent_snapshot"]["resolved_model"]["provider_id"], f.model["provider_id"]);
    for marker in ["PERSONAL_PHYSICAL_ROLE_PERSONA", "PERSONAL_PHYSICAL_ROLE_RULE_V1"] {
        assert!(before["agent_snapshot"]["instructions"].as_str().unwrap().contains(marker));
    }
    let (status, editor) = call(f.router.clone(), "GET", &format!("/api/agent-presets/{personal_id}/editor"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{editor}");
    assert_eq!(editor["data"]["draft"]["document"]["enabled_capabilities"], document["enabled_capabilities"],
        "product launch does not rewrite the authored Agent");
    let mut draft = editor["data"]["draft"].clone();
    draft["document"]["instructions"] = json!("PERSONAL_PHYSICAL_ROLE_RULE_V2");
    let (status, saved) = call(f.router.clone(), "POST", &format!("/api/agent-presets/{personal_id}/revisions"), json!({
        "expected_current_revision":draft["current_revision"], "draft":draft,
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let selection = json!({"kind":"preset", "preset_id":personal_id});
    let (status, future) = call(f.router.clone(), "PUT", &robot_path, json!({
        "selection":selection, "model":f.model,
        "resource_selections":[{"resource_kind":"robot", "resource_id":robot.robot_id}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{future}");
    assert_explicit_wide_default(&f, discovery).await;
    let default_before_apply = product_default(&f, "robot").await;
    assert_eq!(binding(&projection(&f.router, &physical.conversation_id).await), binding(&before),
        "future defaults cannot edit an existing physical Session");
    let (status, applied) = call(f.router.clone(), "PUT", &robot_path, json!({
        "selection":selection, "conversation_id":physical.conversation_id, "update_default":false,
        "resource_selections":[{"resource_kind":"robot", "resource_id":robot.robot_id}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let after = projection(&f.router, &physical.conversation_id).await;
    assert_explicit_role_binding(&after, discovery, &[AUTHORED_SKILL]);
    assert_physical_skill_locks(&f, &after, &[AUTHORED_SKILL]).await;
    assert!(after["agent_snapshot"]["instructions"].as_str().unwrap().contains("PERSONAL_PHYSICAL_ROLE_RULE_V2"));
    assert_eq!(after["agent_snapshot"]["resolved_model"], before["agent_snapshot"]["resolved_model"]);
    assert_eq!(device(&after)["resource_id"], robot.robot_id);
    assert_eq!(product_default(&f, "robot").await, default_before_apply, "current-only apply preserves the future default");
    let (status, composed) = call(f.router.clone(), "PUT",
        &format!("/api/agent-sessions/{}/capability-selection", physical.conversation_id), json!({
            "expected_binding_version":binding(&after)["binding_version"],
            "selection":{"skill_names":[], "mcp_server_ids":[]},
        })).await;
    assert_eq!(status, StatusCode::OK, "{composed}");
    let cleared = projection(&f.router, &physical.conversation_id).await;
    assert_explicit_role_binding(&cleared, discovery, &[]);
    assert_physical_skill_locks(&f, &cleared, &[]).await;
    assert_eq!(binding(&cleared)["typed_resource_bindings"], binding(&after)["typed_resource_bindings"],
        "clearing a Skill keeps the exact Companion, memory, device and zero-operation workspace choices");
    assert_eq!(binding(&projection(&f.router, &desktop.conversation_id).await), binding(&desktop_before));
    assert_eq!(product_default(&f, "robot").await, default_before_apply);
    let source_after: String = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=?")
        .bind(source_binding["resolved_snapshot_ref"]["snapshot_id"].as_str().unwrap())
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(source_after, source_raw, "launching and editing another Session never rewrites the source variant's frozen Skill facts");
    assert!(f.upstream.received_requests().await.unwrap().is_empty(), "role configuration performs no model call");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn physical_role_without_robot_tools_keeps_verified_device_identity_and_rejects_foreign_device() {
    let call = companion_agent_test_request;
    let f = fixture().await;
    let services = &f.services;
    let companion_id = &f.companion_id;
    let (status, other) = call(f.router.clone(), "POST", "/api/companion/companions",
        json!({"name":"Separate physical owner", "character":"ink"})).await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    let other_id = other["data"]["companion_id"].as_str().unwrap();
    let registry = &services.robot.as_ref().unwrap().registry;
    for (device_id, profile_id) in [("optional-hardware-owned", companion_id.as_str()), ("optional-hardware-foreign", other_id)] {
        let (device, _) = registry.upsert_on_report(nomifun_robot::registry::RobotReport {
            robot_id:device_id.into(), client_id:format!("client-{device_id}"),
            board:"fixture-board".into(), firmware_version:"test".into(),
        }, 1).await.unwrap();
        registry.claim(device.activation_code.as_deref().unwrap(), profile_id).await.unwrap();
    }
    let (status, official) = call(f.router.clone(), "POST", "/api/agent-presets/from-template/companion.robot", json!({
        "display_name":"companion.robot", "reuse_existing":true, "model":f.model,
    })).await;
    assert_eq!(status, StatusCode::OK, "{official}");
    let mut document = official["data"]["draft"]["document"].clone();
    document["enabled_capabilities"].as_array_mut().unwrap().retain(|selection|
        matches!(selection["capability"]["id"].as_str(), Some("companion" | "companion.memory")));
    document["context_order"] = json!([]);
    document["middleware_order"] = json!([]);
    let (status, personal) = call(f.router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"Physical chat without hardware tools", "document":document,
    })).await;
    assert_eq!(status, StatusCode::OK, "{personal}");
    let (status, configured) = call(f.router.clone(), "PUT",
        &format!("/api/product-agent-bindings/robot/{companion_id}"), json!({
            "selection":{"kind":"preset", "preset_id":personal["data"]["preset"]["preset_id"]},
            "model":f.model, "resource_selections":[],
        })).await;
    assert_eq!(status, StatusCode::OK, "{configured}");
    let physical = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some("optional-hardware-owned")).await.unwrap();
    let current = projection(&f.router, &physical.conversation_id).await;
    assert_eq!(current["extra"]["companion_role"], "robot");
    assert_eq!(current["extra"]["robot_id"], "optional-hardware-owned");
    assert!(!current["agent_snapshot"]["enabled_capabilities"].as_array().unwrap().contains(&json!("robot")),
        "physical conversation does not require granting hardware actions");
    assert!(!binding(&current)["typed_resource_bindings"].as_array().unwrap().iter().any(|resource|
        resource["resource_kind"] == "robot"), "a physical-channel identity does not create a hardware resource grant");
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let canonical = store.get_live_session(&AgentSessionId::from(physical.conversation_id)).await.unwrap();
    let origin = canonical.metadata.companion.unwrap();
    assert_eq!(origin.role, CompanionSessionRole::Robot);
    assert_eq!(origin.robot_id.as_deref(), Some("optional-hardware-owned"));
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(services.database.pool()).await.unwrap();
    for rejected_device in ["optional-hardware-foreign", "optional-hardware-missing"] {
        assert!(services.companion_service.ensure_session_for_role(
            companion_id, nomifun_companion::CompanionRole::Robot, Some(rejected_device)).await.is_err(),
            "a writable device ID cannot create authenticated physical ownership");
    }
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(after, before, "foreign and unknown device requests create no canonical Session");
    assert!(f.upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn robot_role_configuration_preserves_desktop_and_each_exact_device_session() {
    let call = companion_agent_test_request;
    let f = fixture().await;
    let router = &f.router;
    let services = &f.services;
    let companion_id = &f.companion_id;
    let desktop_path = format!("/api/product-agent-bindings/companion/{companion_id}");
    let robot_path = format!("/api/product-agent-bindings/robot/{companion_id}");
    let desktop_selection = json!({"kind":"template", "template_key":"companion.default"});
    let robot_selection = json!({"kind":"template", "template_key":"companion.robot"});
    let (status, saved) = call(router.clone(), "PUT", &desktop_path, json!({
        "selection":desktop_selection, "model":f.model, "resource_selections":[],
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let desktop = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Companion, None).await.unwrap();
    let desktop_default = product_default(&f, "companion").await;
    let (status, robot_provider) = call(router.clone(), "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Robot-only model route",
        "base_url":format!("{}/step_plan/v1", f.upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash", "enabled":true,
            "capabilities":[{"task":"chat", "traits":[], "protocol":"openai.chat_text",
                "connection_role":"default", "provider_params":{}}]},
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{robot_provider}");
    let robot_model = json!({"provider_id":robot_provider["data"]["provider_id"], "model":"step-3.7-flash"});
    let (status, patched) = call(router.clone(), "PATCH",
        &format!("/api/companion/companions/{companion_id}"), json!({"robot":{"model":robot_model}})).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["data"]["model"], f.model, "Robot defaults do not overwrite the desktop profile model");
    let registry = &services.robot.as_ref().expect("robot fixture service").registry;
    for id in ["role-device-a", "role-device-b"] {
        let (robot, _) = registry.upsert_on_report(nomifun_robot::registry::RobotReport {
            robot_id:id.into(), client_id:format!("client-{id}"), board:"fixture-board".into(),
            firmware_version:"test".into(),
        }, 1).await.unwrap();
        registry.claim(robot.activation_code.as_deref().unwrap(), companion_id).await.unwrap();
    }
    let (status, saved) = call(router.clone(), "PUT", &robot_path, json!({
        "selection":robot_selection, "model":robot_model,
        "resource_selections":[{"resource_kind":"robot", "resource_id":"role-device-a"}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let a = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some("role-device-a")).await.unwrap();
    let b = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some("role-device-b")).await.unwrap();
    assert_ne!(desktop.conversation_id, a.conversation_id);
    assert_ne!(a.conversation_id, b.conversation_id);
    let a_before = projection(router, &a.conversation_id).await;
    let b_before = projection(router, &b.conversation_id).await;
    for (current, id) in [(&a_before, "role-device-a"), (&b_before, "role-device-b")] {
        assert_eq!(current["extra"]["companion_role"], "robot");
        assert_eq!(current["extra"]["product_agent_target_kind"], "robot");
        assert_eq!(current["agent_snapshot"]["resolved_model"]["provider_id"], robot_model["provider_id"]);
        assert_eq!(device(current)["resource_id"], id, "exact device creation overrides the cached role device");
        for kind in ["companion", "companion_memory"] {
            assert!(binding(current)["typed_resource_bindings"].as_array().unwrap().iter().any(|resource|
                resource["resource_kind"] == kind && resource["resource_id"] == *companion_id));
        }
    }
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let canonical = store.get_live_session(&AgentSessionId::from(a.conversation_id.clone())).await.unwrap();
    let scope = canonical.metadata.companion.unwrap();
    assert_eq!(scope.role, CompanionSessionRole::Robot);
    assert_eq!(scope.robot_id.as_deref(), Some("role-device-a"));
    let reopened = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Robot, Some("role-device-a")).await.unwrap();
    assert_eq!(reopened.conversation_id, a.conversation_id);
    let (status, authorized) = call(router.clone(), "PUT", &desktop_path, json!({
        "selection":robot_selection, "model":f.model, "conversation_id":desktop.conversation_id, "update_default":false,
        "resource_selections":[{"resource_kind":"robot", "resource_id":"role-device-a"}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{authorized}");
    let desktop_before = projection(router, &desktop.conversation_id).await;
    assert_eq!(desktop_before["extra"]["companion_role"], "companion", "selecting capabilities does not retarget the desktop receive lane");
    assert_eq!(device(&desktop_before)["resource_id"], "role-device-a", "physical operations retain a specific owned device");
    assert!(desktop_before["agent_snapshot"]["enabled_capabilities"].as_array().unwrap().contains(&json!("robot")),
        "the desktop role may explicitly select physical capabilities or the Robot template");
    assert_eq!(product_default(&f, "companion").await, desktop_default);

    // Future defaults may change while existing Sessions retain their exact device.
    let (status, saved) = call(router.clone(), "PUT", &robot_path, json!({
        "selection":robot_selection, "model":robot_model,
        "resource_selections":[{"resource_kind":"robot", "resource_id":"role-device-b"}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let robot_default = product_default(&f, "robot").await;
    assert_eq!(binding(&projection(router, &a.conversation_id).await), binding(&a_before));
    assert_eq!(binding(&projection(router, &b.conversation_id).await), binding(&b_before));
    let (status, retargeted) = call(router.clone(), "PUT", &robot_path, json!({
        "selection":robot_selection, "model":robot_model, "conversation_id":a.conversation_id, "update_default":false,
        "resource_selections":[{"resource_kind":"robot", "resource_id":"role-device-b"}],
    })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{retargeted}");
    assert_eq!(binding(&projection(router, &a.conversation_id).await), binding(&a_before));
    assert_eq!(product_default(&f, "robot").await, robot_default);

    // Device configuration changes remain frozen until an explicit same-device apply.
    let robot_services = services.robot.as_ref().unwrap();
    let (device_tx, _device_rx) = tokio::sync::mpsc::channel(4);
    registry.connect("role-device-a", companion_id, "profile-connection").await.unwrap();
    robot_services.tools.attach("role-device-a", std::sync::Arc::new(
        nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx, "profile-connection".into())),
        vec![nomifun_robot::mcp_bridge::RobotToolDescriptor {
            device_name:"vendor.arm.pose".into(), exposed_name:"robot_vendor_arm_pose".into(),
            description:"Device gesture".into(), input_schema:json!({"type":"object"}),
        }]).await;
    let mut profile = nomifun_robot::capability_profile::RobotCapabilityProfile::default();
    profile.tool_overrides.insert("vendor.arm.pose".into(), nomifun_robot::capability_profile::RobotToolOverride {
        description:Some("A user-edited gesture description".into()), ..Default::default()
    });
    registry.set_capability_profile("role-device-a", 0, profile, &robot_services.tools).await.unwrap();
    assert_eq!(device(&projection(router, &a.conversation_id).await)["typed_parameters"]["capability_revision"], "0");
    let apply = json!({"selection":robot_selection, "conversation_id":a.conversation_id,
        "update_default":false, "resource_selections":[{"resource_kind":"robot", "resource_id":"role-device-a"}]});
    let (status, refreshed) = call(router.clone(), "PUT", &robot_path, apply.clone()).await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    let a_after = projection(router, &a.conversation_id).await;
    assert_eq!(device(&a_after)["typed_parameters"]["capability_revision"], "1");
    assert_eq!(a_after["agent_snapshot"]["resolved_model"], a_before["agent_snapshot"]["resolved_model"]);
    assert_eq!(product_default(&f, "robot").await, robot_default, "current apply leaves future defaults alone");
    let (status, noop) = call(router.clone(), "PUT", &robot_path, apply).await;
    assert_eq!(status, StatusCode::OK, "{noop}");
    assert_eq!(binding(&projection(router, &a.conversation_id).await), binding(&a_after));
    let transitions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed'")
        .bind(&a.conversation_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(transitions, 1, "same-device refresh is one transition; exact reapply is a no-op");
    assert_eq!(binding(&projection(router, &b.conversation_id).await), binding(&b_before));
    assert_eq!(binding(&projection(router, &desktop.conversation_id).await), binding(&desktop_before));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions WHERE state='live'")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 3, "desktop and both physical-device Sessions coexist");
    assert!(f.upstream.received_requests().await.unwrap().is_empty(), "configuration never invokes a model");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn deleting_companion_preserves_files_owned_by_a_separate_same_workspace_session() {
    let call = companion_agent_test_request;
    let f = fixture().await;
    let router = &f.router;
    let services = &f.services;
    let companion_id = &f.companion_id;
    let desktop = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Companion, None).await.unwrap();
    let current = projection(router, &desktop.conversation_id).await;
    let workspace = current["extra"]["workspace"].as_str().unwrap();
    let retained = std::path::Path::new(workspace).join("separate-session.txt");
    std::fs::write(&retained, "KEEP_SEPARATE_SESSION_FILES").unwrap();
    let (status, preset) = call(router.clone(), "POST", "/api/agent-presets", json!({
        "display_name":"Separate workspace reader", "document":{
            "schema_version":"1.0.0", "model_route_refs":{}, "chat_route_records":{},
            "enabled_capabilities":[{"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read"]}],
            "skill_bindings":[], "system_role_provider_overrides":{}, "persona":"Read the selected workspace",
            "instructions":"Preserve the shared files", "starter_prompts":[],
        },
    })).await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let (status, ordinary) = call(router.clone(), "POST", "/api/agent-sessions", json!({
        "preset_id":preset["data"]["preset"]["preset_id"], "model":f.model,
        "title":"Separate same-workspace Session", "workspace":workspace,
        "resource_selections":[{"resource_kind":"workspace", "resource_id":"default-workspace"}],
    })).await;
    assert_eq!(status, StatusCode::OK, "{ordinary}");
    let ordinary_id = ordinary["data"]["agent_session_id"].as_str().unwrap();
    assert!(projection(router, ordinary_id).await["extra"]["companion_session"].is_null(), "directory grants no product affiliation");
    let (status, rejected) = call(router.clone(), "DELETE",
        &format!("/api/companion/companions/{companion_id}"), json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
    assert_eq!(std::fs::read_to_string(&retained).unwrap(), "KEEP_SEPARATE_SESSION_FILES");
    assert_eq!(projection(router, ordinary_id).await["conversation_id"], ordinary_id);
    assert_eq!(projection(router, &desktop.conversation_id).await["conversation_id"], desktop.conversation_id);
    assert!(services.companion_service.get_companion(companion_id).await.is_ok(), "blocked deletion preserves the profile");
    assert!(f.upstream.received_requests().await.unwrap().is_empty());
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn companion_deletion_waits_for_the_exact_terminal_report_before_deleting_any_session() {
    use nomifun_common::{AdaptationPolicy, AgentExecutionActor, AgentExecutionEventKind,
        AgentExecutionStatus, DecisionPolicy, DelegationPolicy};
    use nomifun_db::{CreateAgentExecutionParams, IAgentExecutionRepository, NewAgentExecutionEvent,
        NewAgentExecutionParticipant, SqliteAgentExecutionRepository};
    let call = companion_agent_test_request;
    let f = fixture().await;
    let router = &f.router;
    let services = &f.services;
    let companion_id = &f.companion_id;
    let lead = services.companion_service.ensure_session_for_role(
        companion_id, nomifun_companion::CompanionRole::Companion, None).await.unwrap();
    // The newer topic sorts before the lead. A delete loop that starts
    // deleting members before checking every outbox would destroy this one.
    let topic = services.companion_service.new_companion_thread(companion_id,
        &uuid::Uuid::now_v7().to_string(), Some("Preserve every topic until delivery settles".into())).await.unwrap();
    let current = projection(router, &lead.conversation_id).await;
    let workspace = current["extra"]["workspace"].as_str().unwrap();
    assert!(nomifun_common::paths::paths_equivalent(
        std::path::Path::new(workspace).parent().unwrap(),
        &services.companion_service.companion_workspaces_root(),
    ), "the real role lifecycle must create this test's workspace directly inside its managed owner directory");
    let marker = std::path::Path::new(workspace).join("pending-report-must-preserve.txt");
    std::fs::write(&marker, "PRESERVE_UNTIL_THE_REAL_REPORT_IS_DELIVERED").unwrap();
    let snapshot = &current["agent_snapshot"];
    let participant = NewAgentExecutionParticipant {
        participant_id:uuid::Uuid::now_v7().to_string(), source_agent_id:nomifun_common::NOMI_AGENT_ID.into(),
        preset_id:Some(snapshot["preset_id"].as_str().unwrap().into()),
        preset_revision:snapshot["preset_revision"].as_i64(), agent_snapshot:Some(snapshot.to_string()),
        provider_id:Some(f.model["provider_id"].as_str().unwrap().into()), model:Some("step-3.7-flash".into()),
        role:Some("desktop_companion".into()), capability:None, constraints:None,
        description:Some("Paused before task work began".into()), system_prompt:None,
        enabled_skills:"[]".into(), disabled_builtin_skills:"[]".into(), sort_order:0,
    };
    let event = |kind: AgentExecutionEventKind, payload: Value| NewAgentExecutionEvent {
        event_type:kind, step_id:None, attempt_id:None, actor:AgentExecutionActor::system(), payload:payload.to_string(),
    };
    let repository = SqliteAgentExecutionRepository::new(services.database.pool().clone());
    let owner = services.authoritative_user_id.as_ref();
    let created = repository.create_execution_with_participants(owner, &CreateAgentExecutionParams {
        goal:"Verify pending terminal report delivery protects the Companion".into(),
        status:AgentExecutionStatus::Paused, adaptation_policy:AdaptationPolicy::Fixed,
        decision_policy:DecisionPolicy::AskUser, delegation_policy:DelegationPolicy::Disabled,
        max_parallel:1, work_dir:Some(workspace.into()), lead_conversation_id:Some(lead.conversation_id.clone()),
        initial_plan_input:json!({"mode":"automatic"}).to_string(),
    }, &[participant], &event(AgentExecutionEventKind::Created, json!({"status":"paused"}))).await.unwrap();
    let report_operation = format!("exec-lead-report:{}:event:{}", created.execution_id, created.event_sequence + 1);
    const REPORT: &str = "Execution was cancelled before task work started.";
    let terminal = repository.cancel_execution(owner, &created.execution_id, created.version,
        &event(AgentExecutionEventKind::StatusChanged, json!({
            "status":"cancelled", "lead_report_operation_id":report_operation,
        }))).await.unwrap().execution;
    assert_eq!(terminal.status, "cancelled");
    for session_path in [
        format!("/api/agent-sessions/{}", lead.conversation_id),
        format!("/api/companion/companions/{companion_id}/companion/threads/{}", lead.conversation_id),
    ] {
        let (status, pending_source) = call(router.clone(), "DELETE", &session_path, json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT, "single source deletion must preserve its report outbox: {pending_source}");
        let state: String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
            .bind(&lead.conversation_id).fetch_one(services.database.pool()).await.unwrap();
        assert_eq!(state, "live");
    }
    let path = format!("/api/companion/companions/{companion_id}");
    let (status, pending) = call(router.clone(), "DELETE", &path, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{pending}");
    for id in [&lead.conversation_id, &topic.conversation_id] {
        let state: String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
            .bind(id).fetch_one(services.database.pool()).await.unwrap();
        assert_eq!(state, "live", "every member remains live until the original report is delivered");
    }
    assert!(services.companion_service.get_companion(companion_id).await.is_ok());
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "PRESERVE_UNTIL_THE_REAL_REPORT_IS_DELIVERED");

    // Acknowledge only after a real canonical assistant projection exists for
    // that exact operation. The test does not replace report delivery with an ack.
    let canonical = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id = AgentSessionId::from(lead.conversation_id.clone());
    let before = canonical.store().read_events(&session_id, None, 100).await.unwrap();
    let cause = before.events.last().unwrap().event_id.clone();
    let message_id = uuid::Uuid::now_v7().to_string();
    let projected = canonical.store().append_event(&nomifun_agent_contracts::SessionEventAppend {
        agent_session_id:session_id.clone(), event_id:message_id.clone().into(), producer_id:"session_api".into(),
        idempotency_key:format!("agent-execution-projection:{report_operation}").into(),
        semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {
            kind:nomifun_agent_contracts::SessionEventKind("message/assistant-projected".into()), kind_version:1, correlation_id:message_id.clone().into(),
            causation_event_id:Some(cause), payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                nomifun_agent_contracts::StrictJsonValue(json!({
                    "content":REPORT, "origin":"agent_execution_report", "operation_id":report_operation,
                }))),
        },
    }).await.unwrap();
    assert!(projected.persisted);
    let durable = canonical.store().read_events(&session_id, None, 100).await.unwrap();
    assert!(durable.events.iter().any(|source| source.event_id.as_ref() == message_id
        && source.kind.0 == "message/assistant-projected"
        && matches!(&source.payload, nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(payload)
            if payload.0["operation_id"] == report_operation && payload.0["content"] == REPORT)));
    repository.append_event(owner, &created.execution_id, terminal.version,
        &event(AgentExecutionEventKind::StatusChanged, json!({
            "change":"lead_report_delivered", "operation_id":report_operation, "projection_suppressed":false,
        }))).await.unwrap();
    let (status, deleted) = call(router.clone(), "DELETE", &path, json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the exact original report was delivered before retry: {deleted}");
    assert!(deleted.is_null(), "the successful deletion follows the route's empty response contract");
    for id in [&lead.conversation_id, &topic.conversation_id] {
        let state: String = sqlx::query_scalar("SELECT state FROM agent_sessions WHERE agent_session_id=?")
            .bind(id).fetch_one(services.database.pool()).await.unwrap();
        assert_eq!(state, "deleted");
    }
    assert!(matches!(services.companion_service.get_companion(companion_id).await,
        Err(nomifun_common::AppError::NotFound(_))));
    assert!(!marker.exists(), "only settled deletion removes the owned workspace file");
    assert!(!std::path::Path::new(workspace).exists(), "the domain's final directory check must remove the whole verified owned leaf");
    assert!(services.companion_service.companion_workspaces_root().is_dir(), "deletion must preserve the managed owner root itself");
    assert!(f.upstream.received_requests().await.unwrap().is_empty(), "report projection does not start another model Turn");
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

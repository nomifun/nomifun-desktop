//! Real canonical owner acceptance. Everything, including the Chat transport,
//! belongs to a temporary fixture; no application user data is opened.
use super::*;
use axum::{Router, body::Body, http::Request};
use std::{fs, time::Duration};
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const TRUST: &str = "session-management-owner-fixture";

async fn request(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(status.is_success(), "{method} {path}: {status} {body}");
    body["data"].clone()
}

fn input(content: &str) -> nomifun_api_types::SendMessageRequest {
    serde_json::from_value(json!({"content":content,"origin":"agent"})).unwrap()
}

#[tokio::test]
async fn real_owner_creation_and_send_replay_use_exact_input_without_starting_a_second_turn() {
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let mut services = crate::services::AppServices::from_config(
        database,
        &crate::AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(TRUST.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let skills = nomifun_skill_library::SkillPaths {
        data_dir: services.data_dir.clone(),
        user_skills_dir: services.data_dir.join("skills"),
        cron_skills_dir: services.data_dir.join("cron/skills"),
        builtin_skills_dir: services.data_dir.join("builtin-skills"),
        builtin_rules_dir: services.data_dir.join("builtin-rules"),
    };
    for path in [
        &skills.user_skills_dir,
        &skills.cron_skills_dir,
        &skills.builtin_skills_dir,
        &skills.builtin_rules_dir,
    ] {
        fs::create_dir_all(path).unwrap();
    }
    services.skill_paths = Arc::new(skills);
    let (states, _components) = super::super::state::try_build_module_states(&services)
        .await
        .unwrap();
    let owner = states.nomi_core_agent_api.session_owner.clone();
    let api = states.nomi_core_agent_api.clone();
    let management_lease = Arc::downgrade(&states._session_management_state);
    let router = super::super::create_router_with_states(&services, states);
    assert!(
        management_lease.upgrade().is_some(),
        "Router must retain the late owner lease after ModuleStates is consumed"
    );
    let chat = MockServer::start().await;
    let message = json!({"id":"management-fixture","choices":[{"index":0,"delta":{"role":"assistant","content":"fixture completed"},"finish_reason":null}]});
    let finish = json!({"id":"management-fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!(
                    "data: {message}\n\ndata: {finish}\n\ndata: [DONE]\n\n"
                ))
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&chat)
        .await;
    let provider=request(&router,"POST","/api/providers",json!({
        "platform":"custom","name":"management fixture","base_url":format!("{}/v1",chat.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["fixture-secret"]},"enabled":true,
        "initial_model":{"model":"fixture-chat","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default"}]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"fixture-chat"});
    let library = request(&router, "GET", "/api/agent-preset-templates", Value::Null).await;
    let official_id = library["official_presets"]["chat.minimal"]["preset_id"].as_str().unwrap();
    assert!(library["official_presets"]["chat.minimal"]["current_stable_revision"].is_null());
    let official_key = uuid::Uuid::now_v7().to_string();
    let official_create = || super::super::nomi_core_session::management_create_agent_session(
        api.clone(), &services.authoritative_user_id, official_id,
        Some(serde_json::from_value(model.clone()).unwrap()), Some("Current official management target".into()),
        &official_key,
    );
    let official = official_create().await.unwrap();
    assert_eq!(official_create().await.unwrap().agent_session_id, official.agent_session_id);
    assert!(!official.agent_binding.typed_resource_bindings.is_empty(), "listed official Agents must prepare real canonical resources on first use");
    let library = request(&router, "GET", "/api/agent-preset-templates", Value::Null).await;
    assert!(library["official_presets"]["chat.minimal"]["current_stable_revision"].is_null(),
        "management preparation cannot publish the official author's configuration");
    let preset = request(
        &router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({"reuse_existing":false,"display_name":"Target minimal Agent","model":model}),
    )
    .await;
    let creation_key = invocation_key(
        &uuid::Uuid::now_v7().to_string().into(),
        "session.management/create",
        &format!("agent-tool:{}", "a".repeat(64)),
    )
    .unwrap();
    let create = || {
        super::super::nomi_core_session::management_create_agent_session(
            api.clone(),
            &services.authoritative_user_id,
            preset["preset"]["preset_id"].as_str().unwrap(),
            Some(serde_json::from_value(model.clone()).unwrap()),
            Some("Managed fixture target".into()),
            &creation_key,
        )
    };
    let first = create().await.unwrap();
    let replay = create().await.unwrap();
    assert_eq!(first.agent_session_id, replay.agent_session_id);
    let target: AgentSessionId = first.agent_session_id.into();
    let binding = first.agent_binding.binding_version;
    let key = invocation_key(
        &uuid::Uuid::now_v7().to_string().into(),
        "session.management/send_input",
        "fixture-host-operation",
    )
    .unwrap();
    let accepted = owner
        .management_send_input_idempotent(
            &services.authoritative_user_id,
            &target,
            &key,
            input("Complete the fixture request"),
            binding,
            0,
        )
        .await
        .unwrap()
        .unwrap();
    let duplicate = owner
        .management_send_input_idempotent(
            &services.authoritative_user_id,
            &target,
            &key,
            input("Complete the fixture request"),
            binding,
            0,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(duplicate.replayed);
    assert_eq!(accepted.message_id, duplicate.message_id);
    assert!(
        owner
            .management_send_input_idempotent(
                &services.authoritative_user_id,
                &target,
                &key,
                input("Different input under the original key"),
                binding,
                0
            )
            .await
            .is_err()
    );
    assert!(
        owner
            .management_send_input_idempotent(
                &services.authoritative_user_id,
                &target,
                "fixture-other-turn",
                input("Cannot enter an active target"),
                binding,
                0
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        owner
            .management_send_input_idempotent(
                &uuid::Uuid::now_v7().to_string(),
                &target,
                "fixture-foreign-owner",
                input("Foreign input"),
                binding,
                0
            )
            .await
            .is_err()
    );
    let page = owner
        .canonical()
        .store()
        .read_events(&target, None, 100)
        .await
        .unwrap();
    assert_eq!(
        page.events
            .iter()
            .filter(|event| event.kind.0 == "message/user-accepted")
            .count(),
        1
    );
    let active = owner
        .canonical()
        .store()
        .head(&target)
        .await
        .unwrap()
        .active_turn_id
        .unwrap();
    let cancelled = owner
        .cancel_management_exact_turn(
            &services.authoritative_user_id,
            &target,
            "fixture-cancel",
            &active.into(),
        )
        .await
        .unwrap();
    assert!(!cancelled.duplicate);
    let cancelled_replay = owner
        .cancel_management_exact_turn(
            &services.authoritative_user_id,
            &target,
            "fixture-cancel",
            &cancelled.target_operation_id,
        )
        .await
        .unwrap();
    assert!(cancelled_replay.duplicate);
    assert_eq!(cancelled.event_id, cancelled_replay.event_id);
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

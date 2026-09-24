//! IDMM AgentSession route contract: owner scoping, opt-in defaults and strict
//! validation for the bypass-model tier.

mod common;

use axum::http::StatusCode;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, CorrelationId, DigestHex, EventProducerId, IdempotencyKey,
    OperationId, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef,
    StrictJsonValue,
};
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, get_with_token, json_with_token, setup_and_login};

async fn seed_chat_model(
    services: &nomifun_app::compatibility::AppServices,
    provider_id: &str,
    model: &str,
) {
    nomifun_db::sqlx::query(
        "INSERT INTO providers \
         (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
          created_at, updated_at) \
         VALUES (?, 'openai', 'IDMM Sidecar', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
    )
    .bind(provider_id)
    .bind(
        nomifun_common::encrypt_string(
            r#"{"api_keys":["test-only"]}"#,
            &services.encryption_key,
        )
        .unwrap(),
    )
    .execute(services.database.pool())
    .await
    .unwrap();
    common::seed_openai_chat_model(services.database.pool(), provider_id, model).await;
}

async fn seed_session(
    services: &nomifun_app::compatibility::AppServices,
    session_id: &str,
) {
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let session = AgentSessionLiveRecord {
        agent_session_id: AgentSessionId::from(session_id.to_owned()),
        owner_ref: PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: services.authoritative_user_id.to_string(),
        },
        metadata: AgentSessionMetadata {
            title: Some("IDMM E2E".to_owned()),
            archived: false,
            pinned: false,
            reasoning_effort: None,
        },
        agent_binding: AgentBindingValue {
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from("idmm-e2e-preset"),
                revision: 1,
                revision_digest: DigestHex::from("a".repeat(64)),
            },
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("idmm-e2e-snapshot"),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
            typed_resource_bindings: Vec::new(),
            binding_version: 1,
        },
        remote_binding_provenance: None,
        parent_session_id: None,
        fork_base_payload_id: None,
        next_seq: 1,
    };
    let key = format!("idmm-e2e:{session_id}");
    let created = store
        .create_session(nomifun_agent_session::CreateSessionRequest::new(
            session,
            1,
            OperationId::from(format!("{key}:open")),
            EventProducerId::from("session_api"),
            IdempotencyKey::from(format!("{key}:open")),
            CorrelationId::from(format!("{key}:open")),
        ))
        .await
        .unwrap();
    store
        .append_event(&SessionEventAppend {
            agent_session_id: AgentSessionId::from(session_id.to_owned()),
            event_id: nomifun_agent_contracts::EventId::from(format!("{key}:ready")),
            producer_id: EventProducerId::from("runtime_supervisor"),
            idempotency_key: IdempotencyKey::from(format!("{key}:ready")),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("session/ready".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(format!("{key}:ready")),
                causation_event_id: Some(created.opening_ack.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
            },
        })
        .await
        .unwrap();
}

async fn project_assistant_message(
    services: &nomifun_app::compatibility::AppServices,
    session_id: &str,
    content: &str,
) {
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let message_id = uuid::Uuid::now_v7().to_string();
    store
        .append_event(&SessionEventAppend {
            agent_session_id: AgentSessionId::from(session_id.to_owned()),
            event_id: nomifun_agent_contracts::EventId::from(message_id.clone()),
            producer_id: EventProducerId::from("session_api"),
            idempotency_key: IdempotencyKey::from(format!("idmm-e2e-message:{message_id}")),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("message/assistant-projected".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from(message_id),
                causation_event_id: Some(nomifun_agent_contracts::EventId::from(format!(
                    "idmm-e2e:{session_id}:ready"
                ))),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "content": content
                }))),
            },
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn agent_runtime_policy_is_frozen_into_new_session_without_overwriting_an_override() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let provider_id = "0190f5fe-7c00-7a00-8000-000000000089";
    seed_chat_model(&services, provider_id, "primary").await;

    let response = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/agent-presets/from-template/chat.minimal",
            json!({
                "display_name": "IDMM policy Agent",
                "reuse_existing": false,
                "model": {"provider_id": provider_id, "model": "primary"}
            }),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let editor = body_json(response).await["data"].clone();
    let preset_id = editor["preset"]["preset_id"].as_str().unwrap().to_owned();
    let mut draft = editor["draft"].clone();
    draft["document"]["runtime_policy"]["idmm"] = json!({
        "mode": "rule_only",
        "idle_timeout_secs": 120
    });
    let expected_current_revision = draft["current_revision"].clone();
    let response = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            &format!("/api/agent-presets/{preset_id}/revisions"),
            json!({
                "expected_current_revision": expected_current_revision,
                "draft": draft
            }),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    let status = response.status();
    let saved = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["data"]["revision"]["document"]["runtime_policy"]["idmm"]["mode"],
        "rule_only"
    );

    let create_body = json!({"preset_id": preset_id, "title": "Inherited IDMM"});
    let create_key = "idmm-agent-policy-create";
    let create_request = || {
        let mut request = json_with_token(
            "POST",
            "/api/agent-sessions",
            create_body.clone(),
            &token,
            &csrf,
        );
        request.headers_mut().insert(
            "idempotency-key",
            axum::http::HeaderValue::from_static(create_key),
        );
        request
    };
    let response = app.clone().oneshot(create_request()).await.unwrap();
    let status = response.status();
    let created = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let session_id = created["data"]["agent_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/api/agent-sessions/{session_id}/idmm");
    let inherited = body_json(
        app.clone()
            .oneshot(get_with_token(&path, &token))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(inherited["data"]["revision"], 1);
    assert_eq!(inherited["data"]["config"]["mode"], "rule_only");
    assert_eq!(inherited["data"]["config"]["idle_timeout_secs"], 120);

    let response = app
        .clone()
        .oneshot(json_with_token(
            "PUT",
            &path,
            json!({"mode": "off"}),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app.clone().oneshot(create_request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let replay = body_json(response).await;
    assert_eq!(replay["data"]["agent_session_id"], session_id);
    let overridden = body_json(app.oneshot(get_with_token(&path, &token)).await.unwrap()).await;
    assert_eq!(overridden["data"]["revision"], 2);
    assert_eq!(overridden["data"]["config"]["mode"], "off");
}

#[tokio::test]
async fn idmm_defaults_off_and_rule_config_roundtrips() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let session_id = uuid::Uuid::now_v7().to_string();
    seed_session(&services, &session_id).await;
    let path = format!("/api/agent-sessions/{session_id}/idmm");

    let response = app
        .clone()
        .oneshot(get_with_token(&path, &token))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8_lossy(&bytes);
    assert_eq!(status, StatusCode::OK, "{raw}");
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["data"]["config"]["mode"], "off");
    assert_eq!(body["data"]["run_state"], "off");

    let response = app
        .clone()
        .oneshot(json_with_token(
            "PUT",
            &path,
            json!({"mode":"rule_only"}),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["config"]["mode"], "rule_only");
    assert_eq!(body["data"]["run_state"], "monitoring");
    assert_eq!(body["data"]["revision"], 1);

    let response = app.oneshot(get_with_token(&path, &token)).await.unwrap();
    assert_eq!(body_json(response).await["data"]["config"]["mode"], "rule_only");
}

#[tokio::test]
async fn rule_plus_model_requires_an_explicit_bypass_model() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let session_id = uuid::Uuid::now_v7().to_string();
    seed_session(&services, &session_id).await;
    let response = app
        .clone()
        .oneshot(json_with_token(
            "PUT",
            &format!("/api/agent-sessions/{session_id}/idmm"),
            json!({"mode":"rule_plus_model"}),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let provider_id = "0190f5fe-7c00-7a00-8000-000000000088";
    seed_chat_model(&services, provider_id, "sidecar").await;
    let response = app
        .oneshot(json_with_token(
            "PUT",
            &format!("/api/agent-sessions/{session_id}/idmm"),
            json!({
                "mode":"rule_plus_model",
                "bypass_model":{"provider_id":provider_id,"model":"sidecar"}
            }),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn evaluate_reads_canonical_message_projection_and_reserves_rule_action() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let session_id = uuid::Uuid::now_v7().to_string();
    seed_session(&services, &session_id).await;
    project_assistant_message(
        &services,
        &session_id,
        "请选择下一步：\n1. 删除所有数据\n2. 继续分析（推荐）\n3. 取消",
    )
    .await;
    let path = format!("/api/agent-sessions/{session_id}/idmm");
    let response = app
        .clone()
        .oneshot(json_with_token(
            "PUT",
            &path,
            json!({"mode":"rule_only","min_interval_secs":0}),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(json_with_token(
            "POST",
            &format!("{path}/evaluate"),
            json!({}),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["data"]["recent_interventions"][0]["kind"], "option_decision");
    assert_eq!(
        body["data"]["recent_interventions"][0]["action"],
        "select_safe_option"
    );
    assert_eq!(body["data"]["recent_interventions"][0]["status"], "failed");
}

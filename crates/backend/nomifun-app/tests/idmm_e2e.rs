//! IDMM AgentSession route contract: owner scoping, opt-in defaults and strict
//! validation for the bypass-model tier.

mod common;

use axum::http::StatusCode;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, CorrelationId, DigestHex, EventProducerId, IdempotencyKey,
    IdmmDecisionNotice, IdmmDecisionNoticeStatus, IdmmDecisionSource,
    OperationId, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef,
    StrictJsonValue,
};
use serde_json::json;
use serde_json::Value;
use std::time::Duration;
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
            purpose: Default::default(),
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
    let status=response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK,"{body}");
    assert_eq!(body["data"]["recent_interventions"][0]["kind"], "option_decision");
    assert_eq!(
        body["data"]["recent_interventions"][0]["action"],
        "select_safe_option"
    );
    assert_eq!(body["data"]["recent_interventions"][0]["status"], "failed");
}

#[tokio::test]
async fn public_turn_routes_reject_forged_idmm_sources_before_admission() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let session_id=uuid::Uuid::now_v7().to_string();
    seed_session(&services,&session_id).await;
    for input in [json!({"content":"hello","origin":"idmm"}),
        json!({"content":"hello","idmm_decision":null}),
        json!({"content":"hello","input_source":{"kind":"idmm"}})] {
        let response=app.clone().oneshot(json_with_token("POST",&format!("/api/agent-sessions/{session_id}/turns"),
            json!({"input":input,"idempotency_key":uuid::Uuid::now_v7().to_string()}),&token,&csrf)).await.unwrap();
        assert_eq!(response.status(),StatusCode::BAD_REQUEST);
    }
    let turns:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(&session_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(turns,0,"source forgery must not create an accepted input or model request");
}

fn scripted_chat(request: &wiremock::Request) -> wiremock::ResponseTemplate {
    let body:Value=serde_json::from_slice(&request.body).unwrap();
    let messages=body["messages"].as_array().unwrap();
    let system=messages.iter().filter(|message| message["role"]=="system")
        .map(|message|message["content"].to_string()).collect::<String>();
    let last=messages.last().unwrap()["content"].to_string();
    let text=if system.contains("constrained decision sidecar") {
        r#"{"action":"answer_text","text":"简单","reason":"UI basis remains metadata only."}"#
    } else if last.contains("rule_case") {
        "请选择实现方式：\n1. React\n2. HTML（推荐）"
    } else if last.contains("sidecar_case") {
        "请问初始难度设置成什么？"
    } else { "设计已确认。" };
    scripted_response(text)
}

fn scripted_response(text: &str) -> wiremock::ResponseTemplate {
    let delta=json!({"id":"idmm-scripted-provider","choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]});
    let done=json!({"id":"idmm-scripted-provider","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
    wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
        .set_body_string(format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
}

fn stored_content(row:&Value) -> Value {
    if let Some(content)=row["content"].as_str() { serde_json::from_str(content).unwrap() }
    else { row["content"].clone() }
}

#[tokio::test]
async fn invalid_sidecar_decision_reaches_both_history_routes_as_the_same_typed_failed_notice() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(scripted_response("invalid JSON")).mount(&server).await;
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let provider_id = uuid::Uuid::now_v7().to_string();
    seed_chat_model(&services, &provider_id, "sidecar").await;
    nomifun_db::sqlx::query("UPDATE providers SET base_url = ? WHERE provider_id = ?")
        .bind(format!("{}/v1", server.uri())).bind(&provider_id)
        .execute(services.database.pool()).await.unwrap();
    let session_id = uuid::Uuid::now_v7().to_string();
    seed_session(&services, &session_id).await;
    project_assistant_message(&services, &session_id, "请问初始难度设置成什么？").await;
    let path = format!("/api/agent-sessions/{session_id}/idmm");
    let response = app.clone().oneshot(json_with_token("PUT", &path,
        json!({"mode":"rule_plus_model", "scan_interval_secs":300, "min_interval_secs":0,
            "bypass_model":{"provider_id":provider_id,"model":"sidecar"}}), &token, &csrf,
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app.clone().oneshot(json_with_token("POST", &format!("{path}/evaluate"),
        json!({}), &token, &csrf,
    )).await.unwrap();
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["recent_interventions"][0]["status"], "failed");
    assert_eq!(body["data"]["recent_interventions"][0]["reason"], "bypass_model_failed");
    let turns: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id = ?")
        .bind(&session_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(turns, 0, "an invalid sidecar answer must never be admitted as a primary-model Turn");
    let notices: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id = ? AND kind = 'idmm/notice-recorded'",
    ).bind(&session_id).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(notices, 1, "the still-unanswered question must have one durable canonical failed notice");

    let response = app.clone().oneshot(get_with_token(
        &format!("/api/agent-sessions/{session_id}/message-history?page_size=100"), &token,
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let history = body_json(response).await;
    let row = history["data"]["items"].as_array().unwrap().iter()
        .find(|row| stored_content(row)["idmm_notice"].is_object())
        .expect("the invalid sidecar answer must publish a notice for the still-unanswered question");
    let notice: IdmmDecisionNotice = serde_json::from_value(stored_content(row)["idmm_notice"].clone()).unwrap();
    notice.validate().unwrap();
    assert_eq!(notice.status, IdmmDecisionNoticeStatus::Failed);
    assert_eq!(notice.decision.source, IdmmDecisionSource::BypassModel);
    assert_eq!(notice.decision.reason_code, "bypass_model_failed");
    assert_eq!(notice.decision.model.as_ref().unwrap().provider_id, provider_id);
    let response = app.oneshot(get_with_token(
        &format!("/api/agent-sessions/{session_id}/message-history/{}", row["message_id"].as_str().unwrap()), &token,
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let single = body_json(response).await;
    let single_notice: IdmmDecisionNotice = serde_json::from_value(stored_content(&single["data"])["idmm_notice"].clone()).unwrap();
    assert_eq!(single_notice, notice,
        "cold history and single-message history must expose the same typed failed notice");
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.is_empty(), "the failure must come from the actual sidecar provider call");
    assert!(requests.iter().all(|request| String::from_utf8_lossy(&request.body).contains("constrained decision sidecar")),
        "the invalid decision must not start a primary-model request");
}

#[tokio::test]
async fn canonical_idmm_decisions_reach_ui_history_with_exact_sources_and_no_prompt_metadata() {
    let server=wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(scripted_chat).mount(&server).await;
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let provider_id=uuid::Uuid::now_v7().to_string();
    seed_chat_model(&services,&provider_id,"primary").await;
    nomifun_db::sqlx::query("UPDATE providers SET base_url=? WHERE provider_id=?")
        .bind(format!("{}/v1",server.uri())).bind(&provider_id).execute(services.database.pool()).await.unwrap();
    let preset=body_json(app.clone().oneshot(json_with_token("POST","/api/agent-presets/from-template/chat.minimal",
        json!({"display_name":"Canonical IDMM UI","reuse_existing":false,"model":{"provider_id":provider_id,"model":"primary"}}),
        &token,&csrf)).await.unwrap()).await;
    let preset_id=preset["data"]["preset"]["preset_id"].as_str().unwrap();
    for (case,source,expected) in [("rule_case","rule","2"),("sidecar_case","bypass_model","简单")] {
        let session=body_json(app.clone().oneshot(json_with_token("POST","/api/agent-sessions",
            json!({"preset_id":preset_id,"title":case}),&token,&csrf)).await.unwrap()).await;
        let sid=session["data"]["agent_session_id"].as_str().unwrap();
        let response=app.clone().oneshot(json_with_token("PUT",&format!("/api/agent-sessions/{sid}/idmm"),
            json!({"mode":"rule_plus_model","scan_interval_secs":5,"min_interval_secs":0,
                "bypass_model":{"provider_id":provider_id,"model":"primary"}}),&token,&csrf)).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        let response=app.clone().oneshot(json_with_token("POST",&format!("/api/agent-sessions/{sid}/turns"),
            json!({"input":{"content":case},"idempotency_key":uuid::Uuid::now_v7().to_string()}),&token,&csrf)).await.unwrap();
        let status=response.status();
        let body=body_json(response).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        let row=tokio::time::timeout(Duration::from_secs(25),async {
            loop {
                let response=app.clone().oneshot(get_with_token(&format!("/api/agent-sessions/{sid}/message-history?page_size=100"),&token)).await.unwrap();
                assert_eq!(response.status(),StatusCode::OK);
                let body=body_json(response).await;
                if let Some(row)=body["data"]["items"].as_array().unwrap().iter()
                    .find(|row| stored_content(row)["idmm_decision"].is_object()) {
                    let turn_state: Option<String> = nomifun_db::sqlx::query_scalar(
                        "SELECT state FROM agent_turns WHERE session_id = ? AND source_message_id = ?",
                    ).bind(sid).bind(row["message_id"].as_str().unwrap())
                        .fetch_optional(services.database.pool()).await.unwrap();
                    if turn_state.as_deref() == Some("completed") {
                        break row.clone();
                    }
                    assert!(!matches!(turn_state.as_deref(), Some("failed" | "cancelled" | "interrupted")),
                        "the automatic reply must complete its primary-model Turn: {turn_state:?}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.expect("the real background supervisor must deliver a typed decision and complete its primary-model Turn");
        let content=stored_content(&row);
        assert_eq!(content["content"],expected);
        assert_eq!(content["idmm_decision"]["source"],source);
        assert!(content["idmm_decision"]["question"]["message_id"].as_str().is_some());
        assert!(!content["idmm_decision"]["rationale"].as_str().unwrap().is_empty());
        if source=="rule" {
            assert!(content["idmm_decision"]["model"].is_null());
            assert_eq!(content["idmm_decision"]["reason_code"],"rule_selected_recommended_option");
        } else {
            assert_eq!(content["idmm_decision"]["model"]["model"],"primary");
            assert_eq!(content["idmm_decision"]["rationale"],"UI basis remains metadata only.");
        }
        let response=app.clone().oneshot(get_with_token(&format!("/api/agent-sessions/{sid}/message-history/{}",row["message_id"].as_str().unwrap()),&token)).await.unwrap();
        assert_eq!(stored_content(&body_json(response).await["data"]),content,
            "annotation fetch and cold history must return the same immutable provenance");
        let response=app.clone().oneshot(json_with_token("PUT",&format!("/api/agent-sessions/{sid}/idmm"),
            json!({"mode":"off"}),&token,&csrf)).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
    }
    let requests=server.received_requests().await.unwrap();
    let main_requests:Vec<_>=requests.iter().filter(|request|!String::from_utf8_lossy(&request.body).contains("constrained decision sidecar")).collect();
    assert!(main_requests.len() >= 4,
        "both initial prompts and both automatic replies must reach the primary model before checking metadata isolation; saw {} requests", main_requests.len());
    assert!(main_requests.iter().all(|request|!String::from_utf8_lossy(&request.body).contains("UI basis remains metadata only.")),
        "the UI-only rationale must not become primary-model instructions or history text");
}

//! Shares the real source/publication harness with discovery, not an executor.
use super::*;
use nomifun_ai_agent::model_middleware as middleware;

const SOURCE: &str = r#"
export async function start() {
  return {
    async invoke({method, payload, signal}) {
      if (method !== 'agent.before_model' || payload.phase !== 'before_model') throw new Error('bad phase');
      if (Object.keys(payload).sort().join(',') !== 'phase,system,tools,turn') throw new Error('unexpected authority');
      if (!payload.turn.source_message_id) throw new Error('missing turn identity');
      if (!payload.tools.some(t => t.name === 'ToolSearch')) throw new Error('missing real candidate');
      if (payload.system.includes('PRODUCT_MIDDLEWARE:')) throw new Error('patch persisted');
      for (const tool of payload.tools) {
        if (Object.keys(tool).sort().join(',') !== 'description,name') throw new Error('schema leaked');
      }
      if (payload.turn.text === 'outside') return {tool_names: ['not-authorized']};
      if (payload.turn.text === 'wait') return new Promise((resolve, reject) => {
        signal.addEventListener('abort', () => reject(new Error('cancelled')), {once:true});
      });
      return {system: 'PRODUCT_MIDDLEWARE:' + payload.turn.text, tool_names: []};
    },
    async dispose() {},
  };
}
"#;

fn capability(id: &str) -> CapabilityManifest {
    let mut manifest = discovery_capability(id);
    manifest.id = format!("plugin.{id}.before-model").into();
    manifest.contribution_id = format!("capability:{}", manifest.id.as_ref()).into();
    manifest.kind = CapabilityKind::TurnMiddleware;
    manifest.display.name = "Before model middleware".into();
    manifest.contributions.actions = vec![middleware::action()];
    manifest
}

#[tokio::test]
async fn product_middleware_order_is_saved_reused_and_frozen_per_session() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    // Wrapping is noncommutative: B(A(prompt)) proves A then B actually ran.
    let source = r#"export async function start() {
      return { async invoke({payload}) { return {system: 'TAG(' + payload.system + ')'}; }, async dispose() {} };
    }"#;
    let mut entries = Vec::new();
    for tag in ["A", "B"] {
        let (id, _) = publish_source(
            &router,
            &source.replace("TAG", tag),
            capability,
            middleware::schemas(),
        )
        .await;
        entries.push((capability(&id).id.as_ref().to_owned(), tag));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(|request: &wiremock::Request| {
        let body = request.body_json::<Value>().unwrap();
        let text = body["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "user").unwrap()["content"].as_str().unwrap();
        let frame = json!({"choices":[{"index":0,"delta":{"content":format!("MIDDLEWARE_DONE_{text}")},"finish_reason":"stop"}]});
        wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {frame}\n\ndata: [DONE]\n\n"))
    }).expect(4).mount(&upstream).await;
    let provider = data(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Ordered middleware model", "base_url":format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash", "enabled":true, "capabilities":[{
            "task":"chat", "traits":["function_calling","streaming"], "protocol":"openai.chat_text", "connection_role":"default", "provider_params":{}
        }]}
    })).await;
    let preset = data(
        &router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name":"Ordered middleware", "reuse_existing":false,
            "model":{"provider_id":provider["provider_id"], "model":"step-3.7-flash"}
        }),
    )
    .await;
    let preset_id = preset["preset"]["preset_id"].as_str().unwrap();
    let editor_path = format!("/api/agent-presets/{preset_id}/editor");
    let save_path = format!("/api/agent-presets/{preset_id}/revisions");
    let editor = data(&router, "GET", &editor_path, Value::Null).await;
    let mut draft = editor["draft"].clone();
    for (id, _) in &entries {
        draft["document"]["enabled_capabilities"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "capability":{"id":id,"version":"1.0.0"},
                "action_allowlist":[middleware::ACTION_ID]
            }));
    }
    let selected: Vec<nomifun_agent_contracts::CapabilitySelection> =
        serde_json::from_value(draft["document"]["enabled_capabilities"].clone()).unwrap();
    let original = data(
        &router,
        "POST",
        &save_path,
        json!({"expected_current_revision":draft["current_revision"], "draft":draft}),
    )
    .await;
    assert!(original["resolved_snapshot_ref"].is_object(), "{original}");
    let old_session = data(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"title":"Original order"}),
    )
    .await;
    let old_id = old_session["agent_session_id"].as_str().unwrap();
    finish_turn(&router, old_id, "default-order", true).await;

    draft["current_revision"] = original["revision"]["reference"].clone();
    // Unlisted selected middleware must follow explicit entries, not disappear.
    draft["document"]["middleware_order"] = json!([entries[1].0]);
    let changed = data(
        &router,
        "POST",
        &save_path,
        json!({"expected_current_revision":draft["current_revision"], "draft":draft}),
    )
    .await;
    assert_ne!(
        changed["revision"]["reference"],
        original["revision"]["reference"]
    );
    assert_ne!(
        changed["resolved_snapshot_ref"],
        original["resolved_snapshot_ref"]
    );
    let editor = data(&router, "GET", &editor_path, Value::Null).await;
    assert_eq!(
        editor["draft"]["document"]["middleware_order"],
        json!([entries[1].0])
    );
    assert_eq!(
        serde_json::from_value::<Vec<nomifun_agent_contracts::CapabilitySelection>>(
            editor["draft"]["document"]["enabled_capabilities"].clone()
        )
        .unwrap(),
        selected
    );
    draft = editor["draft"].clone();
    let reused = data(
        &router,
        "POST",
        &save_path,
        json!({"expected_current_revision":draft["current_revision"], "draft":draft}),
    )
    .await;
    assert_eq!(
        reused["resolved_snapshot_ref"],
        changed["resolved_snapshot_ref"]
    );
    assert_eq!(
        reused["revision"]["reference"],
        changed["revision"]["reference"]
    );
    let new_session = data(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"title":"Explicit order"}),
    )
    .await;
    finish_turn(
        &router,
        new_session["agent_session_id"].as_str().unwrap(),
        "partial-order",
        true,
    )
    .await;
    finish_turn(&router, old_id, "still-frozen", true).await;

    for order in [
        json!([entries[1].0, entries[1].0]),
        json!(["plugin.not-selected.before-model"]),
    ] {
        let mut invalid = draft.clone();
        invalid["document"]["middleware_order"] = order;
        let (status, error) = request(
            &router,
            "POST",
            &save_path,
            json!({"expected_current_revision":draft["current_revision"], "draft":invalid}),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert!(error.to_string().contains("middleware_order"), "{error}");
    }
    // Full explicit order also round trips; rejected saves left the revision intact.
    draft["document"]["middleware_order"] = json!([entries[1].0, entries[0].0]);
    data(
        &router,
        "POST",
        &save_path,
        json!({"expected_current_revision":draft["current_revision"], "draft":draft}),
    )
    .await;
    let full_session = data(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"title":"Full order"}),
    )
    .await;
    finish_turn(
        &router,
        full_session["agent_session_id"].as_str().unwrap(),
        "full-order",
        true,
    )
    .await;
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    for (index, request) in requests.iter().enumerate() {
        let body = request.body_json::<Value>().unwrap();
        let system = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "system")
            .map(|m| m["content"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let (inner, outer) = if index == 0 || index == 2 {
            (entries[0].1, entries[1].1)
        } else {
            (entries[1].1, entries[0].1)
        };
        assert!(
            system.starts_with(&format!("{outer}({inner}(")),
            "request {index}: {body}"
        );
    }
    services
        .plugin_runtime
        .shutdown_service_runtime(services.authoritative_user_id.as_ref())
        .await
        .unwrap();
}

async fn finish_turn(router: &axum::Router, session: &str, text: &str, success: bool) {
    let (status, response) = request(
        router,
        "POST",
        &format!("/api/agent-sessions/{session}/turns"),
        json!({
            "input":{"content":text}, "idempotency_key":format!("before-model-{text}")
        }),
    )
    .await;
    if !status.is_success() {
        assert!(
            !success && response.to_string().contains("before_model"),
            "{status} {response}"
        );
        return;
    }
    let mut latest = Value::Null;
    // An intentionally hung Service keeps its owned task until the production
    // 30s watchdog; the caller's middleware deadline is not a cleanup proof.
    let wait_seconds = if text == "wait" { 45 } else { 15 };
    let finished = tokio::time::timeout(std::time::Duration::from_secs(wait_seconds), async {
        loop {
            latest = data(
                router,
                "GET",
                &format!("/api/agent-sessions/{session}"),
                Value::Null,
            )
            .await;
            let active = matches!(
                latest["head"]["status"].as_str(),
                Some("running" | "starting")
            );
            let expected = if success {
                latest.to_string().contains(&format!("MIDDLEWARE_DONE_{text}"))
            } else {
                true
            };
            if !active && expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await;
    assert!(finished.is_ok(), "turn {text} did not settle: {latest}");
}

#[tokio::test]
async fn product_before_model_publish_select_and_real_node_transform() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (id, base) = publish_source(&router, SOURCE, capability, middleware::schemas()).await;
    let capability_id = capability(&id).id;
    let catalog = data(&router, "GET", "/api/agent-catalog", Value::Null).await;
    assert!(
        catalog["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["capability"]["id"] == capability_id.as_ref())
    );

    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(|request: &wiremock::Request| {
        let body = request.body_json::<Value>().unwrap();
        let messages = body["messages"].as_array().unwrap();
        let text = messages.iter().rev().find(|m| m["role"] == "user").unwrap()["content"].as_str().unwrap();
        let system = messages.iter().filter(|m| m["role"] == "system").map(|m| m["content"].as_str().unwrap()).collect::<Vec<_>>().join("\n");
        assert!(system.contains(&format!("PRODUCT_MIDDLEWARE:{text}")), "{body}");
        assert!(body.get("tools").is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)), "{body}");
        let frame = json!({"choices":[{"index":0,"delta":{"content":format!("MIDDLEWARE_DONE_{text}")},"finish_reason":"stop"}]});
        wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {frame}\n\ndata: [DONE]\n\n"))
    }).expect(3).mount(&upstream).await;
    let provider = data(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Middleware model", "base_url":format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash", "enabled":true, "capabilities":[{
            "task":"chat", "traits":["function_calling","streaming"], "protocol":"openai.chat_text", "connection_role":"default", "provider_params":{}
        }]}
    })).await;
    let preset = data(
        &router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name":"Middleware session", "reuse_existing":false,
            "model":{"provider_id":provider["provider_id"],"model":"step-3.7-flash"}
        }),
    )
    .await;
    let preset_id = preset["preset"]["preset_id"].as_str().unwrap();
    let editor = data(
        &router,
        "GET",
        &format!("/api/agent-presets/{preset_id}/editor"),
        Value::Null,
    )
    .await;
    let mut draft = editor["draft"].clone();
    // Offer one real tool before filtering: the minimal template alone does
    // not enable ToolSearch. This also exercises middleware + discovery in
    // the same Session without conflating their hidden consumers.
    let builtin = catalog["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["role"]["key"]["role_id"] == discovery::ROLE_ID)
        .unwrap()["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["selection"]["provider_mount_id"] == "nomifun-tool-discovery")
        .unwrap()["selection"]
        .clone();
    draft["document"]["system_role_provider_overrides"][discovery::ROLE_ID] = builtin;
    draft["document"]["enabled_capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "capability":{"id":discovery::CAPABILITY_ID,"version":"1.0.0"},
            "action_allowlist":[discovery::ACTION_ID]
        }));
    draft["document"]["enabled_capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "capability":{"id":capability_id,"version":"1.0.0"},
            "action_allowlist":[middleware::ACTION_ID]
        }));
    data(
        &router,
        "POST",
        &format!("/api/agent-presets/{preset_id}/revisions"),
        json!({
            "expected_current_revision":draft["current_revision"], "draft":draft
        }),
    )
    .await;
    let session = data(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id, "title":"Middleware"}),
    )
    .await;
    let session_id = session["agent_session_id"].as_str().unwrap();
    for text in ["first", "second"] {
        finish_turn(&router, session_id, text, true).await;
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
    let observations: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT bounded_observation_json FROM agent_effects WHERE session_id = ? AND action_id = ? AND state = 'returned'"
    ).bind(session_id).bind(middleware::ACTION_ID).fetch_all(services.database.pool()).await.unwrap();
    assert_eq!(observations.len(), 2, "middleware still needs durable outcome receipts");
    for observation in observations {
        let value: Value = serde_json::from_str(&observation).unwrap();
        assert_eq!(value["content_omitted"], true);
        assert_eq!(value["sha256"].as_str().unwrap().len(), 64);
        assert!(value["serialized_bytes"].as_u64().unwrap() > 0);
        assert!(!observation.contains("PRODUCT_MIDDLEWARE:"), "temporary patch leaked into its receipt");
        assert!(value.get("preview").is_none());
    }
    // Unknown timeout effects quarantine only their original Session. Freeze
    // another live Session now so withdrawal still exercises the old binding.
    let disabled_session = data(&router, "POST", "/api/agent-sessions",
        json!({"preset_id":preset_id,"title":"Middleware withdrawal"})).await;
    let disabled_id = disabled_session["agent_session_id"].as_str().unwrap();
    finish_turn(&router, disabled_id, "warm", true).await;
    for text in ["outside", "wait"] {
        finish_turn(&router, session_id, text, false).await;
        assert_eq!(
            upstream.received_requests().await.unwrap().len(),
            3,
            "failed middleware must not call provider"
        );
    }
    let w = data(&router, "GET", &format!("{base}/workshop"), Value::Null).await;
    let p = &w["plugin"];
    data(&router, "POST", &format!("{base}/enabled"), json!({
        "plugin_id":id, "expected_product_revision":p["product_revision"],
        "expected_pointer_revision":p["releases"]["pointer_revision"],
        "expected_active_release_digest":p["releases"]["active"]["release_digest"], "enabled":false
    })).await;
    finish_turn(&router, disabled_id, "disabled", false).await;
    assert_eq!(upstream.received_requests().await.unwrap().len(), 3);
    services
        .plugin_runtime
        .shutdown_service_runtime(services.authoritative_user_id.as_ref())
        .await
        .unwrap();
}

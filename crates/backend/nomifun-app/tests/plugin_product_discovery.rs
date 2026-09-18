//! Source build/publish -> saved choice -> real Nomi ToolSearch -> Node Service.
//! Only the model upstream is simulated; no replacement compiler or invoker.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use nomifun_agent_contracts::*;
use nomifun_ai_agent::tool_discovery as discovery;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tower::ServiceExt;

#[path = "common/mod.rs"]
mod common;

const TRUST: &str = "product-discovery-test";
const SERVICE: &str = r#"
export async function start() {
  return {
    async invoke({method, payload, signal}) {
      if (method !== 'tool.discovery.rank') throw new Error('unexpected invocation');
      for (const c of payload.candidates) {
        if (Object.keys(c).sort().join(',') !== 'aliases,description,name') throw new Error('not metadata only');
      }
      if (payload.query === 'outside') return {names: ['not-a-candidate']};
      if (payload.query === 'wait') return new Promise((resolve, reject) => {
        signal.addEventListener('abort', () => reject(new Error('cancelled')), {once: true});
      });
      return {names: payload.candidates.map(c => c.name).reverse().slice(0, payload.limit)};
    },
    async dispose() {},
  };
}
"#;

async fn request(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

async fn data(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let (status, value) = request(router, method, path, body).await;
    assert!(status.is_success(), "{path}: {status} {value}");
    value["data"].clone()
}

async fn turn(router: &axum::Router, session_id: &str, query: &str) {
    turn_outcome(router, session_id, query, true).await;
}

async fn turn_outcome(router: &axum::Router, session_id: &str, query: &str, success: bool) {
    data(
        router,
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({
            "input":{"content":query}, "idempotency_key":format!("discovery-{query}")
        }),
    )
    .await;
    let marker = format!("PRODUCT_DISCOVERY_DONE_{query}");
    let mut last_page = Value::Null;
    // Failure publication includes retained-task cleanup (30s Service
    // watchdog), not only the much shorter ToolSearch policy deadline.
    let wait_seconds = if success { 20 } else { 45 };
    let completed = tokio::time::timeout(std::time::Duration::from_secs(wait_seconds), async {
        loop {
            let page = data(
                router,
                "GET",
                &format!("/api/agent-sessions/{session_id}"),
                Value::Null,
            )
            .await;
            if success && page.to_string().contains(&marker) {
                break;
            }
            let active = matches!(
                page["head"]["status"].as_str(),
                Some("running" | "starting")
            );
            if !success && !active {
                assert!(!page.to_string().contains(&marker), "timeout must not complete a model turn");
                break;
            }
            last_page = page;
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await;
    assert!(completed.is_ok(), "turn {query} did not finish: {last_page}");
}

async fn publish_source(
    router: &axum::Router, source: &str,
    make_capability: impl FnOnce(&str) -> CapabilityManifest,
    schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
) -> (String, String) {
    let library = data(&router, "GET", "/api/plugins/runtimes", Value::Null).await;
    let w = data(
        &router,
        "POST",
        "/api/plugins/runtimes/projects",
        json!({
            "expected_library_revision": library["library_revision"],
            "display_name": "Product discovery", "service_source": source
        }),
    )
    .await;
    let id = w["plugin"]["plugin_id"].as_str().unwrap();
    let capability = make_capability(id);
    let base = format!("/api/plugins/runtimes/{id}");
    let w = data(&router, "POST", &format!("{base}/source/edit"), json!({
        "plugin_id": id, "expected_product_revision": w["plugin"]["product_revision"],
        "project_id": w["project_id"], "expected_project_revision": w["project_revision"],
        "expected_build_generation": w["build_generation"], "expected_source_snapshot_digest": w["source_snapshot_digest"],
        "path": "nomifun.plugin.json", "content": json!({
            "contributions": PackageContributions { capabilities: vec![capability], ..Default::default() },
            "schemas": schemas
        }).to_string()
    })).await;
    let w = data(&router, "POST", &format!("{base}/build"), json!({
        "plugin_id": id, "expected_product_revision": w["plugin"]["product_revision"],
        "project_id": w["project_id"], "expected_project_revision": w["project_revision"],
        "expected_build_generation": w["build_generation"], "expected_source_snapshot_digest": w["source_snapshot_digest"],
        "expected_dependency_lock_digest": w["dependency_lock_digest"]
    })).await;
    let p = data(
        &router,
        "POST",
        &format!("{base}/publish"),
        json!({
            "plugin_id": id, "expected_product_revision": w["plugin"]["product_revision"],
            "expected_pointer_revision": w["plugin"]["releases"]["pointer_revision"],
            "expected_active_release_epoch": w["plugin"]["releases"]["active_release_epoch"],
            "ready_release_id": w["plugin"]["releases"]["ready"]["release_id"],
            "expected_ready_release_digest": w["plugin"]["releases"]["ready"]["release_digest"],
            "expected_active_release_digest": w["plugin"]["releases"]["active"]["release_digest"],
            "acknowledge_test_warning": true
        }),
    )
    .await["plugin"]
        .clone();
    data(&router, "POST", &format!("{base}/enabled"), json!({
        "plugin_id": id, "expected_product_revision": p["product_revision"],
        "expected_pointer_revision": p["releases"]["pointer_revision"],
        "expected_active_release_digest": p["releases"]["active"]["release_digest"], "enabled": true
    })).await;
    (id.to_owned(), base)
}

fn discovery_capability(id: &str) -> CapabilityManifest {
    let capability_id = format!("plugin.{id}.discovery");
    CapabilityManifest {
        id: capability_id.clone().into(),
        contribution_id: format!("capability:{capability_id}").into(),
        version: "1.0.0".into(),
        kind: CapabilityKind::Tool,
        package: PackageRef {
            id: format!("plugin.{id}").into(),
            version: "1.0.0".into(),
        },
        display: LocalizedMetadata {
            name: "Product discovery".into(),
            description: "Reverse authorized candidates".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        requires: vec![],
        conflicts: vec![],
        requires_runtime_features: vec![],
        supported_surfaces: capability_surface_declarations(
            ["desktop", "headless"],
            [CapabilityConsumer::Agent, CapabilityConsumer::PluginService],
        ),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type":"object"})),
        contributions: CapabilityContributions {
            actions: vec![discovery::action()],
            ..Default::default()
        },
    }
}

#[path = "plugin_product_before_tool.rs"]
mod before_tool;

#[path = "plugin_product_before_model.rs"]
mod before_model;

#[path = "plugin_product_service.rs"]
mod service_invocation;

#[tokio::test]
async fn published_discovery_is_consumed_by_nomi_and_conflicts_are_rejected_before_save() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let (id, base) = publish_source(&router, SERVICE, discovery_capability, discovery::schemas()).await;
    let capability_id = format!("plugin.{id}.discovery");
    let catalog = data(&router, "GET", "/api/agent-catalog", Value::Null).await;
    assert!(
        catalog["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["capability"]["id"] == capability_id)
    );

    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            let messages = body["messages"].as_array().unwrap();
            let done = messages.last().is_some_and(|m| m["role"] == "tool");
            let query = messages.iter().rev().find(|m| m["role"] == "user").unwrap()["content"].as_str().unwrap();
            let frame = if done {
                json!({"choices":[{"index":0,"delta":{"content":format!("PRODUCT_DISCOVERY_DONE_{query}")},"finish_reason":"stop"}]})
            } else {
                json!({"choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":format!("search-product-{query}"),"type":"function","function":{"name":"ToolSearch","arguments":json!({"query":query}).to_string()}}]},"finish_reason":"tool_calls"}]})
            };
            wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {frame}\n\ndata: [DONE]\n\n"))
        }).expect(9).mount(&upstream).await;
    let provider = data(&router, "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Discovery model", "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": {"api_keys":["test-only"]}, "enabled": true,
        "initial_model": {"model":"step-3.7-flash", "enabled":true, "capabilities":[{
            "task":"chat", "traits":["function_calling","streaming"], "protocol":"openai.chat_text", "connection_role":"default", "provider_params":{}
        }]}
    })).await;
    let preset = data(
        &router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name":"Discovery session", "reuse_existing":false,
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
    draft["document"]["enabled_capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "capability":{"id":capability_id,"version":"1.0.0"},
            "action_allowlist":[discovery::ACTION_ID]
        }));
    let saved = data(
        &router,
        "POST",
        &format!("/api/agent-presets/{preset_id}/revisions"),
        json!({
            "expected_current_revision": draft["current_revision"], "draft":draft
        }),
    )
    .await;
    draft["current_revision"] = saved["revision"]["reference"].clone();
    let mut conflicting = draft.clone();
    let builtin_choice = catalog["roles"]
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
    conflicting["document"]["system_role_provider_overrides"][discovery::ROLE_ID] = builtin_choice;
    conflicting["document"]["enabled_capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "capability":{"id":discovery::CAPABILITY_ID,"version":"1.0.0"},
            "action_allowlist":[discovery::ACTION_ID]
        }));
    let (status, error) = request(
        &router,
        "POST",
        &format!("/api/agent-presets/{preset_id}/revisions"),
        json!({
            "expected_current_revision": draft["current_revision"], "draft":conflicting
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert!(
        error.to_string().contains("multiple discovery policies"),
        "{error}"
    );
    let unchanged = data(
        &router,
        "POST",
        &format!("/api/agent-presets/{preset_id}/revisions"),
        json!({
            "expected_current_revision": draft["current_revision"], "draft":draft
        }),
    )
    .await;
    assert_eq!(
        unchanged["resolved_snapshot_ref"],
        saved["resolved_snapshot_ref"]
    );
    let session = data(
        &router,
        "POST",
        "/api/agent-sessions",
        json!({"preset_id":preset_id,"title":"Discovery"}),
    )
    .await;
    let session_id = session["agent_session_id"].as_str().unwrap();
    turn(&router, session_id, "q").await;
    let requests = upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let first = requests[0].body_json::<Value>().unwrap();
    let names = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names.iter().filter(|name| **name == "ToolSearch").count(),
        1
    );
    assert!(
        !names
            .iter()
            .any(|name| name.contains("plugin_product") || name.contains("discovery"))
    );
    let second = requests[1].body_json::<Value>().unwrap();
    let result = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    let content = result["content"].as_str().unwrap();
    // Native discovery rejects inexact one-letter queries. This success must
    // come from the selected hidden JS policy, including for an empty catalog.
    assert!(
        content.starts_with('[') || content.starts_with("No deferred tools matching"),
        "{content}"
    );
    assert!(!content.starts_with("[tool error]"), "{content}");
    assert!(!content.contains("parameters"));
    // Keep withdrawal coverage independent of the timeout's uncertain effect.
    // A timed-out dispatch must not be made reusable just to finish this test.
    let withdrawal_session = data(&router, "POST", "/api/agent-sessions",
        json!({"preset_id":preset_id,"title":"Withdrawal"})).await;
    let withdrawal_id = withdrawal_session["agent_session_id"].as_str().unwrap();
    turn(&router, withdrawal_id, "q").await;
    for (query, expected) in [
        ("outside", "outside the captured catalog"),
        ("wait", "timed out"),
        ("withdrawn", "Selected discovery policy failed"),
    ] {
        if query == "withdrawn" {
            let w = data(&router, "GET", &format!("{base}/workshop"), Value::Null).await;
            let p = &w["plugin"];
            data(&router, "POST", &format!("{base}/enabled"), json!({
                "plugin_id":id, "expected_product_revision":p["product_revision"],
                "expected_pointer_revision":p["releases"]["pointer_revision"],
                "expected_active_release_digest":p["releases"]["active"]["release_digest"], "enabled":false
            })).await;
        }
        if query == "wait" {
            let before = upstream.received_requests().await.unwrap().len();
            turn_outcome(&router, session_id, query, false).await;
            assert_eq!(upstream.received_requests().await.unwrap().len(), before + 1,
                "timeout must fence the next provider round");
            continue;
        }
        turn(&router, if query == "withdrawn" { withdrawal_id } else { session_id }, query).await;
        let requests = upstream.received_requests().await.unwrap();
        let before = requests[requests.len() - 2].body_json::<Value>().unwrap();
        let after = requests.last().unwrap().body_json::<Value>().unwrap();
        let result = after["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["role"] == "tool")
            .unwrap();
        assert!(
            result["content"].as_str().unwrap().contains(expected),
            "{result}"
        );
        assert_eq!(
            before["tools"], after["tools"],
            "failed discovery must not expose more schemas"
        );
    }
    services
        .plugin_runtime
        .shutdown_service_runtime(services.authoritative_user_id.as_ref())
        .await
        .unwrap();
}

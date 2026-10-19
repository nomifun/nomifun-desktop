//! Product resources must make the full official Companion usable immediately.
//! Model transport is local; tool execution, Effects and artifact publication use
//! the production owners in an isolated fixture dataset.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use nomifun_agent_contracts::{AgentSessionId, OperationId};
use nomifun_agent_session::{AgentEffectState, AgentSessionStore, TurnReceiptStatus};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::{Arc, Mutex}};
use tower::ServiceExt;

const TRUST: &str = "official-companion-resource-supply";
const CONTENT: &str = "OFFICIAL_COMPANION_FILE_AND_ARTIFACT\n";
const PLAIN_INPUT: &str = "你好，再聊一下。";

async fn call(router: &axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn wait_completed(store: &AgentSessionStore, session: &AgentSessionId, turn: &Value) {
    let operation = OperationId::from(turn["data"]["operation_id"].as_str().unwrap().to_owned());
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let receipt = store.read_turn_receipt(session, &operation).await.unwrap();
            if receipt.status == TurnReceiptStatus::Completed { break; }
            if receipt.status != TurnReceiptStatus::Running {
                let facts = store.turn_output_facts(session, &operation).await.unwrap();
                let terminal = receipt.terminal_event.as_ref().expect("a failed Turn has its canonical terminal");
                let payload = facts.event_payloads.get(terminal.event_id.as_ref())
                    .expect("the terminal Payload is resolved inside this exact Turn");
                panic!("Companion Turn ended as {:?}: {}", receipt.status, payload);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("the Companion must finish its native Turn");
}

fn tool_output(body: &Value, call_id: &str) -> Value {
    let message = body["messages"].as_array().unwrap().iter().find(|message|
        message["role"] == "tool" && message["tool_call_id"] == call_id)
        .unwrap_or_else(|| panic!("the real tool receipt {call_id} must reach the next model step: {body}"));
    let output: Value = serde_json::from_str(message["content"].as_str().unwrap())
        .unwrap_or_else(|error| panic!("the tool receipt {call_id} must be canonical JSON, not an error: {error}: {message}"));
    assert!(output.get("error").is_none(), "{call_id} returned an error: {output}");
    output
}

fn assert_discovered(body: &Value, call_id: &str, name: &str) {
    let matches = tool_output(body, call_id);
    assert_eq!(matches.as_array().unwrap().len(), 1, "an exact Action query must not widen the result: {matches}");
    assert_eq!(matches[0]["name"], name);
    assert_eq!(matches[0]["activated"], true);
    assert_eq!(matches[0]["already_available"], false);
    assert_eq!(matches[0]["schema_visibility"], "visible_on_next_model_step");
    assert!(body["tools"].as_array().unwrap().iter().any(|tool|
        tool["function"]["name"] == name),
        "canonical discovery must make the frozen {name} schema visible: {body}");
}

fn has_tool_output(body: &Value, call_id: &str) -> bool {
    body["messages"].as_array().unwrap().iter().any(|message|
        message["role"] == "tool" && message["tool_call_id"] == call_id)
}

fn tool_delta(call_id: &str, name: &str, arguments: Value) -> (Value, &'static str) {
    (json!({"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function",
        "function":{"name":name,"arguments":arguments.to_string()}}]}), "tool_calls")
}

fn completion_arguments(body: &Value, summary: &str, evidence: &[(&str, &str)]) -> Option<Value> {
    let tool = body["tools"].as_array().unwrap().iter().find(|tool|
        tool["function"]["name"] == "report_completion")?;
    let properties = &tool["function"]["parameters"]["properties"];
    let eligible = properties["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"]
        .as_array().cloned().unwrap_or_default();
    let criteria = if evidence.is_empty() {
        vec![json!({"disposition":"unverified","rationale":"本回合只回应问候，没有执行外部操作或检查外部状态。"})]
    } else {
        evidence.iter().map(|(call_id, rationale)| {
            assert!(eligible.iter().any(|value| value == *call_id),
                "the real Runtime must retain this actual tool receipt as current evidence: {call_id}; available={eligible:?}");
            json!({"disposition":"supported","evidence_call_ids":[call_id],"rationale":rationale})
        }).collect()
    };
    let errors = properties["observed_tool_error_count"]["const"].as_u64().expect("Runtime supplies its exact error count");
    let commands = properties["observed_command_failure_count"]["const"].as_u64().expect("Runtime supplies its exact command-failure count");
    assert_eq!((errors, commands), (0, 0), "the fixture must not conceal a failed tool or invalid report");
    Some(json!({"summary":summary,"criteria":criteria,
        "observed_tool_error_count":errors,"observed_command_failure_count":commands}))
}

#[tokio::test]
async fn companion_first_entry_executes_and_publishes_without_optional_connections() {
    let (router, services) = common::build_local_trust_app(TRUST).await;
    common::materialize_builtin_skills_for_fixture(&services).await;
    let upstream = wiremock::MockServer::start().await;
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    let emitted_calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let emitted = emitted_calls.clone();
    let report_counts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let reports = report_counts.clone();
    let plain_answers = Arc::new(Mutex::new(0_usize));
    let answers = plain_answers.clone();
    let expected_session = Arc::new(Mutex::new(None::<String>));
    let expected = expected_session.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            for (enabled, capability_id) in [(cfg!(feature = "browser-use"), "browser"),
                (cfg!(feature = "computer-use"), "computer")] {
                if !enabled {
                    let names = nomifun_agent_contracts::tool_presentation::tool_presentations().iter()
                        .filter(|presentation| presentation.capability_id.as_deref() == Some(capability_id))
                        .map(|presentation| presentation.name.as_str()).collect::<std::collections::BTreeSet<_>>();
                    assert!(!names.is_empty(), "the published native tool aliases must exist");
                    assert!(body["tools"].as_array().unwrap().iter().all(|tool|
                        !names.contains(tool["function"]["name"].as_str().unwrap_or_default())),
                        "a missing physical adapter must not expose any exact {capability_id} tool alias");
                }
            }
            let mut requests = captured.lock().unwrap();
            requests.push(body.clone());
            assert!(requests.len() <= 16, "the fixture cannot loop after settled work");
            let latest_input = body["messages"].as_array().unwrap().iter().rev()
                .find(|message| message["role"] == "user").expect("every model step has the accepted input")["content"].to_string();
            let (delta, finish) = if latest_input.contains("companion-proof.txt") {
                let (delta, finish) = if !has_tool_output(&body, "companion-write") {
                    assert!(body["tools"].as_array().unwrap().iter().any(|tool|
                        tool["function"]["name"] == "write_file"), "{body}");
                    assert!(body["tools"].as_array().unwrap().iter().all(|tool|
                        tool["function"]["name"] != "publish_artifact"),
                        "the complete Companion must defer the artifact schema until discovery: {body}");
                    tool_delta("companion-write", "write_file", json!({"path":"companion-proof.txt","content":CONTENT}))
                } else if !has_tool_output(&body, "companion-find-publish") {
                    let written = tool_output(&body, "companion-write");
                    assert!(written.get("sha256").is_some(), "the native write must return its owner proof: {written}");
                    assert!(body["tools"].as_array().unwrap().iter().any(|tool|
                        tool["function"]["name"] == "ToolSearch"), "{body}");
                    tool_delta("companion-find-publish", "ToolSearch", json!({"query":"workspace.artifacts/publish"}))
                } else if !has_tool_output(&body, "companion-publish") {
                    assert_discovered(&body, "companion-find-publish", "publish_artifact");
                    tool_delta("companion-publish", "publish_artifact", json!({"path":"companion-proof.txt"}))
                } else {
                    let artifact = tool_output(&body, "companion-publish");
                    assert_eq!(artifact["artifact_id"], nomifun_agent_contracts::digest_bytes(CONTENT.as_bytes()).as_ref());
                    let arguments = completion_arguments(&body,
                        "companion-proof.txt 已写入，并已发布内容一致的产物。",
                        &[("companion-write", "工作区 owner 回执证明文件已写入指定内容。"),
                          ("companion-publish", "产物 owner 回执证明已发布该文件的内容寻址产物。")])
                        .expect("the actual multi-operation task requires Runtime completion accounting");
                    let mut counts = reports.lock().unwrap();
                    let count = counts.entry("file".into()).or_default();
                    *count += 1;
                    assert!(*count <= 2, "only the native bounded report review may ask for another report");
                    tool_delta(&format!("companion-report-file-{count}"), "report_completion", arguments)
                };
                (delta, finish)
            } else if latest_input.contains("knowledge.management/list") {
                let (delta, finish) = if !has_tool_output(&body, "companion-find-knowledge-list") {
                    assert!(body["tools"].as_array().unwrap().iter().all(|tool|
                        tool["function"]["name"] != "list_knowledge_bases" && tool["function"]["name"] != "list_agent_sessions"),
                        "management tools must begin deferred on the full Companion surface: {body}");
                    tool_delta("companion-find-knowledge-list", "ToolSearch", json!({"query":"knowledge.management/list"}))
                } else if !has_tool_output(&body, "companion-list-knowledge") {
                    assert_discovered(&body, "companion-find-knowledge-list", "list_knowledge_bases");
                    tool_delta("companion-list-knowledge", "list_knowledge_bases", json!({"offset":0,"limit":25}))
                } else if !has_tool_output(&body, "companion-find-session-list") {
                    let listed = tool_output(&body, "companion-list-knowledge");
                    assert_eq!(listed["bases"], json!([]), "the isolated installation has no Knowledge registrations: {listed}");
                    assert_eq!(listed["total"], 0);
                    assert_eq!(listed["next_offset"], Value::Null);
                    tool_delta("companion-find-session-list", "ToolSearch", json!({"query":"session.management/list"}))
                } else if !has_tool_output(&body, "companion-list-sessions") {
                    assert_discovered(&body, "companion-find-session-list", "list_agent_sessions");
                    tool_delta("companion-list-sessions", "list_agent_sessions", json!({"limit":25,"include_presets":false}))
                } else {
                    let listed = tool_output(&body, "companion-list-sessions");
                    let sid = expected.lock().unwrap().clone().expect("the real Companion Session is already open");
                    assert_eq!(listed["total"], 1, "the owned isolated installation contains exactly this Companion: {listed}");
                    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
                    assert_eq!(listed["items"][0]["session_id"], sid);
                    assert_eq!(listed["items"][0]["remote"], false);
                    assert_eq!(listed["has_more"], false);
                    assert!(listed.get("presets").is_none(), "unrequested preset details must not be supplied");
                    let summary = "知识库列表为空；当前会话列表包含这一个伙伴会话。";
                    if let Some(arguments) = completion_arguments(&body, summary,
                        &[("companion-list-knowledge", "知识库管理 owner 返回了当前安装的空列表。"),
                          ("companion-list-sessions", "会话管理 owner 返回了当前安装的伙伴会话。")]) {
                        let mut counts = reports.lock().unwrap();
                        let count = counts.entry("lists".into()).or_default();
                        *count += 1;
                        assert!(*count <= 2, "only the native bounded report review may ask for another report");
                        tool_delta(&format!("companion-report-lists-{count}"), "report_completion", arguments)
                    } else { (json!({"role":"assistant","content":summary}), "stop") }
                };
                (delta, finish)
            } else {
                assert!(latest_input.contains(PLAIN_INPUT), "unrecognized fixture input: {latest_input}");
                let summary = "你好，普通对话仍然可以继续。";
                let (delta, finish) = if let Some(arguments) = completion_arguments(&body, summary, &[]) {
                    let mut counts = reports.lock().unwrap();
                    let count = counts.entry("plain".into()).or_default();
                    *count += 1;
                    assert!(*count <= 2, "only the native bounded report review may ask for another report");
                    tool_delta(&format!("companion-report-plain-{count}"), "report_completion", arguments)
                } else { (json!({"role":"assistant","content":summary}), "stop") };
                (delta, finish)
            };
            if let Some(call) = delta["tool_calls"].as_array().and_then(|calls| calls.first()) {
                let name = call["function"]["name"].as_str().unwrap().to_owned();
                let id = call["id"].as_str().unwrap().to_owned();
                let mut calls = emitted.lock().unwrap();
                assert!(!calls.iter().any(|(_, old)| old == &id), "a settled business/control call cannot be emitted again: {id}");
                calls.push((name, id));
            } else {
                *answers.lock().unwrap() += 1;
            }
            let first = json!({"id":"full-companion","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let last = json!({"id":"full-companion","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
            wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"))
        }).mount(&upstream).await;
    let (status, provider) = call(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"Full Companion fixture", "base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["isolated-fixture"]}, "enabled":true,
        "initial_model":{"model":"full-companion-model","enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default",
            "context_limit":1000000,"output_limit":4096
        }]}
    })).await;
    assert_eq!(status, StatusCode::CREATED, "{provider}");
    let model = json!({"provider_id":provider["data"]["provider_id"],"model":"full-companion-model"});
    let (status, companion) = call(&router, "POST", "/api/companion/companions",
        json!({"name":"Ready Companion", "character":"mochi"})).await;
    assert_eq!(status, StatusCode::CREATED, "{companion}");
    let companion_id = companion["data"]["companion_id"].as_str().unwrap();
    let (status, patched) = call(&router, "PATCH", &format!("/api/companion/companions/{companion_id}"),
        json!({"model":model})).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    let (status, created) = call(&router, "POST", &format!("/api/companion/companions/{companion_id}/companion/threads"), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let sid = created["data"]["conversation_id"].as_str().unwrap();
    *expected_session.lock().unwrap() = Some(sid.to_owned());
    let session = AgentSessionId::from(sid.to_owned());
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let binding = store.get_live_session(&session).await.unwrap().agent_binding;
    let resource = |kind: &str| binding.typed_resource_bindings.iter().find(|resource| resource.resource_kind.as_ref() == kind);
    for kind in ["companion", "companion_memory", "workspace", "process_session", "project_memory", "asset_library", "scheduler"] {
        assert!(resource(kind).is_some(), "the host must automatically bind {kind}");
    }
    for kind in ["knowledge_base", "channel", "robot", "ssh_host", "canvas"] {
        assert!(resource(kind).is_none(), "{kind} cannot be invented to open a Companion");
    }
    if !cfg!(feature = "browser-use") { assert!(resource("browser").is_none()); }
    if !cfg!(feature = "computer-use") { assert!(resource("computer").is_none()); }
    let workspace = resource("workspace").unwrap();
    assert!(workspace.operations.contains("write"));
    let cwd = workspace.typed_parameters.get("workspace_root").expect("Companion has its fixed private workspace");
    assert_eq!(resource("process_session").unwrap().typed_parameters.get("workspace_root"), Some(cwd));
    let actual_workspace = std::fs::canonicalize(cwd).unwrap();
    let companion_workspaces = std::fs::canonicalize(services.data_dir.join("companion").join("workspaces")).unwrap();
    assert!(actual_workspace.starts_with(&companion_workspaces), "{actual_workspace:?} must remain within {companion_workspaces:?}");
    let (status, started) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),
        "input":{"content":format!("请调用 write_file 一次写入 companion-proof.txt，内容准确为 {CONTENT}，随后将这个文件发布为产物并回复产物已经完成。如果发布工具的schema尚未显示，请先用 ToolSearch 精确查询 workspace.artifacts/publish 再调用它。实际写入和发布各执行一次，不要重复执行或调用其它执行工具。")}
    })).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    wait_completed(&store, &session, &started).await;
    assert_eq!(std::fs::read(std::path::Path::new(cwd).join("companion-proof.txt")).unwrap(), CONTENT.as_bytes());
    let artifact_id = nomifun_agent_contracts::digest_bytes(CONTENT.as_bytes());
    assert_eq!(std::fs::read(std::path::Path::new(cwd).join(".nomifun").join("artifacts")
        .join(artifact_id.as_ref())).unwrap(), CONTENT.as_bytes(), "publication must persist the content-addressed artifact");
    let effects = store.list_effects(&session).await.unwrap();
    assert_eq!(effects.len(), 2, "one native write and one native publication");
    assert!(effects.iter().all(|effect| effect.state == AgentEffectState::Returned));
    assert!(effects.iter().all(|effect| effect.resource_binding_id.as_ref() == Some(&workspace.binding_id)));
    assert!((1..=2).contains(report_counts.lock().unwrap().get("file").unwrap()),
        "the first Turn must finish through its real current completion account");
    let (status, started) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(), "input":{"content":PLAIN_INPUT}
    })).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    wait_completed(&store, &session, &started).await;
    for name in ["write_file", "publish_artifact"] {
        assert_eq!(emitted_calls.lock().unwrap().iter().filter(|(tool, _)| tool == name).count(), 1,
            "the ordinary next Turn must not repeat {name}");
    }
    assert_eq!(store.list_effects(&session).await.unwrap().len(), 2, "ordinary next Turn must not replay earlier Effects");
    assert_eq!(store.get_live_session(&session).await.unwrap().agent_binding, binding);
    let (status, started) = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),
        "input":{"content":"请查询我的知识库列表和会话列表。分别用 ToolSearch 精确查询 knowledge.management/list 与 session.management/list，再调用匹配的真实工具。只查询列表，不创建、修改、删除、发送消息或重放之前的文件操作。"}
    })).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    wait_completed(&store, &session, &started).await;
    {
    let calls = emitted_calls.lock().unwrap();
    for (name, expected) in [("write_file", 1), ("publish_artifact", 1), ("ToolSearch", 3),
        ("list_knowledge_bases", 1), ("list_agent_sessions", 1)] {
        assert_eq!(calls.iter().filter(|(tool, _)| tool == name).count(), expected,
            "each business/discovery call must execute exactly its requested count: {name}");
    }
    let reports = report_counts.lock().unwrap();
    assert!(reports.values().all(|count| (1..=2).contains(count)), "only bounded native report review may add completion calls: {reports:?}");
    assert_eq!(calls.iter().filter(|(tool, _)| tool == "report_completion").count(), reports.values().sum::<usize>());
    assert_eq!(requests.lock().unwrap().len(), calls.len() + *plain_answers.lock().unwrap(),
        "each model request must produce only one accounted business/control call or plain answer");
    }
    assert_eq!(store.list_effects(&session).await.unwrap().len(), 2, "read-only management cannot introduce durable Effects or replay writes");
    assert_eq!(store.get_live_session(&session).await.unwrap().agent_binding, binding);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

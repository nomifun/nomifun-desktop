//! A selected directory is the Session default, not an execution prerequisite
//! for independent native targets. This uses the real router/runtime/owners
//! with an isolated local scripted provider, without native administrator auth.
use std::sync::{Arc, Mutex};
use std::time::Duration;
use axum::{Router, body::Body, http::{Request, StatusCode}};
use nomifun_agent_contracts::{AgentSessionId, OperationId};
use nomifun_agent_session::{AgentEffectState, AgentSessionStore, TurnReceiptStatus};
use serde_json::{Value, json};
use tower::ServiceExt;

const TRUST: &str = "missing-default-workspace-native-targets";
const FIRST: &str = "BEFORE_DEFAULT_REMOVAL";
const SECOND: &str = "ABSOLUTE_AFTER_DEFAULT_REMOVED";
const CONTENT: &str = "The frozen default may disappear; this absolute target remains usable.\n";

async fn response(router: &Router, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let response = router.clone().oneshot(Request::builder().method(method).uri(uri)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn call(router: &Router, method: &str, uri: &str, body: Value) -> Value {
    let (status, value) = response(router, method, uri, body).await;
    assert!(matches!(status, StatusCode::OK | StatusCode::CREATED), "{method} {uri}: {status} {value}");
    value["data"].clone()
}

fn stream(tool: Option<(&str, &str, Value)>, text: &str) -> wiremock::ResponseTemplate {
    let (delta, reason) = match tool {
        Some((id, name, args)) => (json!({"tool_calls":[{"index":0,"id":id,"type":"function",
            "function":{"name":name,"arguments":args.to_string()}}]}), "tool_calls"),
        None => (json!({"content":text}), "stop"),
    };
    let first = json!({"id":"missing-default-fixture","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let last = json!({"id":"missing-default-fixture","choices":[{"index":0,"delta":{},"finish_reason":reason}]});
    wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"))
}

async fn wait_completed(store: &AgentSessionStore, session: &AgentSessionId, started: &Value) {
    let operation = OperationId::from(started["operation_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let receipt = store.read_turn_receipt(session, &operation).await.unwrap();
            if receipt.status == TurnReceiptStatus::Completed { break; }
            assert_eq!(receipt.status, TurnReceiptStatus::Running, "{receipt:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the native file/process Turn must settle without recreating its default");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removed_selected_default_keeps_same_session_absolute_file_and_explicit_process_cwd() {
    let fixture = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(fixture.path()).unwrap();
    let default = base.join("selected-default");
    let outside = base.join("independent-target");
    std::fs::create_dir(&default).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let file = outside.join("proof.txt");
    let provider_file = file.clone();
    let provider_outside = outside.clone();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body = request.body_json::<Value>().unwrap();
            let mut requests = captured.lock().unwrap();
            requests.push(body.clone());
            assert!(requests.len() <= 8, "bounded current-generation owner sequence: {body}");
            let messages = body["messages"].as_array().unwrap();
            if !messages.iter().any(|message| message["role"] == "user" && message["content"].to_string().contains(SECOND)) {
                return stream(None, FIRST);
            }
            let steps = [
                ("missing-default-scope", "read_file", json!({"path":provider_outside,"format":"instruction_scope"})),
                ("missing-default-write", "write_file", json!({"path":provider_file,"content":CONTENT})),
                ("missing-default-process", "exec_command", json!({"command":"/bin/pwd","args":[],"cwd":provider_outside,"timeout_ms":10000})),
                ("missing-default-read", "read_file", json!({"path":provider_file})),
            ];
            for (id, name, args) in steps {
                let result = messages.iter().rev().find(|message| message["role"] == "tool" && message["tool_call_id"] == id);
                let Some(result) = result else {
                    assert!(body["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == name), "missing exact default tool {name}");
                    return stream(Some((id, name, args)), "");
                };
                let raw = result["content"].as_str().unwrap();
                let value: Value = serde_json::from_str(raw).unwrap_or_else(|error| panic!("{id}: {error}: {raw}"));
                match id {
                    "missing-default-scope" => {
                        assert_eq!(value["observation_kind"], "instruction_scope");
                        assert_eq!(value["canonical_path"], provider_outside.to_str().unwrap());
                    }
                    "missing-default-write" => {
                        assert_eq!(value["native_path"], provider_file.to_str().unwrap());
                        assert_eq!(value["workspace_path"], Value::Null);
                    }
                    "missing-default-process" => {
                        assert_eq!(value["state"], "exited");
                        assert_eq!(value["exit_code"], 0);
                        assert_eq!(value["cleanup"]["reaped"], true);
                        assert_eq!(value["output"]["text"], format!("{}\n", provider_outside.display()));
                    }
                    "missing-default-read" => {
                        assert_eq!(value["content"], CONTENT);
                        assert_eq!(value["workspace_path"], Value::Null);
                    }
                    _ => unreachable!(),
                }
            }
            let tool = body["tools"].as_array().unwrap().iter()
                .find(|tool| tool["function"]["name"] == "report_completion")
                .expect("effectful work must use the current formal completion account");
            let properties = &tool["function"]["parameters"]["properties"];
            let eligible = properties["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"]
                .as_array().expect("current owner observations must be advertised");
            let evidence = eligible.iter().filter(|id| id.as_str().is_some_and(|id| id.starts_with("missing-default-")))
                .cloned().collect::<Vec<_>>();
            assert!(!evidence.is_empty());
            let mut account = json!({"summary":"The absolute file and explicit-cwd process were verified while the frozen default remained absent.",
                "criteria":[{"step":"Native absolute file and explicit process cwd","disposition":"supported",
                    "rationale":"Exact owner file content, native cwd output and process reaped receipts were checked.","evidence_call_ids":evidence}]});
            for field in ["observed_tool_error_count", "observed_command_failure_count"] {
                if let Some(value) = properties[field].get("const") { account[field] = value.clone(); }
            }
            let accounts = messages.iter().filter(|message| message["role"] == "tool"
                && message["tool_call_id"].as_str().is_some_and(|id| id.starts_with("missing-default-completion-"))).count();
            assert!(accounts <= 1, "at most one normal bounded completion review");
            stream(Some((if accounts == 0 { "missing-default-completion-1" } else { "missing-default-completion-2" }, "report_completion", account)), "")
        }).mount(&upstream).await;
    let (router, services) = super::common::build_local_trust_app(TRUST).await;
    let provider = call(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan","name":"Missing default native targets","base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["fixture-only"]},"enabled":true,
        "initial_model":{"model":"missing-default","enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096
        }]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"missing-default"});
    let preset = call(&router, "POST", "/api/agent-presets/from-template/chat.minimal",
        json!({"display_name":"Missing default fixture","reuse_existing":false,"model":model})).await;
    let create = json!({"preset_id":preset["preset"]["preset_id"],"model":model,"workspace":default,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},
            {"resource_kind":"process_session","resource_id":"managed-process-session"}]});
    let created = call(&router, "POST", "/api/agent-sessions", create.clone()).await;
    let sid = created["agent_session_id"].as_str().unwrap();
    let session = AgentSessionId::from(sid);
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let binding = store.get_live_session(&session).await.unwrap().agent_binding;
    let first = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),"input":{"content":FIRST}
    })).await;
    wait_completed(&store, &session, &first).await;
    std::fs::remove_dir(&default).unwrap();
    let (status, refused_create) = response(&router, "POST", "/api/agent-sessions", create).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "a new directory selection remains strict: {refused_create}");
    assert_eq!(refused_create["code"], "WORKSPACE_DIRECTORY_UNAVAILABLE");
    let second = call(&router, "POST", &format!("/api/agent-sessions/{sid}/turns"), json!({
        "idempotency_key":uuid::Uuid::now_v7().to_string(),
        "input":{"content":format!("{SECOND}: read instruction_scope for {}, write {CONTENT:?} to the absolute file {}, run /bin/pwd with exact explicit cwd {}, then read that file. The selected default {} was removed; do not recreate it or choose a different Session/workspace.",outside.display(),file.display(),outside.display(),default.display())}
    })).await;
    wait_completed(&store, &session, &second).await;
    assert_eq!(std::fs::read(&file).unwrap(), CONTENT.as_bytes());
    assert!(!default.exists(), "global operations must not recreate the selected default");
    assert_eq!(store.get_live_session(&session).await.unwrap().agent_binding, binding);
    let effects = store.list_effects(&session).await.unwrap();
    assert_eq!(effects.len(), 2, "one write and one exact-cwd process, no replay or extra effects");
    assert!(effects.iter().all(|effect| effect.state == AgentEffectState::Returned));
    assert!(!store.has_unsettled_effects(&session).await.unwrap());
    let requests = requests.lock().unwrap();
    assert!((6..=7).contains(&requests.len()), "one initial Turn, four native calls and a formal bounded completion account/review");
    let emitted_native_calls = requests.iter().flat_map(|request| request["messages"].as_array().unwrap())
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["tool_call_id"].as_str())
        .filter(|id| !id.starts_with("missing-default-completion-"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(emitted_native_calls.len(), 4, "no native action was replayed for completion review");
    assert!(requests.last().unwrap()["messages"].to_string().contains(FIRST), "canonical prior context remains in the same Session");
    drop(requests);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}

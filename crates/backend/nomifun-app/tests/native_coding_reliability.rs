//! Replays the failure shapes from two dev Gomoku sessions through actual
//! preset compilation, provider encoding, Runtime, Kernel and filesystem owner.
//! The local scripted provider is deterministic evidence, not a model SLO.
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;

use axum::{Router, body::Body, http::Request};
use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
use nomifun_auth::AuthPolicy;
use serde_json::{Value, json};
use tower::ServiceExt;

const TRUST: &str = "coding-regression-fixture";
const HTML: &str = "<!DOCTYPE html>\n<html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>五子棋</title><canvas id=\"board\" width=\"600\" height=\"600\"></canvas><script>const size = 15;</script></html>";

#[cfg(all(feature="browser-use", feature="computer-use"))]
#[path="support/live_tool_reliability.rs"]
mod live_tool_reliability;

#[cfg(feature="browser-use")]
struct BindingOnlyBrowser(Arc<AtomicUsize>);

#[cfg(feature="browser-use")]
#[async_trait::async_trait]
impl nomifun_browser_platform::runtime::BrowserRuntimeFactory for BindingOnlyBrowser {
    async fn create(&self, _: nomifun_browser_platform::runtime::CreateBrowserRuntime)
        -> Result<Arc<dyn nomifun_browser_platform::runtime::BrowserRuntime>, nomifun_browser_platform::runtime::WorkspaceError>
    {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(nomifun_browser_platform::runtime::WorkspaceError::NativeUnavailable)
    }
}

async fn call(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{path}: {status}: {body}");
    body["data"].clone()
}

fn stream(tool: Option<(&str, &str, Value)>, text: &str) -> wiremock::ResponseTemplate {
    let (delta, reason) = match tool {
        Some((id, name, args)) => (json!({"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]}), "tool_calls"),
        None => (json!({"content":text}), "stop"),
    };
    stream_delta(delta, reason)
}

fn stream_delta(delta: Value, reason: &str) -> wiremock::ResponseTemplate {
    let data = json!({"id":"coding-regression", "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let done = json!({"id":"coding-regression", "choices":[{"index":0,"delta":{},"finish_reason":reason}]});
    wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {data}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn coding_preset_creates_nested_game_in_default_conversation_workspace() {
    scenario("coding.codex", false, false, false, false).await;
}

#[cfg(all(feature="browser-use", feature="computer-use"))]
#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn general_preset_repairs_native_protocol_then_creates_nested_game_in_selected_workspace() {
    scenario("assistant.general", true, true, false, false).await;
}

#[cfg(all(feature="browser-use", feature="computer-use"))]
#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn general_preset_keeps_non_coding_tools_discoverable_with_a_smaller_initial_context() {
    scenario("assistant.general", false, true, true, false).await;
}

#[cfg(all(target_os="macos", feature="browser-use", feature="computer-use"))]
#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn macos_presets_recover_truncation_write_a_batch_and_verify_with_a_native_process() {
    for template in ["coding.codex", "assistant.general"] {
        scenario(template, false, true, false, true).await;
    }
}

async fn scenario(template: &str, malformed_first: bool, selected_workspace: bool, discovery_first: bool, native_process: bool) {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("selected project with spaces");
    std::fs::create_dir(&project).unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir(&config.data_dir).unwrap();
    let upstream = wiremock::MockServer::start().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let is_general = template == "assistant.general";
    // Match the size of the real failed coding payload, rather than passing a
    // tiny write that never puts pressure on the General preset's context.
    let html = format!("{HTML}{}", "<!-- bounded coding scenario data -->".repeat(260));
    let generated_html = html.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let round = seen.fetch_add(1, Ordering::SeqCst);
            let tools = body["tools"].as_array().expect("preset must advertise native tools");
            if round == 0 {
                assert!(!body["messages"].as_array().unwrap().iter().any(|message|
                    message["role"] == "system" && message["content"].as_str().is_some_and(|text|
                        text.contains("<tool_call>") || text.contains("<function="))),
                    "default policy must describe the native interface without priming a competing wire syntax");
                // Opt-in export of this synthetic request for provider-format
                // diagnostics. No headers/credentials or user database data.
                if !is_general && let Ok(path) = std::env::var("NOMIFUN_TEST_CAPTURE_CODING_REQUEST") {
                    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(path).unwrap();
                    std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&body).unwrap()).unwrap();
                }
                for name in ["read_file", "write_file", "apply_patch"] {
                    assert!(tools.iter().any(|tool| tool["function"]["name"] == name), "missing {name}");
                }
                if is_general {
                    assert!(tools.iter().any(|tool| tool["function"]["name"] == "ToolSearch"));
                    assert!(!tools.iter().any(|tool| tool["function"]["description"].as_str().unwrap_or("").contains("Action: browser/navigate.")));
                    assert!(body.to_string().contains("browser/navigate"), "deferred capabilities must remain visible in the catalog");
                }
                eprintln!("CODING_CONTEXT general={is_general} tools={} schema_bytes={} request_bytes={}",
                    tools.len(), serde_json::to_vec(tools).unwrap().len(), request.body.len());
            }
            assert!(body["messages"].as_array().unwrap().iter().any(|message|
                message["role"] == "user" && message["content"].to_string().contains("帮我写一个五子棋的H5游戏")));
            if malformed_first && round == 0 {
                assert_eq!(body["tool_choice"], "auto");
                return stream(None, "<tool_call><function=write_file><parameter=content>REJECTED_PAYLOAD");
            }
            if native_process && round == 0 {
                return stream_delta(json!({"content":"<tool_call><function=write_file><parameter=path>discarded.html<parameter=content>UNFINISHED_TEXTUAL_TOOL_BODY"}), "length");
            }
            if discovery_first && round == 0 {
                return stream(Some(("find-browser", "ToolSearch", json!({"query":"browser/navigate"}))), "");
            }
            let raw_step = round - usize::from(malformed_first) - usize::from(discovery_first) - usize::from(native_process);
            if native_process && raw_step == 1 {
                return stream(Some(("bad-command-shape", "exec_command", json!({"command":"/bin/sh -c true"}))), "");
            }
            let step = raw_step - usize::from(native_process && raw_step > 1);
            if discovery_first {
                assert!(tools.iter().any(|tool| tool["function"]["description"].as_str().unwrap_or("").contains("Action: browser/navigate.")),
                    "discovery must reveal the original non-coding schema without changing preset authority");
            }
            assert!(tools.iter().any(|tool| tool["function"]["name"] == "write_file"),
                "workflow state must not look like a revoked capability");
            if malformed_first && step == 0 {
                assert_eq!(body["tool_choice"], json!({"type":"function","function":{"name":"write_file"}}),
                    "repair should constrain the known authorized function without executing its discarded text");
                assert_eq!(tools.len(),1,"one repair step must not invite an unrelated fallback command");
                assert!(!body.to_string().contains("REJECTED_PAYLOAD"));
            } else {
                assert_eq!(body["tool_choice"], "auto", "valid native output must restore normal final replies");
            }
            match step {
                0 if native_process => stream_delta(json!({"tool_calls":[
                    {"index":0,"id":"create-game","type":"function","function":{"name":"write_file",
                        "arguments":json!({"path":"gomoku/index.html","content":generated_html}).to_string()}},
                    {"index":1,"id":"create-check","type":"function","function":{"name":"write_file",
                        "arguments":json!({"path":"gomoku/check.txt","content":"MAC_PROCESS_OK"}).to_string()}}
                ]}), "tool_calls"),
                1 if native_process => {
                    assert!(body.to_string().contains("output-token ceiling"), "continuation must retain the truncation notice");
                    assert!(tools.iter().any(|tool| tool["function"]["name"] == "exec_command"));
                    assert!(body.to_string().contains("PROCESS_NOT_STARTED"), "a proven failed spawn needs a typed repair observation");
                    stream(Some(("read-game", "exec_command", json!({
                        "cmd":"test -s gomoku/index.html && test \"$(cat gomoku/check.txt)\" = MAC_PROCESS_OK && printf MAC_PROCESS_VERIFIED"}))), "")
                },
                2 => {
                    if native_process {
                    let result = body["messages"].as_array().unwrap().iter().rev()
                        .find(|message| message["role"] == "tool").unwrap()["content"].as_str().unwrap();
                    let result: Value = serde_json::from_str(result).unwrap();
                    assert_eq!(result["success"], true, "native process failed: {result}");
                    assert_eq!(result["exit_code"], 0);
                    assert!(result.to_string().contains("MAC_PROCESS_VERIFIED"));
                    } else {
                        let result = body["messages"].as_array().unwrap().iter().rev()
                            .find(|message| message["role"] == "tool" && message["tool_call_id"] == "read-game").unwrap()["content"].as_str().unwrap();
                        let result: Value = serde_json::from_str(result).unwrap();
                        assert_eq!(result["start_line"],2);
                        assert!(result["content"].as_str().unwrap().starts_with("<html"));
                        assert!(!result["content"].as_str().unwrap().contains("<!DOCTYPE"));
                        let repository = body["messages"].as_array().unwrap().iter().rev()
                            .find(|message| message["tool_call_id"] == "repo-status").unwrap()["content"].as_str().unwrap();
                        assert_eq!(serde_json::from_str::<Value>(repository).unwrap()["is_repository"],false);
                    }
                    let evidence = if native_process { json!({"evidence_call_ids":["read-game"]}) }
                        else { json!({"evidence_paths":["gomoku/index.html"]}) };
                    let mut criterion = json!({"step":"File delivery","disposition":"supported","rationale":"Fresh file or command observation"});
                    criterion.as_object_mut().unwrap().extend(evidence.as_object().unwrap().clone());
                    stream(Some(("account", "report_completion", json!({
                        "summary":"File path verified; gameplay not checked by this fixture",
                        "criteria":[criterion,{"step":"Gameplay","disposition":"unverified","rationale":"The fixture does not run a playable game"}]
                    }))), "")
                },
                0 => stream(Some(("create-game", "write_file", json!({"path":"gomoku/index.html","content":generated_html}))), ""),
                1 => stream_delta(json!({"tool_calls":[
                    {"index":0,"id":"read-game","type":"function","function":{"name":"read_file",
                        "arguments":json!({"path":"gomoku/index.html","start_line":2,"line_count":1,"recursive":false}).to_string()}},
                    {"index":1,"id":"repo-status","type":"function","function":{"name":"git_status","arguments":"{}"}},
                    {"index":2,"id":"fresh-byte-range","type":"function","function":{"name":"read_file",
                        "arguments":json!({"path":"gomoku/index.html","offset":17,"limit":20}).to_string()}},
                    {"index":3,"id":"scope-with-defaults","type":"function","function":{"name":"read_file",
                        "arguments":json!({"path":".","format":"instruction_scope","recursive":false,"missing_ok":false}).to_string()}}
                ]}),"tool_calls"),
                _ => panic!("unexpected retry or planning loop after nested file creation: {round}"),
            }
        }).mount(&upstream).await;
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    #[allow(unused_mut)]
    let mut app = AppServices::from_config(db, &config).await.unwrap();
    let browser_starts = Arc::new(AtomicUsize::new(0));
    #[cfg(feature="browser-use")]
    {
        app.browser_resources = Some(Arc::new(nomifun_browser_platform::workspace::BrowserResourceService::new(
            Arc::new(BindingOnlyBrowser(browser_starts.clone())),
        )));
    }
    let router = create_router(&app).await;
    let provider = call(&router, "POST", "/api/providers", json!({
        "platform":"stepfun-plan", "name":"scripted coding regression", "base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]}, "enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router, "POST", &format!("/api/agent-presets/from-template/{template}"),
        json!({"reuse_existing":false,"display_name":"coding regression","model":model})).await;
    let mut resources = json!([
        {"resource_kind":"workspace","resource_id":"default-workspace"},
        {"resource_kind":"process_session","resource_id":"managed-process-session"},
        {"resource_kind":"project_memory","resource_id":"default-project-memory"}
    ]);
    if template == "assistant.general" {
        resources.as_array_mut().unwrap().extend([
            json!({"resource_kind":"browser","resource_id":"managed-browser"}),
            json!({"resource_kind":"computer","resource_id":"local-desktop"}),
            json!({"resource_kind":"scheduler","resource_id":"installation-scheduler"}),
        ]);
    }
    let mut input = json!({"preset_id":preset["preset"]["preset_id"],"model":model,"resource_selections":resources});
    if selected_workspace { input["workspace"] = json!(project); }
    let session = call(&router, "POST", "/api/agent-sessions", input).await;
    let id = session["agent_session_id"].as_str().unwrap();
    call(&router, "POST", &format!("/api/agent-sessions/{id}/turns"),
        json!({"idempotency_key":"gomoku-regression","input":{"content":"帮我写一个五子棋的H5游戏"}})).await;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let state = call(&router, "GET", &format!("/api/agent-sessions/{id}/execution"), Value::Null).await;
            if state["state"] == "completed" { break; }
            assert_eq!(state["state"], "running", "coding did not complete: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("coding task must settle in bounded time");
    let workspace = if selected_workspace { project } else { config.work_dir.join("conversations").join(id) };
    assert_eq!(std::fs::read_to_string(workspace.join("gomoku/index.html")).unwrap(), html);
    assert!(!workspace.join("discarded.html").exists(), "truncated calls never execute");
    assert!(!workspace.join(".git").exists(),"status probing must not initialize Git");
    if native_process {
        let discarded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='model_output_truncated'")
            .bind(id).fetch_one(app.database.pool()).await.unwrap();
        assert_eq!(discarded, 1, "the complete discard fact remains in canonical history");
        let protocol_errors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='model_response_rejected'")
            .bind(id).fetch_one(app.database.pool()).await.unwrap();
        assert_eq!(protocol_errors,0,"text spilled by a length-limited provider must not enter protocol retries");
    }
    if native_process { assert_eq!(std::fs::read_to_string(workspace.join("gomoku/check.txt")).unwrap(), "MAC_PROCESS_OK"); }
    assert_eq!(requests.load(Ordering::SeqCst), 3 + usize::from(malformed_first) + usize::from(discovery_first) + 2 * usize::from(native_process));
    let delivered: String = sqlx::query_scalar("SELECT json_extract(inline_json,'$.event.text') FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='completion_delivered'")
        .bind(id).fetch_one(app.database.pool()).await.unwrap();
    assert!(delivered.contains("File path verified; gameplay not checked by this fixture"));
    assert!(delivered.contains("The fixture does not run a playable game"), "unverified scope must reach the final delivery");
    for (kind, count) in [("effect/failed", 0_i64), ("turn/failed", 0), ("turn/completed", 1)] {
        let actual: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind=?")
            .bind(id).bind(kind).fetch_one(app.database.pool()).await.unwrap();
        assert_eq!(actual, count, "{kind}");
    }
    let writes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_started' AND json_extract(inline_json,'$.event.action_id')='workspace.files/write'")
        .bind(id).fetch_one(app.database.pool()).await.unwrap();
    assert_eq!(writes, if native_process { 2 } else { 1 }, "rejected/truncated text cannot execute or duplicate a write");
    let compactions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='compaction_started'")
        .bind(id).fetch_one(app.database.pool()).await.unwrap();
    assert_eq!(compactions, 0, "a bounded single-file task must not turn into serial summarization");
    assert_eq!(browser_starts.load(Ordering::SeqCst), 0, "binding/discovering a capability must not start its heavy runtime");
    drop(router);
    app.shutdown_browser_platform().await.unwrap();
    app.database.close().await;
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
async fn delegated_provider_pause_reaches_scheduler_without_waiting_thirty_minutes() {
    let root = tempfile::tempdir().unwrap();
    let config = AppConfig { data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default() };
    std::fs::create_dir(&config.data_dir).unwrap();
    let upstream = wiremock::MockServer::start().await;
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                let delegate = body["tools"].as_array().unwrap().iter().find(|tool|
                    tool["function"]["description"].as_str().unwrap_or("").contains("Action: agent/delegate."))
                    .expect("coding preset exposes its admitted collaboration tool");
                stream(Some(("delegate-once", delegate["function"]["name"].as_str().unwrap(), json!({
                    "strategy":"parallel", "tasks":[{"name":"Create file","prompt":"Create answer.txt containing ok"}], "synthesize":false,
                }))), "")
            } else {
                wiremock::ResponseTemplate::new(400).set_body_json(json!({"error":{"message":"fixture rejects child request","type":"invalid_request_error"}}))
            }
        }).mount(&upstream).await;
    let db = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let app = AppServices::from_config(db, &config).await.unwrap();
    let router = create_router(&app).await;
    let provider = call(&router,"POST","/api/providers",json!({
        "platform":"stepfun-plan","name":"pause propagation fixture","base_url":format!("{}/v1",upstream.uri()),
        "auth_scheme":"bearer","credentials":{"api_keys":["test-only"]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"step-3.7-flash"});
    let preset = call(&router,"POST","/api/agent-presets/from-template/coding.codex",json!({"reuse_existing":false,"display_name":"Pause regression","model":model})).await;
    let session = call(&router,"POST","/api/agent-sessions",json!({"preset_id":preset["preset"]["preset_id"],"model":model,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"},{"resource_kind":"process_session","resource_id":"managed-process-session"},{"resource_kind":"project_memory","resource_id":"default-project-memory"}]})).await;
    let id = session["agent_session_id"].as_str().unwrap();
    call(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"idempotency_key":"pause-regression","input":{"content":"Delegate creation of answer.txt to one worker."}})).await;
    let failure: String = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT a.status,a.error FROM agent_execution_attempts a JOIN conversation_execution_links l ON l.execution_id=a.execution_id WHERE l.conversation_id=? AND l.relation='lead'")
                .bind(id).fetch_optional(app.database.pool()).await.unwrap();
            if let Some((status, error)) = row {
                if status == "failed" { break error.unwrap(); }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("paused child must settle promptly instead of waiting for the 30-minute receipt timeout");
    assert!(failure.contains("agent_execution_paused"), "{failure}");
    assert!(failure.contains("EXECUTION_MODEL_INVALID_REQUEST"), "{failure}");
    assert!(!failure.contains("receipt_missing"));
    assert_eq!(requests.load(Ordering::SeqCst),2,"neither the handoff nor the paused task may be replayed");
    let parent = call(&router,"GET",&format!("/api/agent-sessions/{id}/execution"),Value::Null).await;
    assert_eq!(parent["state"],"completed");
    drop(router);
    app.shutdown_browser_platform().await.unwrap(); app.database.close().await;
}

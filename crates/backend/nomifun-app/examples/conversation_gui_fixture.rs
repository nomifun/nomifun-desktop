//! Deterministic acceptance through the real desktop, Runtime, tools and history.
//! Modes include --creative-failure, --creative-submit-failure,
//! --creative-retry-ack-loss, --shutdown-wait, --crash-tree, --lease-retirement
//! --active-quit (one long owned exec, no pending model request), and
//! --success-recovery (one completed write, held model-only tail, fresh read).
//! Launch NomiFun with that NOMIFUN_DATA_DIR; send a normal request, inspect the
//! live journal, POST /finish to release the final response, then reload. Send
//! "格式异常" in a second turn to exercise split pseudo-tool-call rejection.
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use nomifun_app::{DesktopHostServices, DesktopServer};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

struct Fixture {
    calls: AtomicUsize,
    creative_failure: bool,
    creative_submit_failure: bool,
    creative_retry_ack_loss: bool,
    shutdown_wait: bool,
    success_recovery: bool,
    recovery_armed: AtomicBool,
    recovery_wait_observed: AtomicBool,
    crash_tree: bool,
    process_timeout_ms: u64,
    tree_script: std::sync::OnceLock<PathBuf>,
    waiting_streams: AtomicUsize,
    finish: Semaphore,
    stop: CancellationToken,
}

struct WaitingStream(Arc<Fixture>);

impl Drop for WaitingStream {
    fn drop(&mut self) {
        self.0.waiting_streams.fetch_sub(1, Ordering::SeqCst);
    }
}

fn frame(delta: Value, finish: Option<&str>) -> String {
    format!("data: {}\n\n", json!({
        "id":"journal-gui", "object":"chat.completion.chunk", "created":1,
        "model":"journal-fixture", "choices":[{"index":0,"delta":delta,"finish_reason":finish}]
    }))
}

const RECOVERY_FILE: &str = "recovery-check.txt";
const RECOVERY_CONTENT: &str = "GLOBAL_C_AUTO_RECOVERY_OK\n";
const RECOVERY_WRITE: &str = "global-c-write-once";
const RECOVERY_REPLAN: &str = "global-c-replan-fresh";
const RECOVERY_READ: &str = "global-c-read-fresh";
const RECOVERY_CLOSE: &str = "global-c-plan-close-fresh";
const RECOVERY_REPORT: &str = "global-c-report-fresh";

enum RecoveryReply { Hold, Tool(&'static str, &'static str, Value) }

fn recovery_result(body: &Value, id: &str) -> Option<Value> {
    let messages = body["messages"].as_array()?.iter().filter(|message|
        message["role"] == "tool" && message["tool_call_id"] == id).collect::<Vec<_>>();
    let [message] = messages.as_slice() else { return None; };
    if message.get("is_error") == Some(&Value::Bool(true)) { return None; }
    let result: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
    (result.is_object() && result.get("is_error") != Some(&Value::Bool(true))).then_some(result)
}

fn recovery_write_returned(body: &Value) -> bool {
    recovery_result(body, RECOVERY_WRITE).is_some_and(|result|
        result["written"] == true && result["path"] == RECOVERY_FILE
            && result["bytes"] == RECOVERY_CONTENT.len()
            && result["sha256"] == format!("{:x}", Sha256::digest(RECOVERY_CONTENT.as_bytes())))
}

fn recovery_read_returned(body: &Value) -> bool {
    recovery_result(body, RECOVERY_READ).is_some_and(|result|
        result["path"] == RECOVERY_FILE && result["content"] == RECOVERY_CONTENT
            && result["eof"] == true && result["offset"] == 0
            && result["total_bytes"] == RECOVERY_CONTENT.len()
            && result["sha256"] == format!("{:x}", Sha256::digest(RECOVERY_CONTENT.as_bytes())))
}

fn recovery_plan_returned(body: &Value, id: &str, state: &str) -> bool {
    recovery_result(body, id).is_some_and(|result|
        matches!(result["status"].as_str(), Some("updated" | "unchanged"))
            && result["needs_replan"] == false
            && result["requirement_ids"].as_array().is_some_and(|ids| ids.iter().any(|id| id == "input_0"))
            && result["plan"] == json!([{"step":"Verify saved file","status":state}]))
}

fn recovery_reply(call: usize, body: &Value, armed: bool) -> Result<RecoveryReply, &'static str> {
    let written = recovery_write_returned(body);
    let read_matches = recovery_read_returned(body);
    let reply = match call {
        0 if !armed => RecoveryReply::Tool(RECOVERY_WRITE, "write_file",
            json!({"path":RECOVERY_FILE,"content":RECOVERY_CONTENT})),
        1 if !armed && written => RecoveryReply::Hold,
        2 if armed && written => RecoveryReply::Tool(RECOVERY_REPLAN, "update_plan",
            json!({"explanation":"Reconsider the saved task after cold recovery; do not repeat its completed write.",
                "plan":[{"step":"Verify saved file","status":"in_progress"}]})),
        3 if armed && written && recovery_plan_returned(body, RECOVERY_REPLAN, "in_progress") => RecoveryReply::Tool(RECOVERY_READ, "read_file",
            json!({"path":RECOVERY_FILE})),
        4 if armed && written && read_matches => RecoveryReply::Tool(RECOVERY_CLOSE, "update_plan",
            json!({"explanation":"The fresh read confirms the saved content.",
                "plan":[{"step":"Verify saved file","status":"completed"}]})),
        5 if armed && written && recovery_plan_returned(body, RECOVERY_CLOSE, "completed") && read_matches => RecoveryReply::Tool(RECOVERY_REPORT, "report_completion",
            json!({"summary":format!("冷恢复后已回读 recovery-check.txt，当前完整正文（包括末尾 LF）为：\n{RECOVERY_CONTENT}"),
                "criteria":[{"step":"Verify saved file","disposition":"supported",
                    "evidence_call_ids":[RECOVERY_READ],
                    "rationale":"The fresh read confirms the requested file content after cold recovery."}]})),
        _ => return Err("SUCCESS_RECOVERY_UNEXPECTED_REQUEST_OR_MISSING_RESULT"),
    };
    if let RecoveryReply::Tool(_, name, arguments) = &reply {
        let tools = body["tools"].as_array().ok_or("SUCCESS_RECOVERY_TOOL_NOT_EXPOSED")?;
        let function = tools.iter().find(|tool| tool["function"]["name"] == *name)
            .map(|tool| &tool["function"]).ok_or("SUCCESS_RECOVERY_TOOL_NOT_EXPOSED")?;
        let validator = jsonschema::validator_for(&function["parameters"])
            .map_err(|_| "SUCCESS_RECOVERY_TOOL_SCHEMA_INVALID")?;
        if !function["parameters"].is_object() || !validator.is_valid(arguments) {
            return Err("SUCCESS_RECOVERY_ARGUMENTS_NOT_ADVERTISED");
        }
        if *name == "report_completion" && !function["parameters"]
            .pointer("/properties/criteria/items/properties/evidence_call_ids/items/enum")
            .and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|id| id == RECOVERY_READ)) {
            return Err("SUCCESS_RECOVERY_FRESH_READ_NOT_ELIGIBLE");
        }
    }
    Ok(reply)
}

fn recovery_rejection(message: &'static str) -> axum::response::Response {
    axum::response::Response::builder().status(400).header("content-type", "application/json")
        .body(axum::body::Body::from(json!({"error":{"message":message,"type":"invalid_request_error"}}).to_string())).unwrap()
}

async fn success_recovery_model(fixture: Arc<Fixture>, body: Value, call: usize) -> axum::response::Response {
    match recovery_reply(call, &body, fixture.recovery_armed.load(Ordering::SeqCst)) {
        Err(message) => recovery_rejection(message),
        Ok(RecoveryReply::Hold) => {
            fixture.recovery_wait_observed.store(true, Ordering::SeqCst);
            fixture.waiting_streams.fetch_add(1, Ordering::SeqCst);
            // Comments only: no text, tool proposal, finish or semantic output
            // may arrive after the production checkpoint before the GUI fault.
            // /finish deliberately cannot release this original response.
            let stream = futures_util::stream::unfold(WaitingStream(fixture), |guard| async move {
                let current = Arc::clone(&guard.0);
                tokio::select! {
                    _ = current.stop.cancelled() => None,
                    _ = tokio::time::sleep(Duration::from_millis(250)) =>
                        Some((Ok::<_, std::io::Error>(": SUCCESS_RECOVERY_WAIT_FOR_GUI_FAULT\n\n".to_owned()), guard)),
                }
            });
            axum::response::Response::builder().header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(stream)).unwrap()
        },
        Ok(RecoveryReply::Tool(id, name, arguments)) => {
            let frames = vec![frame(json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
                "function":{"name":name,"arguments":arguments.to_string()}}]}), None),
                frame(json!({}), Some("tool_calls")), "data: [DONE]\n\n".into()];
            let stream = futures_util::stream::unfold(frames.into_iter(), |mut frames| async move {
                let next = frames.next()?;
                tokio::time::sleep(Duration::from_millis(180)).await;
                Some((Ok::<_, std::io::Error>(next), frames))
            });
            axum::response::Response::builder().header("content-type", "text/event-stream")
                .body(axum::body::Body::from_stream(stream)).unwrap()
        },
    }
}

async fn arm_success_recovery(State(f): State<Arc<Fixture>>) -> (axum::http::StatusCode, Json<Value>) {
    if !f.success_recovery || !f.recovery_wait_observed.load(Ordering::SeqCst)
        || f.waiting_streams.load(Ordering::SeqCst) != 0
        || (f.calls.load(Ordering::SeqCst) != 2 && !f.recovery_armed.load(Ordering::SeqCst)) {
        return (axum::http::StatusCode::CONFLICT, Json(json!({"armed":false,
            "reason":"Original held stream must be dropped; operator must independently prove the exact GUI fault and checkpoint."})));
    }
    f.recovery_armed.store(true, Ordering::SeqCst);
    (axum::http::StatusCode::OK, Json(json!({"armed":true,"grants_recovery_authority":false})))
}

async fn model(State(fixture): State<Arc<Fixture>>, Json(body): Json<Value>) -> axum::response::Response {
    let call = fixture.calls.fetch_add(1, Ordering::SeqCst);
    if fixture.success_recovery { return success_recovery_model(fixture, body, call).await; }
    let active_quit = fixture.crash_tree && fixture.process_timeout_ms == 600000;
    if active_quit && call >= 1 {
        return axum::response::Response::builder().status(400)
            .header("content-type", "application/json")
            .body(axum::body::Body::from("{\"error\":{\"message\":\"ACTIVE_QUIT_SINGLE_EXEC_ONLY\"}}"))
            .unwrap();
    }
    if fixture.crash_tree && call >= 2 {
        // Cold recovery may resume this same accepted Turn. Keep its inference
        // observable while preventing the fixture from proposing another tree.
        return axum::response::Response::builder().status(400)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json!({"error":{"message":"CRASH_RECOVERY_OBSERVATION_ONLY","type":"invalid_request_error"}}).to_string())).unwrap();
    }
    if fixture.creative_retry_ack_loss {
        let stream = futures_util::stream::unfold((fixture, 0_u8), |(fixture, phase)| async move {
            match phase {
                0 => {
                    let current = Arc::clone(&fixture);
                    tokio::select! {
                        _ = current.stop.cancelled() => None,
                        permit = current.finish.acquire() => {
                            permit.ok()?.forget();
                            Some((Ok::<_, std::io::Error>(frame(json!({"role":"assistant",
                                "content":"MM_RETRY_VERIFIED_FIXTURE：建议以蓝色与留白设计主题海报。本次仅提供创作方案，未改动画布。"}), None)), (fixture, 1)))
                        },
                        _ = tokio::time::sleep(Duration::from_millis(250)) =>
                            Some((Ok(": fixture keepalive\n\n".to_owned()), (fixture, 0))),
                    }
                },
                1 => Some((Ok(format!("{}data: [DONE]\n\n", frame(json!({}), Some("stop")))), (fixture, 2))),
                _ => None,
            }
        });
        return axum::response::Response::builder().header("content-type", "text/event-stream")
            .body(axum::body::Body::from_stream(stream)).unwrap();
    }
    if fixture.creative_submit_failure {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::BAD_REQUEST)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json!({"error":{
                "message":"MM_SUBMIT_UNCONFIRMED_FIXTURE", "type":"invalid_request_error"
            }}).to_string()))
            .unwrap();
    }
    if fixture.creative_failure {
        let frames = vec![
            frame(
                json!({"role":"assistant","content":"MM_HISTORY_FAILURE_FIXTURE"}),
                None,
            ),
            // The accepted stream terminates with an impossible tool-call
            // reason and no calls. Runtime must settle the canonical Turn as
            // failed so a later cold load observes history, not a pending retry.
            frame(json!({}), Some("tool_calls")),
            "data: [DONE]\n\n".into(),
        ];
        let stream = futures_util::stream::unfold(frames.into_iter(), |mut frames| async move {
            let frame = frames.next()?;
            tokio::time::sleep(Duration::from_millis(180)).await;
            Some((Ok::<_, std::io::Error>(frame), frames))
        });
        return axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(axum::body::Body::from_stream(stream))
            .unwrap();
    }
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let malformed = messages.iter().rev().find(|message| message["role"] == "user")
        .is_some_and(|message| message["content"].to_string().contains("格式异常"));
    let has_tool = messages.iter().rev().take_while(|message| message["role"] != "user")
        .any(|message| message["role"] == "tool");
    if fixture.shutdown_wait && (has_tool || (fixture.crash_tree && call == 1)) {
        fixture.waiting_streams.fetch_add(1, Ordering::SeqCst);
        let stream = futures_util::stream::unfold((WaitingStream(fixture),0_u8), |(guard,phase)| async move {
            match phase {
                0 => {
                    let content = if guard.0.crash_tree { "原进程已启动，正在等待故障测试控制。" } else { "检查文件已经写入，正在等待后续检查。" };
                    Some((Ok::<_,std::io::Error>(frame(json!({"role":"assistant","content":content}),None)),(guard,1)))
                },
                1 => {
                    let current = Arc::clone(&guard.0);
                    tokio::select! {
                    _ = current.stop.cancelled() => None,
                    permit = current.finish.acquire() => {
                        if let Ok(permit) = permit { permit.forget(); }
                        Some((Ok(format!("{}data: [DONE]\n\n",frame(json!({}),Some("stop")))),(guard,2)))
                    },
                    _ = tokio::time::sleep(Duration::from_millis(250)) =>
                        Some((Ok(": fixture keepalive\n\n".to_owned()),(guard,1))),
                    }
                },
                _ => None,
            }
        });
        return axum::response::Response::builder()
            .header("content-type","text/event-stream")
            .body(axum::body::Body::from_stream(stream)).unwrap();
    }
    let mut frames = if fixture.crash_tree {
        vec![
            frame(json!({"role":"assistant","content":"正在启动隔离守候进程。"}), None),
            frame(json!({"tool_calls":[{"index":0,"id":"crash-tree-start","type":"function","function":{
                "name":if active_quit { "exec_command" } else { "start_process" },
                "arguments":if active_quit {
                    json!({"command":"bun","args":[fixture.tree_script.get().expect("prepared crash tree")],"tty":false,"timeout_ms":600000}).to_string()
                } else {
                    json!({"command":"bun","args":[fixture.tree_script.get().expect("prepared crash tree")],"tty":false,"wait_ms":500,"timeout_ms":fixture.process_timeout_ms}).to_string()
                }
            }}]}), None),
            frame(json!({}), Some("tool_calls")),
        ]
    } else if malformed {
        vec![
            frame(json!({"role":"assistant","content":"正在准备文件。 <to"}), None),
            frame(json!({"content":"ol_call>\n<fun"}), None),
            frame(json!({"content":"ction=write_file>\n<parameter=content>RAW_PAYLOAD_MUST_NOT_APPEAR"}), None),
            frame(json!({}), Some("stop")),
        ]
    } else if !has_tool {
        vec![
            frame(json!({"role":"assistant","content":"我会创建一个检查文件，"}), None),
            frame(json!({"content":"完成后给出结果。"}), None),
            frame(json!({"tool_calls":[{"index":0,"id":"journal-write","type":"function","function":{
                "name":"write_file","arguments":json!({"path":"journal-check.md","content":"# 会话回归检查\n\n- 实时说明\n- 阶段工具回执\n- 结果文件\n"}).to_string()
            }}]}), None),
            frame(json!({}), Some("tool_calls")),
        ]
    } else {
        tokio::select! {
            _ = fixture.stop.cancelled() => {},
            permit = fixture.finish.acquire() => { if let Ok(permit) = permit { permit.forget(); } },
        }
        vec![
            frame(json!({"role":"assistant","content":"文件已创建。\n\n"}), None),
            frame(json!({"content":"**检查结果**\n\n- 说明与操作按顺序展示\n- 工具明细可展开\n"}), None),
            frame(json!({"content":"- 结果文件在正文下方\n\n打开 `journal-check.md` 可查看内容。"}), None),
            frame(json!({}), Some("stop")),
        ]
    };
    frames.push("data: [DONE]\n\n".into());
    let stream = futures_util::stream::unfold(frames.into_iter(), |mut frames| async move {
        let frame = frames.next()?;
        tokio::time::sleep(Duration::from_millis(180)).await;
        Some((Ok::<_, std::io::Error>(frame), frames))
    });
    axum::response::Response::builder().header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream)).unwrap()
}

async fn api(app: &DesktopServer, path: &str, body: Value) -> anyhow::Result<Value> {
    let response = reqwest::Client::builder().no_proxy().build()?
        .post(format!("http://127.0.0.1:{}{path}", app.loopback_port()))
        .header("x-nomi-local-trust", app.local_trust_secret()).json(&body).send().await?;
    let status = response.status();
    let value: Value = response.json().await?;
    anyhow::ensure!(status.is_success(), "fixture setup {path}: {status} {value}");
    Ok(value["data"].clone())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("new absolute data directory required"))?);
    anyhow::ensure!(root.is_absolute() && !root.exists(), "refusing an existing data directory");
    let mode = std::env::args().nth(2);
    let creative_failure = mode.as_deref() == Some("--creative-failure");
    let creative_submit_failure = mode.as_deref() == Some("--creative-submit-failure");
    let creative_retry_ack_loss = mode.as_deref() == Some("--creative-retry-ack-loss");
    let lease_retirement = mode.as_deref() == Some("--lease-retirement");
    let active_quit = mode.as_deref() == Some("--active-quit");
    let success_recovery = mode.as_deref() == Some("--success-recovery");
    let crash_tree = mode.as_deref() == Some("--crash-tree") || lease_retirement || active_quit;
    let process_timeout_ms = if active_quit { 600000 } else if lease_retirement { 1000 } else { 30000 };
    let shutdown_wait = mode.as_deref() == Some("--shutdown-wait") || crash_tree;
    anyhow::ensure!(mode.is_none() || creative_failure || creative_submit_failure || creative_retry_ack_loss || shutdown_wait || success_recovery, "unsupported fixture mode");
    std::fs::create_dir(&root)?;
    let fixture = Arc::new(Fixture {
        calls: AtomicUsize::new(0),
        creative_failure,
        creative_submit_failure,
        creative_retry_ack_loss,
        shutdown_wait,
        success_recovery,
        recovery_armed: AtomicBool::new(false),
        recovery_wait_observed: AtomicBool::new(false),
        crash_tree,
        process_timeout_ms,
        tree_script: std::sync::OnceLock::new(),
        waiting_streams: AtomicUsize::new(0),
        finish: Semaphore::new(0),
        stop: CancellationToken::new(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let routes = Router::new().route("/v1/chat/completions", post(model))
        .route("/arm-recovery", post(arm_success_recovery))
        .route("/finish", post(|State(f): State<Arc<Fixture>>| async move { f.finish.add_permits(1); "released" }))
        .route("/status", get(|State(f): State<Arc<Fixture>>| async move { Json(json!({"calls":f.calls.load(Ordering::SeqCst),
            "creative_failure":f.creative_failure,"creative_submit_failure":f.creative_submit_failure,"creative_retry_ack_loss":f.creative_retry_ack_loss,"shutdown_wait":f.shutdown_wait,
            "crash_tree":f.crash_tree,"process_timeout_ms":f.process_timeout_ms,"waiting_streams":f.waiting_streams.load(Ordering::SeqCst),
            "success_recovery":f.success_recovery,"recovery_armed":f.recovery_armed.load(Ordering::SeqCst),
            "recovery_wait_observed":f.recovery_wait_observed.load(Ordering::SeqCst)})) }))
        .route("/shutdown", post(|State(f): State<Arc<Fixture>>| async move { f.stop.cancel(); "stopped" }))
        .with_state(fixture.clone());
    let stop = fixture.stop.clone();
    tokio::spawn(async move { axum::serve(listener, routes).with_graceful_shutdown(stop.cancelled_owned()).await });
    let cli = nomifun_app::cli::Cli {
        host:"127.0.0.1".into(), port:0, data_dir:root.clone(),
        work_dir:(shutdown_wait || success_recovery).then(|| root.parent().unwrap().join("work")),
        app_version:env!("CARGO_PKG_VERSION").into(), local:true,
        log_dir:Some(root.join("logs")), log_level:Some("off".into()), command:None,
    };
    let (app, keep_alive) = DesktopServer::start_with_outcome(&cli,"",None,None,None,DesktopHostServices::default()).await?;
    let prepared = async {
        let provider = api(&app,"/api/providers",json!({"platform":"custom","name":"会话回归测试模型","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"journal-fixture","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider = provider["provider_id"].as_str().ok_or_else(||anyhow::anyhow!("provider missing"))?;
        if creative_failure || creative_submit_failure || creative_retry_ack_loss {
            let canvas = api(&app,"/api/creative-studio/canvases",json!({
                "title":"MM 旧失败重试验收",
                "agentKickoff":{
                    "prompt":"请为这个空画布提出一个创作方案。",
                    "model":{"providerId":provider,"model":"journal-fixture"}
                }
            })).await?;
            let canvas_id = canvas["canvas"]["canvasId"].as_str()
                .ok_or_else(||anyhow::anyhow!("creative failure canvas missing"))?;
            Ok::<_,anyhow::Error>(json!({"canvas_id":canvas_id}))
        } else {
            let editor = api(&app,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"会话内容回归","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"journal-fixture"}})).await?;
            let preset = editor["preset"]["preset_id"].as_str().ok_or_else(||anyhow::anyhow!("preset missing"))?;
            let mut draft = editor["draft"].clone();
            draft["document"]["enabled_capabilities"] = if crash_tree {
                json!([
                    {"capability":{"id":"workspace.process"},"action_allowlist":if active_quit {json!(["workspace.process/exec"])} else {json!(["workspace.process/start"])}},
                    {"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read"]}
                ])
            } else { json!([{"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/write"]}]) };
            api(&app,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],"draft":draft,"reason":"real desktop conversation regression fixture"})).await?;
            let mut resources = vec![json!({"resource_kind":"workspace","resource_id":"default-workspace"})];
            if crash_tree { resources.push(json!({"resource_kind":"process_session","resource_id":"managed-process-session"})); }
            let session = api(&app,"/api/agent-sessions",json!({"preset_id":preset,"title":"会话内容回归 · 正常与异常","resource_selections":resources,"model":{"provider_id":provider,"model":"journal-fixture"}})).await?;
            if crash_tree {
                let workspace = root.parent().unwrap().join("work/conversations").join(session["agent_session_id"].as_str().unwrap());
                std::fs::create_dir_all(&workspace)?;
                std::fs::write(workspace.join("tree.mjs"), r#"const file = import.meta.path;
const hold = () => setInterval(() => {}, 1000);
if (process.argv[2] === 'grandchild') { hold(); }
else if (process.argv[2] === 'child') {
  const grandchild = Bun.spawn([process.execPath, file, 'grandchild'], {stdin:'ignore',stdout:'ignore',stderr:'ignore'});
  process.stdout.write(JSON.stringify({child:process.pid,grandchild:grandchild.pid})+'\n');
  hold();
} else {
  const child = Bun.spawn([process.execPath, file, 'child'], {stdin:'ignore',stdout:'pipe',stderr:'ignore'});
  const reader = child.stdout.getReader(); let bytes = '';
  while (!bytes.includes('\n')) { const part=await reader.read(); if(part.done)throw new Error('child exited before readiness'); bytes+=new TextDecoder().decode(part.value); }
  const descendants=JSON.parse(bytes.split('\n')[0]);
  process.stdout.write('CRASH_TREE_READY '+JSON.stringify({parent:process.pid,...descendants})+'\n');
  hold();
}
"#)?;
                fixture.tree_script.set(workspace.join("tree.mjs")).map_err(|_|anyhow::anyhow!("tree script already prepared"))?;
            }
            Ok::<_,anyhow::Error>(json!({"session_id":session["agent_session_id"].clone()}))
        }
    }.await;
    app.shutdown_all().await?;
    drop(app); drop(keep_alive);
    let prepared = prepared?;
    println!("CONVERSATION_GUI_FIXTURE_READY {}",json!({
        "data_dir":root,"control":format!("http://{address}"),
        "session_id":prepared.get("session_id"),"canvas_id":prepared.get("canvas_id"),
        "creative_failure":creative_failure,"creative_submit_failure":creative_submit_failure,"creative_retry_ack_loss":creative_retry_ack_loss,"shutdown_wait":shutdown_wait,"crash_tree":crash_tree,"process_timeout_ms":process_timeout_ms,
        "success_recovery":success_recovery
    }));
    fixture.stop.cancelled().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recovery_test_body() -> Value {
        let plan = json!({"type":"object","additionalProperties":false,"required":["plan"],"properties":{
            "explanation":{"type":"string"},"plan":{"type":"array","minItems":1,"items":{
                "type":"object","additionalProperties":false,"required":["step","status"],"properties":{
                    "step":{"type":"string"},"status":{"enum":["in_progress","completed"]}}}}}});
        let report = json!({"type":"object","additionalProperties":false,"required":["summary","criteria"],"properties":{
            "summary":{"type":"string"},"criteria":{"type":"array","minItems":1,"items":{
                "type":"object","additionalProperties":false,"required":["disposition","rationale","evidence_call_ids"],
                "properties":{"step":{"type":"string"},"disposition":{"const":"supported"},"rationale":{"type":"string"},
                    "evidence_call_ids":{"type":"array","minItems":1,"items":{"enum":[RECOVERY_READ]}}}}}}});
        json!({"messages":[],"tools":[
            {"type":"function","function":{"name":"write_file","parameters":{
                "type":"object","additionalProperties":false,"required":["path","content"],"properties":{
                    "path":{"const":RECOVERY_FILE},"content":{"const":RECOVERY_CONTENT}}}}},
            {"type":"function","function":{"name":"read_file","parameters":{
                "type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"const":RECOVERY_FILE}}}}},
            {"type":"function","function":{"name":"update_plan","parameters":plan}},
            {"type":"function","function":{"name":"report_completion","parameters":report}}
        ]})
    }

    fn recovery_test_receipt(id: &str) -> Value {
        let digest = format!("{:x}", Sha256::digest(RECOVERY_CONTENT.as_bytes()));
        match id {
            RECOVERY_WRITE => json!({"written":true,"path":RECOVERY_FILE,"bytes":RECOVERY_CONTENT.len(),"sha256":digest}),
            RECOVERY_READ => json!({"path":RECOVERY_FILE,"content":RECOVERY_CONTENT,"sha256":digest,
                "total_bytes":RECOVERY_CONTENT.len(),"offset":0,"eof":true}),
            RECOVERY_REPLAN | RECOVERY_CLOSE => json!({"status":"updated","needs_replan":false,"requirement_ids":["input_0"],
                "plan":[{"step":"Verify saved file","status":if id==RECOVERY_REPLAN {"in_progress"} else {"completed"}}]}),
            _ => json!({"status":"accepted"}),
        }
    }

    fn recovery_test_add_receipt(body: &mut Value, id: &str) {
        body["messages"].as_array_mut().unwrap().push(json!({"role":"tool","tool_call_id":id,
            "content":recovery_test_receipt(id).to_string()}));
    }

    #[test]
    fn success_recovery_rejects_unexecuted_or_incomplete_results_and_unadvertised_controls() {
        let mut body = recovery_test_body();
        body["messages"] = json!([{"role":"tool","tool_call_id":RECOVERY_WRITE,"content":"Not executed: invalid arguments"}]);
        assert!(recovery_reply(1, &body, false).is_err());
        body["messages"] = json!([]);
        recovery_test_add_receipt(&mut body, RECOVERY_WRITE);
        let mut failed_write = recovery_test_receipt(RECOVERY_WRITE);
        failed_write["written"] = json!(false);
        body["messages"][0]["content"] = json!(failed_write.to_string());
        assert!(recovery_reply(1, &body, false).is_err());
        body["messages"][0]["content"] = json!(recovery_test_receipt(RECOVERY_WRITE).to_string());
        assert!(matches!(recovery_reply(1, &body, false).unwrap(), RecoveryReply::Hold));
        let mut unadvertised = body.clone();
        unadvertised["tools"].as_array_mut().unwrap().retain(|tool| tool["function"]["name"] != "update_plan");
        assert!(recovery_reply(2, &unadvertised, true).is_err());
        recovery_test_add_receipt(&mut body, RECOVERY_REPLAN);
        let mut rejected_plan = recovery_test_receipt(RECOVERY_REPLAN);
        rejected_plan["status"] = json!("rejected");
        body["messages"][1]["content"] = json!(rejected_plan.to_string());
        assert!(recovery_reply(3, &body, true).is_err());
        body["messages"][1]["content"] = json!(recovery_test_receipt(RECOVERY_REPLAN).to_string());
        recovery_test_add_receipt(&mut body, RECOVERY_READ);
        let mut partial = recovery_test_receipt(RECOVERY_READ);
        partial["eof"] = json!(false);
        body["messages"][2]["content"] = json!(partial.to_string());
        assert!(recovery_reply(4, &body, true).is_err());
        body["messages"][2]["content"] = json!(recovery_test_receipt(RECOVERY_READ).to_string());
        recovery_test_add_receipt(&mut body, RECOVERY_CLOSE);
        let RecoveryReply::Tool(_, _, arguments) = recovery_reply(5, &body, true).unwrap() else { panic!("report expected"); };
        assert!(!arguments["criteria"][0].as_object().unwrap().contains_key("requirement_ids"),
            "the current advertised omission covers all immutable accepted input without guessed IDs");
        assert!(arguments["summary"].as_str().unwrap().ends_with(RECOVERY_CONTENT));
        body["tools"][3]["function"]["parameters"]["properties"]["criteria"]["items"]["properties"]
            ["evidence_call_ids"]["items"]["enum"] = json!([RECOVERY_WRITE]);
        assert!(recovery_reply(5, &body, true).is_err(), "historical write cannot replace current fresh-read evidence");
    }

    #[tokio::test]
    async fn success_recovery_holds_model_only_tail_and_never_reproposes_the_write() {
        use futures_util::StreamExt;
        let fixture = Arc::new(Fixture {
            calls: AtomicUsize::new(0), creative_failure: false,
            creative_submit_failure: false, creative_retry_ack_loss: false,
            shutdown_wait: false, success_recovery: true,
            recovery_armed: AtomicBool::new(false), recovery_wait_observed: AtomicBool::new(false),
            crash_tree: false, process_timeout_ms: 30000,
            tree_script: std::sync::OnceLock::new(), waiting_streams: AtomicUsize::new(0),
            finish: Semaphore::new(0), stop: CancellationToken::new(),
        });
        let mut body = recovery_test_body();
        assert_eq!(arm_success_recovery(State(fixture.clone())).await.0, axum::http::StatusCode::CONFLICT);
        let first = model(State(fixture.clone()), Json(body.clone())).await;
        let bytes = axum::body::to_bytes(first.into_body(), 65536).await.unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains(RECOVERY_WRITE) && text.contains("write_file") && text.contains("[DONE]"));
        recovery_test_add_receipt(&mut body, RECOVERY_WRITE);
        let held = model(State(fixture.clone()), Json(body.clone())).await;
        let mut stream = held.into_body().into_data_stream();
        let comment = stream.next().await.unwrap().unwrap();
        assert!(std::str::from_utf8(&comment).unwrap().starts_with(": SUCCESS_RECOVERY_WAIT_FOR_GUI_FAULT"));
        assert_eq!(fixture.waiting_streams.load(Ordering::SeqCst), 1);
        fixture.finish.add_permits(1);
        assert!(std::str::from_utf8(&stream.next().await.unwrap().unwrap()).unwrap().starts_with(':'));
        assert_eq!(arm_success_recovery(State(fixture.clone())).await.0, axum::http::StatusCode::CONFLICT,
            "arming must not release or complete the still-live original stream");
        drop(stream);
        assert_eq!(fixture.waiting_streams.load(Ordering::SeqCst), 0);
        assert_eq!(arm_success_recovery(State(fixture.clone())).await.0, axum::http::StatusCode::OK);
        assert!(recovery_reply(2, &json!({"messages":[],"tools":body["tools"]}), true).is_err(),
            "recovery requires the original write result; it must never fall back to writing again");
        let expected = [(RECOVERY_REPLAN,"update_plan"),(RECOVERY_READ,"read_file"),
            (RECOVERY_CLOSE,"update_plan"),(RECOVERY_REPORT,"report_completion")];
        let mut ids = std::collections::BTreeSet::from([RECOVERY_WRITE.to_owned()]);
        for (id, name) in expected {
            let response = model(State(fixture.clone()), Json(body.clone())).await;
            assert_eq!(response.status(), 200);
            let bytes = axum::body::to_bytes(response.into_body(), 65536).await.unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("[DONE]") && !text.contains("write_file"));
            let delta = text.lines().filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .find(|frame| frame["choices"][0]["delta"]["tool_calls"].is_array()).unwrap();
            let call = &delta["choices"][0]["delta"]["tool_calls"][0];
            assert_eq!(call["id"], id); assert_eq!(call["function"]["name"], name);
            assert!(ids.insert(id.to_owned()), "all resumed tool IDs must be fresh");
            let arguments: Value = serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
            if id == RECOVERY_READ { assert_eq!(arguments["path"], RECOVERY_FILE); }
            if id == RECOVERY_REPORT { assert_eq!(arguments["criteria"][0]["evidence_call_ids"], json!([RECOVERY_READ])); }
            recovery_test_add_receipt(&mut body, id);
        }
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 6);
        assert_eq!(model(State(fixture), Json(body)).await.status(), 400,
            "an unexpected post-report request must stop the fixture, never recreate the file");
    }

    #[tokio::test]
    async fn active_quit_uses_one_owned_exec_without_a_waiting_model_stream() {
        let fixture = Arc::new(Fixture {
            calls: AtomicUsize::new(0), creative_failure: false,
            creative_submit_failure: false, creative_retry_ack_loss: false,
            shutdown_wait: true, crash_tree: true, process_timeout_ms: 600000,
            success_recovery: false, recovery_armed: AtomicBool::new(false), recovery_wait_observed: AtomicBool::new(false),
            tree_script: std::sync::OnceLock::new(), waiting_streams: AtomicUsize::new(0),
            finish: Semaphore::new(0), stop: CancellationToken::new(),
        });
        fixture.tree_script.set(PathBuf::from("isolated-tree.mjs")).unwrap();
        let response = model(State(fixture.clone()), Json(json!({"messages":[]}))).await;
        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), 65536).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        let frames: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str(line).ok()).collect();
        let function = &frames.iter().find(|frame| frame["choices"][0]["delta"]["tool_calls"].is_array()).unwrap()
            ["choices"][0]["delta"]["tool_calls"][0]["function"];
        assert_eq!(function["name"], "exec_command");
        let arguments: Value = serde_json::from_str(function["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments["timeout_ms"], 600000);
        assert!(arguments.get("wait_ms").is_none());
        assert!(text.contains("[DONE]"));
        assert_eq!(fixture.waiting_streams.load(Ordering::SeqCst), 0);
        assert_eq!(model(State(fixture), Json(json!({"messages":[]}))).await.status(), 400,
            "a completed exec must not be replaced or restarted by the fixture");
    }

    #[tokio::test]
    async fn creative_submit_failure_is_one_http_rejection_not_a_success_stream() {
        let fixture = Arc::new(Fixture {
            calls: AtomicUsize::new(0), creative_failure: false, creative_submit_failure: true, creative_retry_ack_loss: false,
            shutdown_wait: false, crash_tree: false, process_timeout_ms: 30000, tree_script: std::sync::OnceLock::new(), waiting_streams: AtomicUsize::new(0),
            success_recovery: false, recovery_armed: AtomicBool::new(false), recovery_wait_observed: AtomicBool::new(false),
            finish: Semaphore::new(0), stop: CancellationToken::new(),
        });
        let response = model(State(fixture.clone()), Json(json!({"messages":[]}))).await;
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
        assert!(!fixture.creative_failure);
        assert_eq!(fixture.waiting_streams.load(Ordering::SeqCst), 0);
    }
}

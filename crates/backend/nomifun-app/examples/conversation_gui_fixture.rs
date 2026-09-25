//! Deterministic acceptance through the real desktop, Runtime, tools and history.
//! cargo run -p nomifun-app --example conversation_gui_fixture -- <new-data-dir>
//! Launch NomiFun with that NOMIFUN_DATA_DIR; send a normal request, inspect the
//! live journal, POST /finish to release the final response, then reload. Send
//! "格式异常" in a second turn to exercise split pseudo-tool-call rejection.
use axum::{Json, Router, extract::State, routing::{get, post}};
use nomifun_app::{DesktopHostServices, DesktopServer};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

struct Fixture {
    calls: AtomicUsize,
    finish: Semaphore,
    stop: CancellationToken,
}

fn frame(delta: Value, finish: Option<&str>) -> String {
    format!("data: {}\n\n", json!({
        "id":"journal-gui", "object":"chat.completion.chunk", "created":1,
        "model":"journal-fixture", "choices":[{"index":0,"delta":delta,"finish_reason":finish}]
    }))
}

async fn model(State(fixture): State<Arc<Fixture>>, Json(body): Json<Value>) -> axum::response::Response {
    fixture.calls.fetch_add(1, Ordering::SeqCst);
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let malformed = messages.iter().rev().find(|message| message["role"] == "user")
        .is_some_and(|message| message["content"].to_string().contains("格式异常"));
    let has_tool = messages.iter().rev().take_while(|message| message["role"] != "user")
        .any(|message| message["role"] == "tool");
    let mut frames = if malformed {
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
    std::fs::create_dir(&root)?;
    let fixture = Arc::new(Fixture { calls: AtomicUsize::new(0), finish: Semaphore::new(0), stop: CancellationToken::new() });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let routes = Router::new().route("/v1/chat/completions", post(model))
        .route("/finish", post(|State(f): State<Arc<Fixture>>| async move { f.finish.add_permits(1); "released" }))
        .route("/status", get(|State(f): State<Arc<Fixture>>| async move { Json(json!({"calls":f.calls.load(Ordering::SeqCst)})) }))
        .route("/shutdown", post(|State(f): State<Arc<Fixture>>| async move { f.stop.cancel(); "stopped" }))
        .with_state(fixture.clone());
    let stop = fixture.stop.clone();
    tokio::spawn(async move { axum::serve(listener, routes).with_graceful_shutdown(stop.cancelled_owned()).await });
    let cli = nomifun_app::cli::Cli {
        host:"127.0.0.1".into(), port:0, data_dir:root.clone(), work_dir:None,
        app_version:env!("CARGO_PKG_VERSION").into(), local:true,
        log_dir:Some(root.join("logs")), log_level:Some("off".into()), command:None,
    };
    let (app, keep_alive) = DesktopServer::start_with_outcome(&cli,"",None,None,None,DesktopHostServices::default()).await?;
    let prepared = async {
        let provider = api(&app,"/api/providers",json!({"platform":"custom","name":"会话回归测试模型","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"journal-fixture","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider = provider["provider_id"].as_str().ok_or_else(||anyhow::anyhow!("provider missing"))?;
        let editor = api(&app,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"会话内容回归","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"journal-fixture"}})).await?;
        let preset = editor["preset"]["preset_id"].as_str().ok_or_else(||anyhow::anyhow!("preset missing"))?;
        let mut draft = editor["draft"].clone();
        draft["document"]["enabled_capabilities"] = json!([{"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/write"]}]);
        api(&app,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],"draft":draft,"reason":"real desktop conversation regression fixture"})).await?;
        let session = api(&app,"/api/agent-sessions",json!({"preset_id":preset,"title":"会话内容回归 · 正常与异常","resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],"model":{"provider_id":provider,"model":"journal-fixture"}})).await?;
        Ok::<_,anyhow::Error>(session["agent_session_id"].clone())
    }.await;
    app.shutdown_all().await?;
    drop(app); drop(keep_alive);
    let session = prepared?;
    println!("CONVERSATION_GUI_FIXTURE_READY {}",json!({"data_dir":root,"control":format!("http://{address}"),"session_id":session}));
    fixture.stop.cancelled().await;
    Ok(())
}

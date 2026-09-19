//! Unified Runtime -> exact Browser Actions -> actual native WebView2.
//!
//! The local model is deterministic, but every Session, compiler, Kernel,
//! effect-ledger, Browser Resource and native interaction boundary is the
//! production path.

use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::{get, post},
};
use nomifun_browser_platform::{
    run_guard::BrowserInputState,
    runtime::{BrowserSurfaceBounds, BrowserTabCommand, WorkspaceError},
    workspace::BrowserResourceService,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tauri::{Listener, Manager};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const TEXT: &str = "Unified Runtime 原生输入";
const UPLOAD_TEXT: &str = "Unified Browser upload 中文 payload";
const DOWNLOAD_TEXT: &str = "Unified Browser download 中文 payload\n";
const STEP: &str = "Exercise the unified Browser Action path";
const REQUIREMENT: &str = "browser-flow";
const PROMPT: &str =
    "Use the Browser to open the local fixture, observe, type, click and verify the same native page.";
const HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>Unified Browser turn</title>
<style>body{font:18px system-ui;margin:36px}input,button{display:block;margin:16px 0;padding:12px;font:inherit}</style>
<h1>Unified Browser turn fixture</h1><label for="field">Input</label><input id="field"><button id="submit">Submit</button><output id="result"></output>
<label for="attachment">Attachment</label><input id="attachment" type="file"><a href="/download" target="_blank">Download local fixture</a>
<script>window.proof={nonce:crypto.randomUUID(),events:[]};for(const type of ['pointerdown','pointerup','mousedown','mouseup','click','beforeinput','input','change'])document.addEventListener(type,event=>proof.events.push({type,trusted:event.isTrusted,target:event.target.id}),true);document.getElementById('submit').onclick=()=>document.getElementById('result').textContent=document.getElementById('field').value;document.getElementById('attachment').onchange=event=>{window.uploadComplete=fetch('/upload',{method:'POST',body:event.target.files[0]}).then(response=>response.status);};</script>"#;

struct Model {
    url: String,
    calls: AtomicUsize,
    presented: Semaphore,
    terminal_entered: Semaphore,
    finish: Semaphore,
    response_tasks: tokio_util::task::TaskTracker,
    uploaded: Mutex<Vec<Vec<u8>>>,
    download_result: Mutex<Option<Value>>,
    stop: CancellationToken,
    failure: Mutex<Option<String>>,
}

fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}

fn latest_tool_value(body: &Value) -> Result<Value, String> {
    let content = body["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message["role"] == "tool")
        })
        .and_then(|message| message["content"].as_str())
        .ok_or("Missing Browser Action result")?;
    serde_json::from_str(content).map_err(error)
}

fn observed_element(body: &Value, name: &str) -> Result<Value, String> {
    let observation = latest_tool_value(body)?;
    observation["elements"]
        .as_array()
        .and_then(|elements| elements.iter().find(|element| element["name"] == name))
        .cloned()
        .ok_or_else(|| format!("Missing observed element for {name}: {observation}"))
}

fn browser_tool_name(body: &Value, action_id: &str) -> Result<String, String> {
    let marker = format!("Action: {action_id}.");
    body["tools"]
        .as_array()
        .and_then(|tools| {
            tools.iter().find(|tool| {
                tool["function"]["description"]
                    .as_str()
                    .is_some_and(|description| description.contains(&marker))
            })
        })
        .and_then(|tool| tool["function"]["name"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("Production model request omitted {action_id}"))
}

fn tool_call(name: String, step: usize, arguments: Value) -> Value {
    json!({
        "role":"assistant",
        "tool_calls":[{
            "index":0,
            "id":format!("native-step-{step}"),
            "type":"function",
            "function":{"name":name,"arguments":arguments.to_string()}
        }]
    })
}

async fn completion(
    State(model): State<Arc<Model>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let step = model.calls.fetch_add(1, Ordering::SeqCst);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let worker = model.clone();
    model.response_tasks.spawn(async move {
        let response = completion_body(worker, body, step).await;
        let _ = sender.send(response);
    });
    receiver
        .await
        .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn completion_body(
    model: Arc<Model>,
    body: Value,
    step: usize,
) -> axum::response::Response {
    if step == 1 {
        let prior = body["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "tool")
            })
            .and_then(|message| message["content"].as_str())
            .unwrap_or("missing adaptive-gate result");
        if !prior.contains("Call update_plan") {
            *model.failure.lock().unwrap() = Some(format!("Adaptive plan gate failed: {prior}"));
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if step == 2 {
        let prior = body["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "tool")
            })
            .and_then(|message| message["content"].as_str())
            .unwrap_or("missing plan result");
        if !prior.contains("Plan and source-anchored requirements recorded") {
            *model.failure.lock().unwrap() = Some(format!("Initial plan failed: {prior}"));
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    if step == 3 {
        match tokio::time::timeout(
            std::time::Duration::from_secs(10),
            model.presented.acquire(),
        )
        .await
        {
            Ok(Ok(permit)) => permit.forget(),
            _ => {
                let prior = body["messages"]
                    .as_array()
                    .and_then(|messages| {
                        messages
                            .iter()
                            .rev()
                            .find(|message| message["role"] == "tool")
                    })
                    .and_then(|message| message["content"].as_str())
                    .unwrap_or("missing tool result");
                *model.failure.lock().unwrap() = Some(format!(
                    "Native presentation acknowledgement missing; navigate result={prior}"
                ));
                return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }
    if step == 11 {
        match latest_tool_value(&body) {
            Ok(value) if value["download"]["path"].is_string() => {
                *model.download_result.lock().unwrap() = Some(value);
            }
            outcome => {
                *model.failure.lock().unwrap() =
                    Some(format!("Native download result is invalid: {outcome:?}"));
                return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    }
    if step == 14 {
        let prior = body["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "tool")
            })
            .and_then(|message| message["content"].as_str())
            .unwrap_or("missing completion report result");
        if !prior.contains("Completion account recorded") {
            *model.failure.lock().unwrap() =
                Some(format!("Completion report failed: {prior}"));
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let response = (|| -> Result<(Value, &'static str), String> {
        let delta = match step {
            0 => tool_call(
                browser_tool_name(&body, "browser/navigate")?,
                step,
                json!({"url":model.url}),
            ),
            1 => tool_call(
                "update_plan".into(),
                step,
                json!({
                    "explanation":"Exercise the exact Browser Action owner and native lifecycle.",
                    "requirements":[{
                        "id":REQUIREMENT,
                        "description":"Open and interact with the requested native Browser page.",
                        "source":{"input":0,"quote":"Use the Browser"}
                    }],
                    "plan":[{"step":STEP,"status":"in_progress"}]
                }),
            ),
            2 => tool_call(
                browser_tool_name(&body, "browser/navigate")?,
                step,
                json!({"url":model.url}),
            ),
            3 | 5 | 7 | 9 | 11 => tool_call(
                browser_tool_name(&body, "browser/observe")?,
                step,
                json!({}),
            ),
            4 => tool_call(
                browser_tool_name(&body, "browser/act")?,
                step,
                json!({"action":"type","element":observed_element(&body,"Input")?,"text":TEXT}),
            ),
            6 => tool_call(
                browser_tool_name(&body, "browser/act")?,
                step,
                json!({"action":"click","element":observed_element(&body,"Submit")?}),
            ),
            8 => tool_call(
                browser_tool_name(&body, "browser/upload")?,
                step,
                json!({"element":observed_element(&body,"Attachment")?,"files":["upload.txt"]}),
            ),
            10 => tool_call(
                browser_tool_name(&body, "browser/download")?,
                step,
                json!({"element":observed_element(&body,"Download local fixture")?}),
            ),
            12 => tool_call(
                "update_plan".into(),
                step,
                json!({
                    "explanation":"The native Browser actions completed and were freshly observed.",
                    "plan":[{"step":STEP,"status":"completed"}]
                }),
            ),
            13 => tool_call(
                "report_completion".into(),
                step,
                json!({
                    "summary":"The unified Browser Action path completed on the native page.",
                    "criteria":[{
                        "step":STEP,
                        "disposition":"supported",
                        "evidence_call_ids":["native-step-11"],
                        "rationale":"Navigation, fresh observations and trusted native input all returned successful owner results.",
                        "requirement_ids":[REQUIREMENT]
                    }]
                }),
            ),
            14 => {
                model.terminal_entered.add_permits(1);
                return Ok((
                    json!({"role":"assistant","content":"UNIFIED_BROWSER_DONE"}),
                    "stop",
                ));
            }
            _ => return Err(format!("Unexpected extra model request {step}")),
        };
        Ok((delta, "tool_calls"))
    })();
    let (delta, finish_reason) = match response {
        Ok(value) => value,
        Err(reason) => {
            *model.failure.lock().unwrap() = Some(reason.clone());
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"error":{"message":reason,"type":"fixture_error"}})),
            )
                .into_response();
        }
    };
    if step == 14 {
        tokio::select! {
            _ = model.stop.cancelled() => {
                return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            permit = model.finish.acquire() => {
                if let Ok(permit) = permit { permit.forget(); }
            }
        }
    }
    let chunk = |delta: Value, finish: Option<&str>| {
        json!({
            "id":format!("native-completion-{step}"),
            "object":"chat.completion.chunk",
            "created":1,
            "model":"native-fixture",
            "choices":[{"index":0,"delta":delta,"finish_reason":finish}]
        })
        .to_string()
    };
    (
        [("content-type", "text/event-stream")],
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            chunk(delta, None),
            chunk(json!({}), Some(finish_reason))
        ),
    )
        .into_response()
}

async fn uploaded_body(
    State(model): State<Arc<Model>>,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    model.uploaded.lock().unwrap().push(body.to_vec());
    axum::http::StatusCode::NO_CONTENT
}

async fn download_body() -> impl IntoResponse {
    (
        [
            ("content-type", "text/plain; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"unified-download.txt\"",
            ),
        ],
        DOWNLOAD_TEXT,
    )
}

async fn api(
    router: &nomifun_app::DesktopServer,
    method: &str,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(error)?
        .request(
            method.parse().map_err(error)?,
            format!("http://127.0.0.1:{}{path}", router.loopback_port()),
        )
        .header("x-nomi-local-trust", router.local_trust_secret())
        .json(&body)
        .send()
        .await
        .map_err(error)?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(error)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(error)?;
    if !status.is_success() {
        return Err(format!("{method} {path}: {status} {value}"));
    }
    Ok(value["data"].clone())
}

async fn wait_user_ready(
    router: &nomifun_app::DesktopServer,
    id: &str,
    workspace: &nomifun_browser_platform::workspace::BrowserResource,
    view: &tauri::Webview,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if workspace.snapshot().await.map_err(error)?.run.input_state
            == BrowserInputState::UserReady
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Unified Browser turn did not restore native input".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let projection = api(
        router,
        "GET",
        &format!("/api/agent-sessions/{id}/projection"),
        Value::Null,
    )
    .await?;
    if projection["status"] != "finished" || native_state(view).await? != (true, true) {
        return Err("Session terminal and native input unlock did not agree".into());
    }
    Ok(())
}

pub(super) async fn native_state(view: &tauri::Webview) -> Result<(bool, bool), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<(bool, bool)> {
            let mut hwnd = windows::Win32::Foundation::HWND::default();
            let mut visible = windows::core::BOOL::default();
            unsafe {
                platform.controller().ParentWindow(&mut hwnd)?;
                platform.controller().IsVisible(&mut visible)?;
                Ok((
                    windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled(hwnd).as_bool(),
                    visible.as_bool()
                        && windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool(),
                ))
            }
        })()
        .map_err(error);
        let _ = tx.send(result);
    })
    .map_err(error)?;
    rx.await.map_err(error)?
}

pub(super) fn run(app: tauri::AppHandle) -> Result<Value, String> {
    let root = tempfile::Builder::new()
        .prefix("nomifun-unified-browser-")
        .tempdir()
        .map_err(error)?;
    nomifun_runtime::init(&root.path().join("data"));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .map_err(error)?;
    let result = runtime.block_on(verify(&app, &root));
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    let cleanup = super::cleanup_fixture_profile(&root);
    match (result, cleanup) {
        (Ok(mut evidence), Ok(())) => {
            evidence["isolated_backend_cleanup"] = json!(true);
            Ok(evidence)
        }
        (result, cleanup) => Err(format!(
            "Unified Browser chain={result:?}; temporary cleanup={cleanup:?}; root={}",
            root.path().display()
        )),
    }
}

async fn verify(app: &tauri::AppHandle, root: &tempfile::TempDir) -> Result<Value, String> {
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).map_err(error)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(error)?;
    let address = listener.local_addr().map_err(error)?;
    let model = Arc::new(Model {
        url: format!("http://{address}/fixture"),
        calls: AtomicUsize::new(0),
        presented: Semaphore::new(0),
        terminal_entered: Semaphore::new(0),
        finish: Semaphore::new(0),
        response_tasks: tokio_util::task::TaskTracker::new(),
        uploaded: Mutex::new(Vec::new()),
        download_result: Mutex::new(None),
        stop: CancellationToken::new(),
        failure: Mutex::new(None),
    });
    let server_stop = model.stop.clone();
    let router = Router::new()
        .route("/v1/chat/completions", post(completion))
        .route("/fixture", get(|| async { axum::response::Html(HTML) }))
        .route("/upload", post(uploaded_body))
        .route("/download", get(download_body))
        .with_state(model.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
    });
    let browser_resources = Arc::new(BrowserResourceService::new(Arc::new(
        super::host::DesktopBrowserHost::new(app.clone()),
    )));
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".into(),
        port: 0,
        data_dir: root.path().join("data"),
        work_dir: Some(work.clone()),
        app_version: env!("CARGO_PKG_VERSION").into(),
        local: true,
        log_dir: Some(root.path().join("logs")),
        log_level: Some("off".into()),
        command: None,
    };
    let (router, keep_alive) = nomifun_app::DesktopServer::start_with_outcome(
        &cli,
        "",
        None,
        None,
        None,
        nomifun_app::DesktopHostServices {
            browser_resources: Some(browser_resources),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| "Isolated application startup failed")?;
    let presentations = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed_presentations = presentations.clone();
    let presentation_model = model.clone();
    let backend = Arc::downgrade(&router);
    let event = app
        .get_window("main")
        .ok_or("No native main window")?
        .listen("browser-workspace-open", move |event| {
            if let Ok(id) = serde_json::from_str::<String>(event.payload()) {
                observed_presentations.lock().unwrap().push(id.clone());
                let backend = backend.clone();
                let model = presentation_model.clone();
                tauri::async_runtime::spawn(async move {
                    let shown = async {
                        let backend = backend.upgrade().ok_or("Backend closed")?;
                        let workspace = backend
                            .browser_resource_for_local_surface(&id)
                            .await
                            .map_err(error)?;
                        workspace
                            .set_surface(
                                BrowserSurfaceBounds {
                                    x: 20.0,
                                    y: 60.0,
                                    width: 1060.0,
                                    height: 640.0,
                                },
                                true,
                                CancellationToken::new(),
                            )
                            .await
                            .map_err(error)
                    }
                    .await;
                    if let Err(reason) = shown {
                        *model.failure.lock().unwrap() = Some(reason);
                    }
                    model.presented.add_permits(1);
                });
            }
        });
    let result = async {
        let provider=api(&router,"POST","/api/providers",json!({"platform":"custom","name":"Unified Browser fixture","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"native-fixture","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider=provider["provider_id"].as_str().ok_or("Provider id missing")?;
        let editor=api(&router,"POST","/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Unified Browser fixture","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"native-fixture"}})).await?;
        let preset=editor["preset"]["preset_id"].as_str().ok_or("Preset id missing")?;
        let mut draft=editor["draft"].clone();
        let catalog=api(&router,"GET","/api/capabilities",Value::Null).await?;
        let browser=catalog.as_array().and_then(|items|items.iter().find(|item|item["capability"]["id"]=="browser")).ok_or("Browser Module missing")?;
        if browser["materialization_state"]!="materialized" {return Err(format!("Browser Module not materialized: {browser}"));}
        draft["document"]["enabled_capabilities"]=json!([{"capability":browser["capability"],"action_allowlist":["browser/navigate","browser/observe","browser/act","browser/upload","browser/download"]}]);
        draft["document"]["skill_bindings"]=json!([]);
        api(&router,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"unified Browser native integration"})).await?;
        let session=api(&router,"POST","/api/agent-sessions",json!({"preset_id":preset,"title":"Unified Browser fixture","model":{"provider_id":provider,"model":"native-fixture"},"resource_selections":[{"resource_kind":"browser","resource_id":"managed-browser"}]})).await?;
        let id=session["agent_session_id"].as_str().ok_or("Session id missing")?;
        let session_work=work.join("agent-sessions").join(id);
        std::fs::create_dir_all(&session_work).map_err(error)?;
        std::fs::write(session_work.join("upload.txt"),UPLOAD_TEXT).map_err(error)?;
        api(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"input":{"content":PROMPT},"idempotency_key":uuid::Uuid::now_v7().to_string()})).await?;
        let terminal_deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(60);
        loop {
            if let Ok(permit)=model.terminal_entered.try_acquire(){permit.forget();break;}
            if let Some(reason)=model.failure.lock().unwrap().clone(){return Err(reason);}
            if tokio::time::Instant::now()>=terminal_deadline{return Err(format!("Unified model did not reach terminal; calls={}",model.calls.load(Ordering::SeqCst)));}
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let workspace=router.browser_resource_for_local_surface(id).await.map_err(error)?;
        let snapshot=workspace.snapshot().await.map_err(error)?;
        if snapshot.run.input_state!=BrowserInputState::AgentRunning {return Err("Browser unlocked before terminal cleanup".into());}
        let runtime=snapshot.runtime.ok_or("Browser Action did not create a native runtime")?;
        if runtime.tabs.len()!=1{return Err(format!("Expected one native tab, got {}",runtime.tabs.len()));}
        let target=runtime.tabs[0].target.clone();
        let view=app.get_webview(&target.tab_id).ok_or("Browser Action did not own a native child")?;
        if native_state(&view).await?!=(false,true){return Err("Native child was not visible and input-locked".into());}
        if presentations.lock().unwrap().as_slice()!=[id]{return Err(format!("Unexpected Browser presentation events: {:?}",presentations.lock().unwrap()));}
        let proof=super::evaluate(&view,"({value:document.getElementById('field').value,result:document.getElementById('result').textContent,proof:window.proof})").await?;
        if proof["value"]!=TEXT || proof["result"]!=TEXT || !proof["proof"]["events"].as_array().is_some_and(|events|events.iter().all(|event|event["trusted"]==true) && events.iter().any(|event|event["type"]=="click" && event["target"]=="submit")){return Err(format!("Native Browser input proof failed: {proof}"));}
        if !matches!(workspace.user_command(BrowserTabCommand::Reload{target:target.clone()}).await,Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::UserInputLocked))){return Err("User command entered during the Agent Browser run".into());}
        let uploaded=super::evaluate(&view,"(async()=>({status:await window.uploadComplete,name:document.getElementById('attachment').files[0]?.name,text:await document.getElementById('attachment').files[0]?.text()}))()").await?;
        if uploaded["status"]!=204 || uploaded["name"]!="upload.txt" || uploaded["text"]!=UPLOAD_TEXT || model.uploaded.lock().unwrap().as_slice()!=[UPLOAD_TEXT.as_bytes().to_vec()]{return Err(format!("Native upload proof failed: {uploaded}"));}
        let download=model.download_result.lock().unwrap().clone().ok_or("Missing native download result")?;
        let relative=download["download"]["path"].as_str().ok_or("Missing native download publication path")?;
        let downloaded=std::fs::read(session_work.join(relative)).map_err(error)?;
        if downloaded!=DOWNLOAD_TEXT.as_bytes() || download["download"]["bytes"]!=downloaded.len() || download["download"]["sha256"]!=format!("{:x}",Sha256::digest(&downloaded)){return Err(format!("Native download publication proof failed: {download}"));}
        model.finish.add_permits(1);
        wait_user_ready(&router,id,&workspace,&view).await?;
        let events=api(&router,"GET",&format!("/api/agent-sessions/{id}/events?after_seq=0&limit=1000"),Value::Null).await?;
        if !events["events"].as_array().is_some_and(|events|events.iter().any(|event|event["kind"]=="turn/completed")){return Err("Unified Browser turn did not publish turn/completed".into());}
        if model.calls.load(Ordering::SeqCst)!=15{return Err(format!("Unified Browser model made an unexpected number of calls: {}",model.calls.load(Ordering::SeqCst)));}
        let messages=api(&router,"GET",&format!("/api/agent-sessions/{id}/messages?after_seq=0&limit=100"),Value::Null).await?;
        if !messages.to_string().contains("UNIFIED_BROWSER_DONE"){return Err("Canonical history omitted the final reply".into());}
        if super::evaluate(&view,"document.getElementById('result').textContent").await?!=TEXT{return Err("Terminal cleanup replaced the native page".into());}
        router.shutdown_all().await.map_err(error)?;
        if app.get_webview(&target.tab_id).is_some(){return Err("Application shutdown retained the native Browser child".into());}
        Ok(json!({
            "model":"local_scripted_protocol",
            "model_calls":model.calls.load(Ordering::SeqCst),
            "selected_module":"browser",
            "selected_actions":["browser/navigate","browser/observe","browser/act","browser/upload","browser/download"],
            "unified_action_tools":true,
            "adaptive_plan_and_completion":true,
            "native_target":target,
            "auto_open_once":true,
            "native_input_locked_until_cleanup":true,
            "trusted_native_input":true,
            "native_upload_snapshot":true,
            "native_download_publication":true,
            "target_blank_download_cleanup":true,
            "same_page_after_terminal":true,
            "canonical_history":true,
            "active_resource_shutdown":true
        }))
    }.await;
    app.unlisten(event);
    model.stop.cancel();
    model.finish.add_permits(1);
    let shutdown=router.shutdown_all().await.map_err(error);
    drop(router);
    drop(keep_alive);
    server.await.map_err(error)?.map_err(error)?;
    model.response_tasks.close();
    tokio::time::timeout(std::time::Duration::from_secs(5),model.response_tasks.wait()).await.map_err(|_|"Model response worker did not stop")?;
    match (result,shutdown){
        (Ok(evidence),Ok(()))=>Ok(evidence),
        (result,shutdown)=>Err(format!("Unified Browser chain={result:?}; shutdown={shutdown:?}")),
    }
}

//! Product API → capability snapshot → Nomi loop → actual native WebView.
//! The model is a deterministic local OpenAI-protocol fixture, not a live LLM.
use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::{get, post},
};
use nomifun_browser_platform::{
    run_guard::BrowserInputState, runtime::*, workspace::BrowserWorkspaceService,
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tauri::{Listener, Manager};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
const TEXT: &str = "Agent 原生输入";
const RESUMED_TEXT: &str = "停止后新一轮输入";
const UPLOAD_TEXT: &str = "Native upload 中文 payload";
const LATER_UPLOAD_TEXT: &str = "Read only after the Agent stops";
const AUTO_CLEAR_TEXT: &str = "Consumed by the application immediately";
const HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>Agent native turn</title>
<style>body{font:18px system-ui;margin:36px}input,button{display:block;margin:16px 0;padding:12px;font:inherit}</style>
<h1>Native Agent turn fixture</h1><label for="field">Input</label><input id="field"><button id="submit">Submit</button><output id="result"></output>
<form id="fileform"><label for="attachment">Attachment</label><input id="attachment" type="file" multiple><button type="reset">Clear attachment</button></form>
<button id="customAttachment">Custom attachment</button>
<button id="waitingAttachment">Wait for attachment</button>
<script>window.proof={nonce:crypto.randomUUID(),events:[]};for(const type of ['pointerdown','pointerup','mousedown','mouseup','click','beforeinput','input','change'])document.addEventListener(type,e=>proof.events.push({type,trusted:e.isTrusted,target:e.target.id}),true);
document.getElementById('attachment').onchange=event=>{if(!event.target.files.length){window.uploadCleared=true;return;}if(event.target.files[1])window.savedFile=event.target.files[1];window.lastSelectedFile=event.target.files[0];window.uploadComplete=fetch('/upload',{method:'POST',body:window.lastSelectedFile}).then(response=>response.status);if(window.autoClearNext)event.target.value='';};
document.getElementById('fileform').onreset=event=>{window.uploadCleared=event.isTrusted;window.autoClearNext=true};
document.getElementById('customAttachment').onclick=event=>{window.customClickTrusted=event.isTrusted;window.customClicks=(window.customClicks||0)+1;const input=document.createElement('input');input.type='file';input.hidden=true;input.onchange=chosen=>{document.getElementById('attachment').onchange(chosen);input.remove();window.customInputRemoved=!input.isConnected;};document.body.appendChild(input);input.click();};
document.getElementById('waitingAttachment').onclick=()=>fetch('/chooser-waiting',{method:'POST'});
document.getElementById('submit').onclick=()=>{document.getElementById('result').textContent=document.getElementById('field').value;console.error('NATIVE_AGENT_DIAGNOSTIC')};</script>"#;

struct Model {
    url: String,
    calls: AtomicUsize,
    presented: Semaphore,
    terminal_entered: Semaphore,
    finish: Semaphore,
    late_entered: Semaphore,
    late_release: Semaphore,
    late_sent: Semaphore,
    slow_entered: Semaphore,
    slow_release: Semaphore,
    previous_reference: Mutex<Option<Value>>,
    late_discarded: AtomicBool,
    response_tasks: tokio_util::task::TaskTracker,
    shutdown_entered: Semaphore,
    shutdown_release: Semaphore,
    shutdown_disconnected: Semaphore,
    uploaded: Mutex<Vec<Vec<u8>>>,
    chooser_waiting: Semaphore,
    stop: CancellationToken,
    failure: Mutex<Option<String>>,
}
struct ResponseWaiter {
    model: Arc<Model>,
    step: usize,
    received: bool,
}
impl Drop for ResponseWaiter {
    fn drop(&mut self) {
        if self.step == 27 && !self.received {
            self.model.shutdown_disconnected.add_permits(1);
        }
    }
}
fn error(value: impl std::fmt::Display) -> String {
    value.to_string()
}
fn reference(body: &Value, name: &str) -> Result<Value, String> {
    let content = body["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message["role"] == "tool")
        })
        .and_then(|message| message["content"].as_str())
        .ok_or("Missing Browser tool result")?;
    let observation: Value = serde_json::from_str(content).map_err(error)?;
    observation["elements"]
        .as_array()
        .and_then(|elements| elements.iter().find(|element| element["name"] == name))
        .map(|element| element["reference"].clone())
        .ok_or_else(|| format!("Missing observed reference for {name}: {content}"))
}
async fn completion(
    State(model): State<Arc<Model>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let step = model.calls.fetch_add(1, Ordering::SeqCst);
    // A real remote model may finish generating after the client disconnects.
    // Keep generation owned by this fixture, not the HTTP response future.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let worker = model.clone();
    model.response_tasks.spawn(async move {
        let response = completion_body(worker.clone(), body, step).await;
        let discarded = sender.send(response).is_err();
        if step == 8 {
            worker.late_discarded.store(discarded, Ordering::SeqCst);
        }
    });
    let mut waiter = ResponseWaiter {
        model,
        step,
        received: false,
    };
    let response = receiver
        .await
        .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response());
    waiter.received = true;
    response
}
async fn completion_body(model: Arc<Model>, body: Value, step: usize) -> axum::response::Response {
    if step == 1 {
        // The real native child must be shown in response to the host event
        // before the model proceeds to observation and input.
        let shown =
            tokio::time::timeout(std::time::Duration::from_secs(5), model.presented.acquire())
                .await;
        if let Ok(Ok(permit)) = shown {
            permit.forget();
        } else {
            *model.failure.lock().unwrap() =
                Some("Native presentation acknowledgement missing".into());
        }
        if model.failure.lock().unwrap().is_some() {
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    let tool = body["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "Browser")
    });
    let operation = (|| -> Result<Option<Value>, String> {
        if !tool {
            return Err("Production model request omitted the selected Browser tool".into());
        }
        let next = match step {
            0 => json!({"operation":"navigate","url":model.url}),
            1 | 3 | 7 | 9 | 11 | 15 | 18 | 21 | 24 | 26 => json!({"operation":"observe"}),
            2 => {
                json!({"operation":"act","action":{"action":"type","element":reference(&body,"Input")?,"text":TEXT}})
            }
            4 => {
                json!({"operation":"act","action":{"action":"click","element":reference(&body,"Submit")?}})
            }
            5 => json!({"operation":"diagnostics"}),
            6 => {
                let diagnostic = body["messages"]
                    .as_array()
                    .and_then(|messages| {
                        messages
                            .iter()
                            .rev()
                            .find(|message| message["role"] == "tool")
                    })
                    .and_then(|message| message["content"].as_str())
                    .ok_or("Missing diagnostics tool result")?;
                if !diagnostic.contains("NATIVE_AGENT_DIAGNOSTIC") {
                    return Err(format!("Actual page diagnostic not returned: {diagnostic}"));
                }
                return Ok(None);
            }
            8 => {
                let previous = reference(&body, "Submit")?;
                *model.previous_reference.lock().unwrap() = Some(previous.clone());
                json!({"operation":"act","action":{"action":"click","element":previous}})
            }
            10 => {
                let fresh = reference(&body, "Input")?;
                let previous = model.previous_reference.lock().unwrap();
                let previous = previous.as_ref().ok_or("Missing prior-turn reference")?;
                if fresh["target"] != previous["target"]
                    || fresh["observation_generation"] == previous["observation_generation"]
                {
                    return Err("New turn did not freshly observe the same native document".into());
                }
                json!({"operation":"act","action":{"action":"type","element":fresh,"text":RESUMED_TEXT}})
            }
            12 => {
                json!({"operation":"act","action":{"action":"click","element":reference(&body,"Submit")?}})
            }
            13 => return Ok(None),
            14 => json!({"operation":"navigate","url":model.url.replace("/fixture","/slow")}),
            16 => {
                json!({"operation":"upload","element":reference(&body,"Attachment")?,"files":["upload.txt","later.txt"]})
            }
            17 | 20 | 23 => {
                let result = body["messages"]
                    .as_array()
                    .and_then(|messages| {
                        messages
                            .iter()
                            .rev()
                            .find(|message| message["role"] == "tool")
                    })
                    .and_then(|message| message["content"].as_str())
                    .ok_or("Missing upload result")?;
                let result: Value = serde_json::from_str(result)
                    .map_err(|error| format!("Upload result was not JSON: {error}; {result}"))?;
                let expected = if step != 20 {
                    "browser_protocol"
                } else {
                    "browser_input"
                };
                if result["interaction_fidelity"] != expected {
                    return Err(format!("Incorrect file interaction fidelity: {result}"));
                }
                return Ok(None);
            }
            19 => {
                json!({"operation":"act","action":{"action":"click","element":reference(&body,"Clear attachment")?}})
            }
            22 => {
                json!({"operation":"upload","element":reference(&body,"Custom attachment")?,"files":["upload.txt"]})
            }
            25 => {
                json!({"operation":"upload","element":reference(&body,"Wait for attachment")?,"files":["upload.txt"]})
            }
            27 => return Ok(None),
            _ => return Err(format!("Unexpected extra model request {step}")),
        };
        Ok(Some(next))
    })();
    let operation = match operation {
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
    if step == 8 {
        model.late_entered.add_permits(1);
        tokio::select! {
            _=model.stop.cancelled()=>return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
            permit=model.late_release.acquire()=>{if let Ok(permit)=permit {permit.forget();}}
        }
        model.late_sent.add_permits(1);
    }
    let (delta, finish) = if let Some(input) = operation {
        (
            json!({"role":"assistant","tool_calls":[{"index":0,"id":format!("native-step-{step}"),"type":"function","function":{"name":"Browser","arguments":input.to_string()}}]}),
            "tool_calls",
        )
    } else {
        let finish = if step == 27 {
            model.shutdown_entered.add_permits(1);
            &model.shutdown_release
        } else {
            model.terminal_entered.add_permits(1);
            &model.finish
        };
        tokio::select! {
            _=model.stop.cancelled()=>return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
            permit=finish.acquire()=>{ if let Ok(permit)=permit {permit.forget();} }
        }
        (
            json!({"role":"assistant","content":if step==6 {"NATIVE_BROWSER_DONE"} else {"NATIVE_BROWSER_RESUMED"}}),
            "stop",
        )
    };
    let chunk = |delta: Value, finish: Option<&str>| {
        json!({"id":format!("native-completion-{step}"),"object":"chat.completion.chunk","created":1,"model":"native-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
    };
    (
        [("content-type", "text/event-stream")],
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            chunk(delta, None),
            chunk(json!({}), Some(finish))
        ),
    )
        .into_response()
}
async fn slow_page(State(model): State<Arc<Model>>) -> axum::response::Response {
    model.slow_entered.add_permits(1);
    tokio::select! {
        _=model.stop.cancelled()=>axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
        permit=model.slow_release.acquire()=>{
            if let Ok(permit)=permit {permit.forget();}
            axum::response::Html("<!doctype html><title>Late navigation</title>STOP_MUST_REJECT_THIS_LATE_PAGE").into_response()
        }
    }
}
async fn uploaded_body(
    State(model): State<Arc<Model>>,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    model.uploaded.lock().unwrap().push(body.to_vec());
    axum::http::StatusCode::NO_CONTENT
}
async fn chooser_waiting(State(model): State<Arc<Model>>) -> axum::http::StatusCode {
    model.chooser_waiting.add_permits(1);
    axum::http::StatusCode::NO_CONTENT
}

async fn permit(semaphore: &Semaphore, label: &str) -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(15), semaphore.acquire())
        .await
        .map_err(|_| format!("Timed out waiting for {label}"))?
        .map_err(error)?
        .forget();
    Ok(())
}
async fn page_proof(view: &tauri::Webview) -> Result<Value, String> {
    super::evaluate(view,"({value:document.getElementById('field').value,result:document.getElementById('result').textContent,proof:window.proof})").await
}
async fn start_turn(
    router: &nomifun_app::DesktopServer,
    id: &str,
    prompt: &str,
) -> Result<(), String> {
    api(
        router,
        "POST",
        &format!("/api/agent-sessions/{id}/turns"),
        json!({"input":{"content":prompt},"idempotency_key":uuid::Uuid::new_v4().to_string()}),
    )
    .await?;
    Ok(())
}
async fn user_ready(
    router: &nomifun_app::DesktopServer,
    id: &str,
    workspace: &nomifun_browser_platform::workspace::BrowserWorkspace,
    view: &tauri::Webview,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if workspace.snapshot().await.map_err(error)?.run.input_state
            == BrowserInputState::UserReady
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Agent did not settle and restore native input".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    if native_state(view).await? != (true, true) {
        return Err("Settled Agent did not leave a visible input-enabled page".into());
    }
    let conversation = api(
        router,
        "GET",
        &format!("/api/conversations/{id}"),
        json!(null),
    )
    .await?;
    if conversation["status"] != "finished" {
        return Err("Native input restored before conversation terminal".into());
    }
    Ok(())
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
        .timeout(std::time::Duration::from_secs(25))
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
        .prefix("nomifun-native-agent-")
        .tempdir()
        .map_err(error)?;
    nomifun_runtime::init(&root.path().join("data"));
    // Match the desktop shell: the backend has its own runtime, not Tauri's
    // process-global executor. Its drop also releases SQLite pool workers.
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
        (result, cleanup) => {
            let mut pending = vec![root.path().to_path_buf()];
            let mut held = vec![];
            while let Some(directory) = pending.pop() {
                let Ok(entries) = std::fs::read_dir(directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    if kind.is_dir() {
                        pending.push(entry.path());
                    } else if kind.is_file() {
                        use std::os::windows::fs::OpenOptionsExt;
                        if let Err(error) = std::fs::OpenOptions::new()
                            .read(true)
                            .share_mode(0)
                            .open(entry.path())
                        {
                            held.push(format!(
                                "{}: {error}",
                                entry.path().strip_prefix(root.path()).unwrap().display()
                            ));
                        }
                    }
                }
            }
            Err(format!(
                "Agent chain={result:?}; temporary cleanup={cleanup:?}; held={held:?}; root={}",
                root.path().display()
            ))
        }
    }
}

async fn verify(app: &tauri::AppHandle, root: &tempfile::TempDir) -> Result<Value, String> {
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).map_err(error)?;
    std::fs::write(work.join("upload.txt"), UPLOAD_TEXT).map_err(error)?;
    std::fs::write(work.join("later.txt"), LATER_UPLOAD_TEXT).map_err(error)?;
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
        late_entered: Semaphore::new(0),
        late_release: Semaphore::new(0),
        late_sent: Semaphore::new(0),
        slow_entered: Semaphore::new(0),
        slow_release: Semaphore::new(0),
        previous_reference: Mutex::new(None),
        late_discarded: AtomicBool::new(false),
        response_tasks: tokio_util::task::TaskTracker::new(),
        shutdown_entered: Semaphore::new(0),
        shutdown_release: Semaphore::new(0),
        shutdown_disconnected: Semaphore::new(0),
        uploaded: Mutex::new(vec![]),
        chooser_waiting: Semaphore::new(0),
        stop: CancellationToken::new(),
        failure: Mutex::new(None),
    });
    let server_stop = model.stop.clone();
    let router = Router::new()
        .route("/v1/chat/completions", post(completion))
        .route("/fixture", get(|| async { axum::response::Html(HTML) }))
        .route("/slow", get(slow_page))
        .route("/upload", post(uploaded_body))
        .route("/chooser-waiting", post(chooser_waiting))
        .with_state(model.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_stop.cancelled_owned())
            .await
    });
    let workspaces = Arc::new(BrowserWorkspaceService::new(Arc::new(
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
            browser_workspaces: Some(workspaces.clone()),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| "Isolated application startup failed")?;
    let presentation = Arc::new(Mutex::new(Vec::<String>::new()));
    let emitted = presentation.clone();
    let presentation_model = model.clone();
    let backend = Arc::downgrade(&router);
    let event = app
        .get_window("main")
        .ok_or("No native main window")?
        .listen("browser-workspace-open", move |event| {
            if let Ok(id) = serde_json::from_str::<String>(event.payload()) {
                emitted.lock().unwrap().push(id.clone());
                let backend = backend.clone();
                let model = presentation_model.clone();
                tauri::async_runtime::spawn(async move {
                    let shown = async {
                        let backend = backend
                            .upgrade()
                            .ok_or("Backend closed before presentation")?;
                        let workspace = backend
                            .browser_workspace_for_local_surface(&id)
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
                    if let Err(error) = shown {
                        *model.failure.lock().unwrap() = Some(error);
                    }
                    model.presented.add_permits(1);
                });
            }
        });
    let result=async {
        let provider=api(&router,"POST","/api/providers",json!({"platform":"custom","name":"Native Agent fixture","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"native-fixture","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider=provider["provider_id"].as_str().ok_or("Provider id missing")?;
        let editor=api(&router,"POST","/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Native Browser Agent fixture","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"native-fixture"}})).await?;
        let preset=editor["preset"]["preset_id"].as_str().ok_or("Preset id missing")?;
        let mut draft=editor["draft"].clone();
        let catalog=api(&router,"GET","/api/capabilities",json!(null)).await?;
        let mut selections=vec![];
        for id in ["browser.navigate","browser.observe","browser.act","browser.upload"] {
            let item=catalog.as_array().and_then(|items|items.iter().find(|item|item["capability"]["id"]==id)).ok_or("Browser capability missing")?;
            if item["materialization_state"]!="materialized" {return Err(format!("{id} not materialized: {item}"));}
            selections.push(json!({"capability":item["capability"],"action_allowlist":[]}));
        }
        draft["document"]["enabled_capabilities"]=json!(selections);
        draft["document"]["skill_bindings"]=json!([]);
        api(&router,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"native browser agent integration"})).await?;
        let session=api(&router,"POST","/api/agent-sessions",json!({"preset_id":preset,"title":"Native Browser Agent fixture","model":{"provider_id":provider,"model":"native-fixture"}})).await?;
        let id=session["agent_session_id"].as_str().ok_or("Session id missing")?;
        api(&router,"PATCH",&format!("/api/conversations/{id}"),json!({"extra":{"workspace":work.to_string_lossy()}})).await?;
        api(&router,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"input":{"content":"Use the Browser to open the local fixture, observe, type, click and read diagnostics."},"idempotency_key":uuid::Uuid::new_v4().to_string()})).await?;
        tokio::time::timeout(std::time::Duration::from_secs(40),model.terminal_entered.acquire()).await.map_err(|_|format!("Model did not reach terminal gate; calls={}, failure={:?}",model.calls.load(Ordering::SeqCst),model.failure.lock().unwrap()))?.map_err(error)?.forget();
        let workspace=router.browser_workspace_for_local_surface(id).await.map_err(error)?;
        let snapshot=workspace.snapshot().await.map_err(error)?;
        if snapshot.run.input_state!=BrowserInputState::AgentRunning { return Err("Browser unlocked before Agent terminal".into()); }
        let runtime=snapshot.runtime.ok_or("Agent did not create a native runtime")?;
        if runtime.tabs.len()!=1 {return Err("Expected exactly one native tab".into());}
        let target=runtime.tabs[0].target.clone();
        let view=app.get_webview(&target.tab_id).ok_or("Browser Tool did not target an actual native child")?;
        if native_state(&view).await? != (false,true) {return Err("Native child must be visible and input-locked during Agent turn".into());}
        let events=presentation.lock().unwrap().clone();
        if events!=[id.to_owned()] {return Err(format!("Expected one auto-open for this conversation: {events:?}"));}
        let proof=super::evaluate(&view,"({value:document.getElementById('field').value,result:document.getElementById('result').textContent,proof:window.proof})").await?;
        if proof["value"]!=TEXT || proof["result"]!=TEXT || !proof["proof"]["events"].as_array().is_some_and(|events|!events.is_empty() && events.iter().all(|event|event["trusted"]==true) && events.iter().any(|event|event["type"]=="click" && event["target"]=="submit")) { return Err(format!("Native input evidence failed: {proof}")); }
        for kind in ["beforeinput","input"] {
            if !proof["proof"]["events"].as_array().unwrap().iter().any(|event|event["type"]==kind && event["target"]=="field") { return Err(format!("Missing native {kind} event")); }
        }
        let rejected=workspace.user_command(BrowserTabCommand::Reload{target:target.clone()}).await;
        if !matches!(rejected,Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::UserInputLocked))) {return Err("User navigation admitted during Agent turn".into());}
        model.finish.add_permits(1);
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(15);
        loop {
            if workspace.snapshot().await.map_err(error)?.run.input_state==BrowserInputState::UserReady {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Agent did not restore native user input".into());}
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let conversation=api(&router,"GET",&format!("/api/conversations/{id}"),json!(null)).await?;
        if conversation["status"]!="finished" || native_state(&view).await? != (true,true) {return Err(format!("Missing terminal/native unlock: {}",conversation["status"]));}
        let messages=api(&router,"GET",&format!("/api/conversations/{id}/messages"),json!(null)).await?;
        if !messages.to_string().contains("NATIVE_BROWSER_DONE") {return Err("Normal conversation history omitted the final Agent reply".into());}
        let after=super::evaluate(&view,"({value:document.getElementById('field').value,result:document.getElementById('result').textContent,proof:window.proof})").await?;
        if after!=proof {return Err("Agent terminal changed or replaced its native page".into());}
        // Stop while the model has prepared an action but not delivered it.
        start_turn(&router,id,"Observe this page; wait for cancellation before the next click.").await?;
        permit(&model.late_entered,"held model response").await?;
        if native_state(&view).await?!=(false,true) {return Err("Waiting model left native input enabled".into());}
        api(&router,"POST",&format!("/api/conversations/{id}/cancel"),json!({})).await?;
        user_ready(&router,id,&workspace,&view).await?;
        if page_proof(&view).await?!=proof {return Err("Stopping the waiting model changed the page".into());}
        model.late_release.add_permits(1);
        permit(&model.late_sent,"late model action response").await?;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if page_proof(&view).await?!=proof || model.calls.load(Ordering::SeqCst)!=9 || !model.late_discarded.load(Ordering::SeqCst) {return Err("Cancelled model response was not disconnected or continued native work".into());}

        // A subsequent turn observes afresh and uses the very same document.
        start_turn(&router,id,"Observe the existing page afresh, change the input and submit.").await?;
        permit(&model.terminal_entered,"resumed turn terminal gate").await?;
        if native_state(&view).await?!=(false,true) {return Err("Resumed turn failed to lock native input".into());}
        let resumed=page_proof(&view).await?;
        if resumed["proof"]["nonce"]!=proof["proof"]["nonce"] || resumed["value"]!=RESUMED_TEXT || resumed["result"]!=RESUMED_TEXT {return Err(format!("Resumed turn lost its existing page: {resumed}"));}
        model.finish.add_permits(1);
        user_ready(&router,id,&workspace,&view).await?;
        if page_proof(&view).await?!=resumed {return Err("Resumed turn terminal altered the page".into());}

        // Stop an actual in-flight native navigation, with HTTP headers held.
        start_turn(&router,id,"Navigate to the slow local page; this turn will be stopped.").await?;
        permit(&model.slow_entered,"native navigation request").await?;
        if native_state(&view).await?!=(false,true) {return Err("Pending native navigation left input enabled".into());}
        api(&router,"POST",&format!("/api/conversations/{id}/cancel"),json!({})).await?;
        user_ready(&router,id,&workspace,&view).await?;
        let stopped=workspace.snapshot().await.map_err(error)?.runtime.ok_or("Stopped native runtime missing")?;
        let current=stopped.tabs.iter().find(|tab|tab.target.tab_id==target.tab_id).ok_or("Stop replaced native tab")?.target.clone();
        workspace.user_command(BrowserTabCommand::Navigate{target:current,url:model.url.clone()}).await.map_err(error)?;
        let ready_deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        let user_page=loop {
            if let Ok(proof)=page_proof(&view).await {if proof["proof"]["nonce"].is_string() && proof["proof"]["nonce"]!=resumed["proof"]["nonce"] {break proof;}}
            if tokio::time::Instant::now()>=ready_deadline {return Err("User navigation after Stop did not become usable".into());}
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        };
        model.slow_release.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if page_proof(&view).await?!=user_page || model.calls.load(Ordering::SeqCst)!=15 {return Err("Cancelled navigation or model continued after user input restored".into());}
        if model.failure.lock().unwrap().is_some() {return Err(format!("Model fixture failure: {:?}",model.failure.lock().unwrap()));}
        start_turn(&router,id,"Observe Attachment and upload the workspace file upload.txt.").await?;
        permit(&model.terminal_entered,"uploaded file terminal gate").await?;
        std::fs::write(work.join("upload.txt"),"changed after selection").map_err(error)?;
        std::fs::write(work.join("later.txt"),"also changed after selection").map_err(error)?;
        let selected=super::evaluate(&view,"(async()=>{const el=document.getElementById('attachment');return {name:el.files[0]?.name,text:await el.files[0]?.text(),events:proof.events.filter(event=>event.target==='attachment')}})()").await?;
        if selected["name"]!="upload.txt" || selected["text"]!=UPLOAD_TEXT {return Err(format!("Native upload did not retain the authorized file snapshot: {selected}"));}
        if super::evaluate(&view,"window.uploadComplete").await?!=204 || model.uploaded.lock().unwrap().as_slice()!=[UPLOAD_TEXT.as_bytes().to_vec()] {return Err("Native browser request did not deliver the selected file bytes".into());}
        for kind in ["input","change"] {
            if !selected["events"].as_array().is_some_and(|events|events.iter().any(|event|event["type"]==kind && event["trusted"]==true)) {return Err(format!("Missing native upload {kind}: {selected}"));}
        }
        model.finish.add_permits(1);
        user_ready(&router,id,&workspace,&view).await?;
        if super::evaluate(&view,"document.getElementById('attachment').files[0].text()").await?!=UPLOAD_TEXT {return Err("File snapshot disappeared when Agent stopped".into());}
        if super::evaluate(&view,"document.getElementById('attachment').files[1].text()").await?!=LATER_UPLOAD_TEXT {return Err("Unread selected file did not survive Agent terminal".into());}
        start_turn(&router,id,"Observe and click the page's Clear attachment button.").await?;
        permit(&model.terminal_entered,"cleared file input terminal gate").await.map_err(|error|format!("{error}; calls={}; model={:?}",model.calls.load(Ordering::SeqCst),model.failure.lock().unwrap()))?;
        if super::evaluate(&view,"document.getElementById('attachment').files.length===0 && window.uploadCleared===true").await?!=true {return Err("Native file input did not clear".into());}
        if super::evaluate(&view,"window.savedFile.text()").await?!=LATER_UPLOAD_TEXT {return Err("Clearing the input invalidated a File retained by the application".into());}
        model.finish.add_permits(1);
        user_ready(&router,id,&workspace,&view).await?;
        std::fs::write(work.join("upload.txt"),AUTO_CLEAR_TEXT).map_err(error)?;
        start_turn(&router,id,"Upload upload.txt through Custom attachment; it creates a hidden input and consumes the selection.").await?;
        permit(&model.terminal_entered,"custom chooser upload terminal gate").await.map_err(|error|format!("{error}; model={:?}",model.failure.lock().unwrap()))?;
        if super::evaluate(&view,"customClickTrusted===true && customClicks===1 && customInputRemoved===true").await?!=true {return Err("Custom upload did not use the real button and dynamically created input".into());}
        if super::evaluate(&view,"document.getElementById('attachment').files.length").await?!=0 || super::evaluate(&view,"window.lastSelectedFile.text()").await?!=AUTO_CLEAR_TEXT {return Err("Application-consumed upload was not delivered before its own reset".into());}
        if super::evaluate(&view,"window.uploadComplete").await?!=204 || model.uploaded.lock().unwrap().last()!=Some(&AUTO_CLEAR_TEXT.as_bytes().to_vec()) {return Err("Application-consumed upload did not reach the server".into());}
        model.finish.add_permits(1);
        user_ready(&router,id,&workspace,&view).await?;
        let delivered=model.uploaded.lock().unwrap().len();
        start_turn(&router,id,"Use Wait for attachment; stop while waiting for its file chooser.").await?;
        permit(&model.chooser_waiting,"custom chooser wait").await?;
        api(&router,"POST",&format!("/api/conversations/{id}/cancel"),json!({})).await?;
        user_ready(&router,id,&workspace,&view).await?;
        if model.uploaded.lock().unwrap().len()!=delivered || model.calls.load(Ordering::SeqCst)!=26 {return Err("Cancelled chooser transferred files or continued the turn".into());}
        start_turn(&router,id,"Observe this page and wait; the application will close during the reply.").await?;
        permit(&model.shutdown_entered,"active model at application shutdown").await?;
        if native_state(&view).await?!=(false,true) {return Err("Shutdown fixture did not start a locked native turn".into());}
        router.shutdown_all().await.map_err(error)?;
        permit(&model.shutdown_disconnected,"application-owned model transport shutdown").await?;
        if app.get_webview(&target.tab_id).is_some() {return Err("Application shutdown retained the native browser child".into());}
        model.shutdown_release.add_permits(1);
        Ok(json!({"model":"local_scripted_protocol","model_calls":model.calls.load(Ordering::SeqCst),"selected_capabilities":["browser.navigate","browser.observe","browser.act","browser.upload"],"native_target":target,"auto_open_once":true,"shown_before_input":true,"native_input_and_diagnostics":true,"terminal_before_unlock":true,"same_page_after_terminal":true,"normal_history_reply":true,"cancelled_model_reply_ignored":true,"next_turn_fresh_same_page":true,"cancelled_native_navigation":true,"user_navigation_after_stop":true,"native_upload_snapshot":true,"active_agent_shutdown":true}))
    }.await;
    app.unlisten(event);
    model.stop.cancel();
    model.finish.add_permits(1);
    let shutdown = router.shutdown_all().await.map_err(error);
    drop(router);
    drop(keep_alive);
    server.await.map_err(error)?.map_err(error)?;
    model.response_tasks.close();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        model.response_tasks.wait(),
    )
    .await
    .map_err(|_| "Model fixture response worker did not stop")?;
    match (result, shutdown) {
        (Ok(evidence), Ok(())) => Ok(evidence),
        (result, shutdown) => Err(format!("Agent chain={result:?}; shutdown={shutdown:?}")),
    }
}

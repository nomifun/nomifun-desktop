//! Opt-in real-model frontend loop. Only disposable fixture code and page
//! observations reach the configured test provider; credentials arrive via stdin.
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use nomifun_browser_platform::{
    run_guard::BrowserInputState, runtime::BrowserSurfaceBounds, workspace::BrowserWorkspaceService,
};
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Read},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{Listener, Manager};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const BROKEN_JS: &str = "function nextCount(value) { return value + 2; }\n";
const HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>Counter app</title>
<style>body{font:20px system-ui;padding:36px}button{font:inherit;padding:12px 24px;margin-top:20px}output{font-size:32px;display:block}</style>
<h1>Counter app</h1><label for="count">Count</label><output id="count">0</output><button id="increment">Increment</button>
<script src="/app.js"></script><script>
(()=>{let value=0;const generation=crypto.randomUUID();const send=window.fetch.bind(window);
document.getElementById('increment').addEventListener('click',event=>{
if(!event.isTrusted)return;value=nextCount(value);document.getElementById('count').textContent=String(value);
void send('/witness',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({generation,value,trusted:event.isTrusted})});});})();
</script>"#;

struct Page {
    work: PathBuf,
    events: Mutex<Vec<Value>>,
    served: Mutex<Vec<String>>,
}
async fn script(State(page): State<Arc<Page>>) -> axum::response::Response {
    match tokio::fs::read_to_string(page.work.join("app.js")).await {
        Ok(source) if source.len() <= 65536 => {
            let mut served = page.served.lock().unwrap();
            if served.len() < 32 {
                served.push(source.clone());
            }
            (
                [
                    ("content-type", "text/javascript; charset=utf-8"),
                    ("cache-control", "no-store"),
                ],
                source,
            )
                .into_response()
        }
        _ => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}
async fn witness(
    State(page): State<Arc<Page>>,
    Json(value): Json<Value>,
) -> axum::http::StatusCode {
    let mut events = page.events.lock().unwrap();
    if events.len() < 128 {
        events.push(value);
    }
    axum::http::StatusCode::NO_CONTENT
}
pub(super) fn credentials() -> Result<Zeroizing<String>, String> {
    if std::env::var_os("NOMIFUN_LIVE_STEPFUN_API_KEY").is_some() || std::io::stdin().is_terminal()
    {
        return Err("LIVE_CREDENTIAL_CHANNEL_INVALID".into());
    }
    let mut raw = Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .take(16385)
        .read_to_string(&mut raw)
        .map_err(|_| "LIVE_CREDENTIAL_READ_FAILED")?;
    if raw.len() > 16384 || raw.trim().is_empty() || raw.trim().contains(['\r', '\n']) {
        return Err("LIVE_CREDENTIAL_INVALID".into());
    }
    Ok(Zeroizing::new(raw.trim().to_owned()))
}
async fn api(
    server: &nomifun_app::DesktopServer,
    method: &str,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|_| "LIVE_CLIENT_FAILED")?
        .request(
            method.parse().map_err(|_| "LIVE_METHOD_INVALID")?,
            format!("http://127.0.0.1:{}{path}", server.loopback_port()),
        )
        .header("x-nomi-local-trust", server.local_trust_secret())
        .json(&body)
        .send()
        .await
        .map_err(|_| "LIVE_API_TRANSPORT_FAILED")?;
    let status = response.status();
    if !status.is_success() {
        let stage = if path.contains("/model-services/") {
            "MODEL_SERVICE"
        } else if path == "/api/providers" {
            "PROVIDER"
        } else if path.contains("/from-template/") {
            "PRESET"
        } else if path.ends_with("/revisions") {
            "REVISION"
        } else if path.ends_with("/turns") {
            "TURN"
        } else if path.starts_with("/api/conversations/") {
            "CONVERSATION"
        } else {
            "SESSION"
        };
        let value = response.json::<Value>().await.unwrap_or(Value::Null);
        let diagnostic = value.to_string().to_ascii_lowercase();
        let tags = [
            "model",
            "route",
            "resource",
            "capability",
            "workspace",
            "platform",
        ]
        .into_iter()
        .filter(|tag| diagnostic.contains(tag))
        .collect::<Vec<_>>()
        .join("_")
        .to_ascii_uppercase();
        return Err(format!(
            "LIVE_{stage}_STATUS_{}_{}",
            status.as_u16(),
            if tags.is_empty() {
                "DETAIL_REDACTED"
            } else {
                &tags
            }
        ));
    }
    let value: Value = response.json().await.map_err(|_| "LIVE_API_JSON_INVALID")?;
    Ok(value["data"].clone())
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| "LIVE_API_FIELD_MISSING".into())
}

pub(super) fn run(app: tauri::AppHandle, key: Zeroizing<String>) -> Result<Value, String> {
    let root = tempfile::Builder::new()
        .prefix("nomifun-live-browser-")
        .tempdir()
        .map_err(|_| "LIVE_TEMP_FAILED")?;
    nomifun_runtime::init(&root.path().join("data"));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .map_err(|_| "LIVE_RUNTIME_FAILED")?;
    let result = runtime.block_on(verify(&app, &root, &key));
    drop(key);
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));
    super::cleanup_fixture_profile(&root).map_err(|_| "LIVE_TEMP_CLEANUP_FAILED")?;
    result
}
async fn verify(
    app: &tauri::AppHandle,
    root: &tempfile::TempDir,
    key: &str,
) -> Result<Value, String> {
    let work = root.path().join("work");
    std::fs::create_dir(&work).map_err(|_| "LIVE_WORKSPACE_FAILED")?;
    std::fs::write(work.join("app.js"), BROKEN_JS).map_err(|_| "LIVE_WORKSPACE_FAILED")?;
    let page = Arc::new(Page {
        work: work.clone(),
        events: Mutex::new(vec![]),
        served: Mutex::new(vec![]),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "LIVE_PAGE_BIND_FAILED")?;
    let address = listener.local_addr().map_err(|_| "LIVE_PAGE_BIND_FAILED")?;
    let stop = CancellationToken::new();
    let stopped = stop.clone();
    let routes = Router::new()
        .route(
            "/",
            get(|| async { ([("cache-control", "no-store")], Html(HTML)) }),
        )
        .route("/app.js", get(script))
        .route("/witness", post(witness))
        .with_state(page.clone());
    let serving = tokio::spawn(async move {
        axum::serve(listener, routes)
            .with_graceful_shutdown(stopped.cancelled_owned())
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
    let (server, keep_alive) = nomifun_app::DesktopServer::start_with_outcome(
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
    .map_err(|_| "LIVE_BACKEND_START_FAILED")?;
    let presented = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let weak = Arc::downgrade(&server);
    let presentation_app = app.clone();
    let shown = presented.clone();
    let show_failed = failed.clone();
    let subscription = app.get_window("main").ok_or("LIVE_WINDOW_MISSING")?.listen(
        "browser-workspace-open",
        move |event| {
            if let Ok(id) = serde_json::from_str::<String>(event.payload()) {
                let (weak, shown, failed, app) = (
                    weak.clone(),
                    shown.clone(),
                    show_failed.clone(),
                    presentation_app.clone(),
                );
                tauri::async_runtime::spawn(async move {
                    let result = async {
                        let server = weak.upgrade().ok_or(())?;
                        let workspace = server
                            .browser_workspace_for_local_surface(&id)
                            .await
                            .map_err(|_| ())?;
                        workspace
                            .set_surface(
                                BrowserSurfaceBounds {
                                    x: 20.,
                                    y: 60.,
                                    width: 1060.,
                                    height: 640.,
                                },
                                true,
                                Default::default(),
                            )
                            .await
                            .map_err(|_| ())?;
                        let snapshot = workspace
                            .snapshot()
                            .await
                            .map_err(|_| ())?
                            .runtime
                            .ok_or(())?;
                        let tab = snapshot
                            .tabs
                            .iter()
                            .find(|tab| Some(&tab.target.tab_id) == snapshot.active_tab_id.as_ref())
                            .ok_or(())?;
                        let view = app.get_webview(&tab.target.tab_id).ok_or(())?;
                        if super::agent_turn::native_state(&view)
                            .await
                            .map_err(|_| ())?
                            != (false, true)
                        {
                            return Err(());
                        }
                        Ok(())
                    }
                    .await;
                    if result.is_ok() {
                        shown.store(true, Ordering::Release);
                    } else {
                        failed.store(true, Ordering::Release);
                    }
                });
            }
        },
    );
    let mut session_id = None;
    let result=tokio::time::timeout(std::time::Duration::from_secs(240),async {
        api(&server,"POST","/api/model-services/free/activate",json!({"enabled":false})).await?;
        let provider=api(&server,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"Native frontend live fixture","base_url":"https://api.stepfun.com/step_plan/v1","auth_scheme":"bearer","credentials":{"api_keys":[key]},"enabled":true,"initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","reasoning","streaming"],"protocol":"openai.chat_text","connection_role":"default","provider_params":{"temperature":0.0},"output_limit":4096}]}})).await?;
        let provider=text(&provider,"provider_id")?;
        let editor=api(&server,"POST","/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Frontend browser live fixture","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"step-3.7-flash"}})).await?;
        let preset=text(&editor["preset"],"preset_id")?;
        let catalog=api(&server,"GET","/api/capabilities",Value::Null).await?;
        let mut selections=vec![];
        for id in ["browser.navigate","browser.observe","browser.act","fs.read","fs.write","fs.patch"] {
            let item=catalog.as_array().and_then(|items|items.iter().find(|item|item["capability"]["id"]==id && item["materialization_state"]=="materialized")).ok_or("LIVE_CAPABILITY_MISSING")?;
            selections.push(json!({"capability":item["capability"],"action_allowlist":[]}));
        }
        let mut draft=editor["draft"].clone();
        draft["document"]["persona"]=json!("You are a precise frontend developer working only in the supplied temporary workspace.");
        draft["document"]["instructions"]=json!("Use the real Browser for page interaction and workspace file tools for editing. Do not read unrelated files, automate DOM events, or replace browser interaction with HTTP requests.");
        draft["document"]["enabled_capabilities"]=json!(selections);draft["document"]["skill_bindings"]=json!([]);
        api(&server,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"live native frontend conformance"})).await?;
        let session=api(&server,"POST","/api/agent-sessions",json!({"preset_id":preset,"title":"Frontend browser live fixture","resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],"model":{"provider_id":provider,"model":"step-3.7-flash"}})).await?;
        let id=text(&session,"agent_session_id")?.to_owned();session_id=Some(id.clone());
        api(&server,"PATCH",&format!("/api/conversations/{id}"),json!({"extra":{"workspace":work.to_string_lossy()}})).await?;
        let prompt=format!("请验证并修复临时前端应用 http://{address}/ 。必须先用 Browser 打开网页、观察并真实点击一次 Increment，确认 Count 错误地变成 2；再读取工作区 app.js，修复 nextCount 使每次只增加 1。只能修改 app.js，不能添加自动点击、伪造事件或改变测试页面。修复后用 Browser 刷新，依次真实点击三次，每次重新观察，确认 Count 分别为 1、2、3，最后停留在 3。不得调用 evaluate、脚本点击或其他浏览器。修复与复测结束后简短报告。请直接执行，不要只给建议。");
        api(&server,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"input":{"content":prompt},"idempotency_key":uuid::Uuid::now_v7().to_string()})).await?;
        let mut running=false;
        loop {
            let state=api(&server,"GET",&format!("/api/conversations/{id}"),Value::Null).await?;
            if state["status"]=="running" {running=true;}
            if state["status"]=="finished" {break;}
            if state["status"]=="error" || state["status"]=="cancelled" {return Err("LIVE_TURN_FAILED".into());}
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if !running || !presented.load(Ordering::Acquire) || failed.load(Ordering::Acquire) {return Err("LIVE_PRESENTATION_NOT_PROVEN".into());}
        let workspace=server.browser_workspace_for_local_surface(&id).await.map_err(|_|"LIVE_WORKSPACE_MISSING")?;
        let snapshot=workspace.snapshot().await.map_err(|_|"LIVE_SNAPSHOT_FAILED")?;
        if snapshot.run.input_state!=BrowserInputState::UserReady {return Err("LIVE_TERMINAL_NOT_UNLOCKED".into());}
        let runtime=snapshot.runtime.ok_or("LIVE_NATIVE_RUNTIME_MISSING")?;
        if runtime.tabs.len()!=1 {return Err("LIVE_NATIVE_TAB_COUNT".into());}
        let view=app.get_webview(&runtime.tabs[0].target.tab_id).ok_or("LIVE_NATIVE_VIEW_MISSING")?;
        if super::agent_turn::native_state(&view).await.map_err(|_|"LIVE_NATIVE_STATE_FAILED")? != (true,true) {return Err("LIVE_NATIVE_UNLOCK_NOT_PROVEN".into());}
        let value=super::evaluate(&view,"document.getElementById('count').textContent").await.map_err(|_|"LIVE_FINAL_DOM_FAILED")?;
        if value!="3" {return Err("LIVE_FINAL_COUNT_WRONG".into());}
        let events=page.events.lock().unwrap().clone();
        let first=events.first().ok_or("LIVE_REPRODUCTION_MISSING")?;
        let last=events.get(events.len().saturating_sub(3)..).ok_or("LIVE_RETEST_MISSING")?;
        if first["value"]!=2 || first["trusted"]!=true || last.len()!=3 || last.iter().enumerate().any(|(i,e)|e["value"]!=i+1 || e["trusted"]!=true || e["generation"]==first["generation"] || e["generation"]!=last[0]["generation"]) {return Err("LIVE_TRUSTED_RETEST_NOT_PROVEN".into());}
        let served=page.served.lock().unwrap();
        if served.first().map(String::as_str)!=Some(BROKEN_JS) || served.last().map(String::as_str)==Some(BROKEN_JS) {return Err("LIVE_SOURCE_CHANGE_NOT_SERVED".into());}
        Ok(json!({"model":"step-3.7-flash","real_provider":true,"native_auto_open":true,"reproduced_bug_with_trusted_click":true,"workspace_code_changed_and_reloaded":true,"trusted_retest_values":[1,2,3],"terminal_before_unlock":true}))
    }).await.unwrap_or_else(|_|Err("LIVE_FRONTEND_TIMEOUT".into()));
    if let Some(id) = session_id {
        if result.is_err() {
            if let Ok(evidence) = diagnostic(
                &server,
                &id,
                &page,
                presented.load(Ordering::Acquire),
                failed.load(Ordering::Acquire),
            )
            .await
            {
                eprintln!("NOMIFUN_BROWSER_LIVE_EVIDENCE {evidence}");
            }
        }
        let _ = api(
            &server,
            "POST",
            &format!("/api/conversations/{id}/cancel"),
            json!({}),
        )
        .await;
    }
    app.unlisten(subscription);
    let shutdown = server.shutdown_all().await;
    drop(server);
    drop(keep_alive);
    stop.cancel();
    serving
        .await
        .map_err(|_| "LIVE_PAGE_SHUTDOWN_FAILED")?
        .map_err(|_| "LIVE_PAGE_SHUTDOWN_FAILED")?;
    shutdown.map_err(|_| "LIVE_BACKEND_SHUTDOWN_FAILED")?;
    result
}

async fn diagnostic(
    server: &nomifun_app::DesktopServer,
    id: &str,
    page: &Page,
    presented: bool,
    presentation_failed: bool,
) -> Result<Value, String> {
    let mut messages = vec![];
    let mut cursor = 0;
    for _ in 0..16 {
        let history = api(
            server,
            "GET",
            &format!("/api/agent-sessions/{id}/messages?after_seq={cursor}&limit=100"),
            Value::Null,
        )
        .await?;
        let rows = history["messages"]
            .as_array()
            .ok_or("LIVE_HISTORY_INVALID")?;
        messages.extend(rows.iter().cloned());
        let next = history["next_cursor"]["seq"]
            .as_u64()
            .ok_or("LIVE_HISTORY_INVALID")?;
        if next <= cursor || rows.is_empty() {
            break;
        }
        cursor = next;
    }
    let tools=messages.iter().filter_map(|row| {
        let p=&row["projection"];
        let name=p["name"].as_str()?;
        let name=match name {"Browser"|"Read"|"Write"|"Edit"|"apply_patch"|"update_plan"|"AskUserQuestion"=>name,_=>"OTHER"};
        let status=match p["status"].as_str(){Some("completed")=>"completed",Some("failed"|"error")=>"failed",_=>"other"};
        let operation=p["args"]["operation"].as_str().filter(|op|["navigate","observe","act","tab","diagnostics","screenshot"].contains(op));
        let action=p["args"]["action"]["action"].as_str().filter(|op|["click","type","press","scroll","hover"].contains(op));
        let raw=p.to_string();
        let codes=["INVALID_PAYLOAD","CAPABILITY_NOT_SELECTED","BROWSER_STALE_OBSERVATION","BROWSER_STALE_TARGET","BROWSER_NOT_ACTIONABLE","BROWSER_NATIVE_COMMAND_FAILED","BROWSER_UNSUPPORTED_ACTION","TOOL_NOT_FOUND","PERMISSION_DENIED"].into_iter().filter(|code|raw.contains(code)).collect::<Vec<_>>();
        Some(json!({"name":name,"status":status,"operation":operation,"action":action,"error_present":!p["error"].is_null(),"result_error":p["result"]["is_error"].as_bool(),"error_codes":codes}))
    }).take(64).collect::<Vec<_>>();
    let numbers = page
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["value"].as_i64().filter(|n| (-100..=100).contains(n)))
        .collect::<Vec<_>>();
    let versions = page.served.lock().unwrap().len();
    let changed = std::fs::read_to_string(page.work.join("app.js"))
        .ok()
        .is_some_and(|source| source != BROKEN_JS);
    Ok(
        json!({"values":numbers,"served_versions":versions,"source_changed":changed,"tools":tools,"presented":presented,"presentation_failed":presentation_failed}),
    )
}

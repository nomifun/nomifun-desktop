//! Opt-in real-model frontend loop. Only disposable fixture code and page
//! observations reach the configured test provider; credentials arrive via stdin.
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use futures_util::StreamExt;
use nomifun_browser_platform::{
    run_guard::BrowserInputState, runtime::BrowserSurfaceBounds, workspace::BrowserResourceService,
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
use tokio_tungstenite::tungstenite::{
    Message as WebSocketFrame, client::IntoClientRequest,
    http::{HeaderValue, header::SEC_WEBSOCKET_PROTOCOL},
};
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
    work: Mutex<PathBuf>,
    events: Mutex<Vec<Value>>,
    served: Mutex<Vec<String>>,
    tool_shapes: Mutex<Vec<Value>>,
}
async fn script(State(page): State<Arc<Page>>) -> axum::response::Response {
    let work = page
        .work
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    match tokio::fs::read_to_string(work.join("app.js")).await {
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
        } else if path.starts_with("/api/agent-sessions/") {
            "AGENT_SESSION"
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

fn browser_act_shape(args: &Value) -> Value {
    let Some(outer) = args.as_object() else {
        return json!({"class":"non_object","tag":Value::Null});
    };
    if outer.contains_key("operation") {
        return json!({"class":"legacy_operation","tag":Value::Null});
    }
    let Some(tag_value) = outer.get("action") else {
        return json!({"class":"missing_action_wrapper","tag":Value::Null});
    };
    if tag_value.is_object() {
        return json!({"class":"nested_action_wrapper","tag":Value::Null});
    }
    let action = outer;
    let tag = tag_value
        .as_str()
        .filter(|tag| {
            ["click", "hover", "type", "press", "select", "scroll", "drag", "dialog"]
                .contains(tag)
        });
    let class = if action.contains_key("tab_id")
        || action.contains_key("observation_id")
        || action.contains_key("ref_id")
    {
        "private_attached"
    } else if action.contains_key("ref") || action.contains_key("selector") {
        "legacy_reference"
    } else if let Some(element) = action.get("element") {
        match element.as_object() {
            Some(element)
                if element.contains_key("target")
                    && element.contains_key("observation_generation")
                    && element.contains_key("ref_id") =>
            {
                "raw_reference"
            }
            Some(element)
                if element.contains_key("reference")
                    && element.contains_key("role")
                    && element.contains_key("name")
                    && element.contains_key("focused") =>
            {
                "canonical_element"
            }
            Some(element) if element.contains_key("reference") => "partial_observed_element",
            Some(_) => "other_element_object",
            None => "scalar_element",
        }
    } else if action.contains_key("from") || action.contains_key("target") {
        "other_canonical_variant"
    } else {
        "missing_reference"
    };
    json!({
        "class":class,
        "tag":tag,
        "outer_unknown_keys":0,
        "action_unknown_keys":action.keys().filter(|key| ![
            "action","element","from","to","button","click_count","text","keys","labels",
            "delta_x","delta_y","target","request_id","accept","tab_id","observation_id",
            "ref_id","ref","selector"
        ].contains(&key.as_str())).count(),
    })
}

async fn start_tool_shape_capture(
    server: &nomifun_app::DesktopServer,
    page: Arc<Page>,
    stop: CancellationToken,
) -> Result<tokio::task::JoinHandle<()>, String> {
    let mut request = format!("ws://127.0.0.1:{}/ws", server.loopback_port())
        .into_client_request()
        .map_err(|_| "LIVE_STREAM_CAPTURE_FAILED")?;
    request.headers_mut().insert(
        SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_str(server.local_trust_secret())
            .map_err(|_| "LIVE_STREAM_CAPTURE_FAILED")?,
    );
    let (mut stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|_| "LIVE_STREAM_CAPTURE_FAILED")?;
    Ok(tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = stop.cancelled() => break,
                frame = stream.next() => {
                    let Some(Ok(WebSocketFrame::Text(text))) = frame else { break; };
                    let Ok(event) = serde_json::from_str::<Value>(text.as_ref()) else { continue; };
                    if event["name"] != "message.stream" || event["data"]["type"] != "tool_call" {
                        continue;
                    }
                    let tool = &event["data"]["data"];
                    if !tool["name"].as_str().is_some_and(|name| name.starts_with("platform__browser_browser_act__"))
                        || tool["status"] != "running"
                    {
                        continue;
                    }
                    let mut shapes = page.tool_shapes.lock().unwrap_or_else(|error| error.into_inner());
                    if shapes.len() < 32 {
                        shapes.push(browser_act_shape(&tool["args"]));
                    }
                }
            }
        }
    }))
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
    let cleanup = super::cleanup_fixture_profile(&root);
    match (result, cleanup) {
        (Ok(evidence), Ok(())) => Ok(evidence),
        (Err(error), _) => Err(error),
        (Ok(_), Err(_)) => Err("LIVE_TEMP_CLEANUP_FAILED".into()),
    }
}
async fn verify(
    app: &tauri::AppHandle,
    root: &tempfile::TempDir,
    key: &str,
) -> Result<Value, String> {
    let work = root.path().join("work");
    std::fs::create_dir(&work).map_err(|_| "LIVE_WORKSPACE_FAILED")?;
    let page = Arc::new(Page {
        work: Mutex::new(work.clone()),
        events: Mutex::new(vec![]),
        served: Mutex::new(vec![]),
        tool_shapes: Mutex::new(vec![]),
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
    let (server, keep_alive) = nomifun_app::DesktopServer::start_with_outcome(
        &cli,
        "",
        None,
        None,
        None,
        nomifun_app::DesktopHostServices {
            browser_resources: Some(browser_resources.clone()),
            ..Default::default()
        },
    )
    .await
    .map_err(|_| "LIVE_BACKEND_START_FAILED")?;
    let capture_stop = CancellationToken::new();
    let capture = start_tool_shape_capture(&server, page.clone(), capture_stop.clone()).await?;
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
                            .browser_resource_for_local_surface(&id)
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
    let result=tokio::time::timeout(std::time::Duration::from_secs(180),async {
        api(&server,"POST","/api/model-services/free/activate",json!({"enabled":false})).await?;
        let provider=api(&server,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"Native frontend live fixture","base_url":"https://api.stepfun.com/step_plan/v1","auth_scheme":"bearer","credentials":{"api_keys":[key]},"enabled":true,"initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","reasoning","streaming"],"protocol":"openai.chat_text","connection_role":"default","provider_params":{"temperature":0.0},"output_limit":4096}]}})).await?;
        let provider=text(&provider,"provider_id")?;
        let editor=api(&server,"POST","/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Frontend browser live fixture","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"step-3.7-flash"}})).await?;
        let preset=text(&editor["preset"],"preset_id")?;
        let catalog=api(&server,"GET","/api/capabilities",Value::Null).await?;
        let items=catalog.as_array().ok_or("LIVE_CAPABILITY_CATALOG_INVALID")?;
        let browser=items.iter().find(|item|item["capability"]["id"]=="browser" && item["materialization_state"]=="materialized").ok_or("LIVE_BROWSER_MODULE_MISSING")?;
        let selections=vec![json!({"capability":browser["capability"],"action_allowlist":["browser/navigate","browser/observe","browser/act"]})];
        let mut draft=editor["draft"].clone();
        draft["document"]["persona"]=json!("You are a precise Browser acceptance agent.");
        draft["document"]["instructions"]=json!("Use only the selected real Browser Actions. For browser/act click, use {\"action\":\"click\",\"element\":ELEMENT} where ELEMENT is the complete element object (reference, role, name, focused) copied unchanged from the latest browser/observe result. Do not add another action wrapper and do not use tab_id, observation_id, ref, selector, evaluate, HTTP requests, workspace tools, or synthetic DOM events.");
        draft["document"]["enabled_capabilities"]=json!(selections);draft["document"]["skill_bindings"]=json!([]);
        api(&server,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"live native frontend conformance"})).await?;
        let session=api(&server,"POST","/api/agent-sessions",json!({"preset_id":preset,"title":"Frontend browser live fixture","resource_selections":[{"resource_kind":"browser","resource_id":"managed-browser"}],"model":{"provider_id":provider,"model":"step-3.7-flash"}})).await?;
        let id=text(&session,"agent_session_id")?.to_owned();session_id=Some(id.clone());
        let session_work=work.join("agent-sessions").join(&id);
        std::fs::create_dir_all(&session_work).map_err(|_|"LIVE_WORKSPACE_FAILED")?;
        std::fs::write(session_work.join("app.js"),BROKEN_JS).map_err(|_|"LIVE_WORKSPACE_FAILED")?;
        *page.work.lock().unwrap_or_else(|error|error.into_inner())=session_work;
        let prompt=format!("请用 Browser 打开 http://{address}/ ，观察页面，并对 Increment 按钮执行恰好一次真实 click。再次观察并确认 Count 变成 2 后立即结束并简短报告。不得读取或修改文件，不得调用 evaluate、脚本点击、HTTP 请求或其他浏览器；请直接执行，不要只给建议。");
        api(&server,"POST",&format!("/api/agent-sessions/{id}/turns"),json!({"input":{"content":prompt},"idempotency_key":uuid::Uuid::now_v7().to_string()})).await?;
        let mut running=false;
        let mut evidence_driven_cancel=false;
        loop {
            let state=api(&server,"GET",&format!("/api/agent-sessions/{id}/projection"),Value::Null).await?;
            if state["status"]=="running" {running=true;}
            if state["status"]=="running" && !evidence_driven_cancel {
                let click_proven=page.events.lock().unwrap_or_else(|error|error.into_inner()).as_slice().first().is_some_and(|event|event["value"]==2 && event["trusted"]==true);
                if click_proven {
                    api(&server,"POST",&format!("/api/agent-sessions/{id}/turns/cancel"),json!({"idempotency_key":uuid::Uuid::now_v7().to_string()})).await?;
                    evidence_driven_cancel=true;
                }
            }
            if state["status"]=="finished" || state["status"]=="cancelled" {break;}
            if state["status"]=="error" || state["status"]=="cancelled" {return Err("LIVE_TURN_FAILED".into());}
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if !running || !evidence_driven_cancel || !presented.load(Ordering::Acquire) || failed.load(Ordering::Acquire) {return Err("LIVE_PRESENTATION_NOT_PROVEN".into());}
        let workspace=server.browser_resource_for_local_surface(&id).await.map_err(|_|"LIVE_WORKSPACE_MISSING")?;
        let snapshot=workspace.snapshot().await.map_err(|_|"LIVE_SNAPSHOT_FAILED")?;
        if snapshot.run.input_state!=BrowserInputState::UserReady {return Err("LIVE_TERMINAL_NOT_UNLOCKED".into());}
        let runtime=snapshot.runtime.ok_or("LIVE_NATIVE_RUNTIME_MISSING")?;
        if runtime.tabs.len()!=1 {return Err("LIVE_NATIVE_TAB_COUNT".into());}
        let view=app.get_webview(&runtime.tabs[0].target.tab_id).ok_or("LIVE_NATIVE_VIEW_MISSING")?;
        if super::agent_turn::native_state(&view).await.map_err(|_|"LIVE_NATIVE_STATE_FAILED")? != (true,true) {return Err("LIVE_NATIVE_UNLOCK_NOT_PROVEN".into());}
        let value=super::evaluate(&view,"document.getElementById('count').textContent").await.map_err(|_|"LIVE_FINAL_DOM_FAILED")?;
        if value!="2" {return Err("LIVE_FINAL_COUNT_WRONG".into());}
        let events=page.events.lock().unwrap().clone();
        let [click]=events.as_slice() else {return Err("LIVE_SINGLE_TRUSTED_CLICK_NOT_PROVEN".into());};
        if click["value"]!=2 || click["trusted"]!=true {return Err("LIVE_SINGLE_TRUSTED_CLICK_NOT_PROVEN".into());}
        let served=page.served.lock().unwrap();
        if served.first().map(String::as_str)!=Some(BROKEN_JS) {return Err("LIVE_PAGE_SOURCE_NOT_SERVED".into());}
        let canonical_shape=page.tool_shapes.lock().unwrap_or_else(|error|error.into_inner()).iter().any(|shape|shape["class"]=="canonical_element" && shape["tag"]=="click");
        if !canonical_shape {return Err("LIVE_CANONICAL_ACTION_SHAPE_NOT_PROVEN".into());}
        Ok(json!({"model":"step-3.7-flash","real_provider":true,"native_auto_open":true,"trusted_click_value":2,"canonical_action_shape":true,"evidence_driven_cancel":true,"terminal_before_unlock":true}))
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
            &format!("/api/agent-sessions/{id}/turns/cancel"),
            json!({"idempotency_key":uuid::Uuid::now_v7().to_string()}),
        )
        .await;
    }
    app.unlisten(subscription);
    capture_stop.cancel();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), capture).await;
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
        let summary=&p["tool_summary"];
        let action_id=summary["action_id"].as_str()?;
        let name=match action_id {
            "browser/navigate"|"browser/observe"|"browser/act"=>"Browser",
            "workspace.files/read"=>"Read",
            "workspace.files/write"=>"Write",
            "workspace.files/patch"=>"Edit",
            _=>"OTHER",
        };
        let status=match p["state"].as_str(){Some("recorded"|"completed")=>"completed",Some("failed"|"error"|"uncertain")=>"failed",_=>"other"};
        let operation=action_id.strip_prefix("browser/").filter(|op|["navigate","observe","act"].contains(op));
        let raw=p.to_string();
        let codes=["INVALID_PAYLOAD","CAPABILITY_NOT_SELECTED","BROWSER_STALE_OBSERVATION","BROWSER_STALE_TARGET","BROWSER_NOT_ACTIONABLE","BROWSER_NATIVE_COMMAND_FAILED","BROWSER_UNSUPPORTED_ACTION","TOOL_NOT_FOUND","PERMISSION_DENIED"].into_iter().filter(|code|raw.contains(code)).collect::<Vec<_>>();
        let error=summary["error"].as_str().unwrap_or_default().to_ascii_lowercase();
        let error_hints=[
            ("oneof","one_of"),("one of","one_of"),
            ("required","required"),("additional","additional_property"),
            ("element","element"),("target","target"),("observation_generation","observation_generation"),
            ("ref_id","ref_id"),("operation","operation"),("action","action"),
        ].into_iter().filter_map(|(needle,label)|error.contains(needle).then_some(label)).collect::<std::collections::BTreeSet<_>>();
        Some(json!({"name":name,"status":status,"operation":operation,"action":Value::Null,"error_present":summary.get("error").is_some(),"result_error":Value::Null,"error_codes":codes,"error_hints":error_hints}))
    }).take(64).collect::<Vec<_>>();
    let numbers = page
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["value"].as_i64().filter(|n| (-100..=100).contains(n)))
        .collect::<Vec<_>>();
    let versions = page.served.lock().unwrap().len();
    let tool_shapes = page
        .tool_shapes
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let work = page
        .work
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let changed = std::fs::read_to_string(work.join("app.js"))
        .ok()
        .is_some_and(|source| source != BROKEN_JS);
    Ok(
        json!({"count":messages.len(),"values":numbers,"served_versions":versions,"source_changed":changed,"tools":tools,"tool_shapes":tool_shapes,"presented":presented,"presentation_failed":presentation_failed}),
    )
}

//! Prepare a NEW disposable main-app dataset and a loopback-only held model.
//! No real provider credentials, user dataset, or browser profile is read.
//! Usage: cargo run -p nomifun-app --example browser_gui_fixture -- <new-data-dir> [--native-actions]
//! Launch the real desktop EXE with NOMIFUN_DATA_DIR set to the printed path.
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use nomifun_app::{DesktopHostServices, DesktopServer};
use nomifun_browser_platform::{
    runtime::{BrowserRuntime, BrowserRuntimeFactory, CreateBrowserRuntime, WorkspaceError},
    workspace::BrowserWorkspaceService,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

struct LiveFrontend {
    key: Zeroizing<String>,
    local_token: String,
    client: reqwest::Client,
    work: PathBuf,
    served: Mutex<Vec<String>>,
}
const BROKEN_JS: &str = "function nextCount(value) { return value + 2; }\n";
const FRONTEND_PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Counter app</title>
<style>body{font:20px system-ui;padding:36px}button{font:inherit;padding:12px 24px}output{display:block;font-size:32px;margin:24px 0}</style>
<h1>Counter app</h1><label for=count>Count</label><output id=count>0</output><button id=increment>Increment</button>
<script src=/app.js></script><script>(()=>{let value=0;const nonce=crypto.randomUUID();const send=window.fetch.bind(window);increment.onclick=e=>{if(!e.isTrusted)return;value=nextCount(value);document.getElementById('count').textContent=String(value);void send('/witness',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({nonce,count:value,trusted:e.isTrusted})});};})();</script>"#;

struct Fixture {
    live: Option<LiveFrontend>,
    calls: AtomicUsize,
    native_url: Option<String>,
    witnesses: Mutex<Vec<Value>>,
    failure: Mutex<Option<String>>,
    finish: Semaphore,
    stop: CancellationToken,
}

/// Declares the native Browser provider while the disposable dataset is being
/// compiled, but can never execute a page. The subsequently launched desktop
/// replaces this preparatory host with the real WebView2 factory before any
/// Agent turn is accepted.
struct PreparatoryBrowserFactory;

#[async_trait::async_trait]
impl BrowserRuntimeFactory for PreparatoryBrowserFactory {
    async fn create(
        &self,
        _: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        Err(WorkspaceError::NativeUnavailable)
    }
}
const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>浏览器运行锁验收</title>
<style>body{font:18px system-ui;padding:32px}button,input{font:inherit;padding:10px;margin:12px 0;display:block}</style>
<h1>真实原生页面</h1><p>计数：<output id="count">0</output></p><button id="increment">增加计数</button>
<label for="note">备注</label><input id="note" placeholder="输入中文"><p id="status">等待点击</p>
<script>window.fixtureNonce=crypto.randomUUID();let count=0;increment.onclick=e=>{if(e.isTrusted){document.getElementById('count').textContent=String(++count);document.getElementById('status').textContent='已收到真实点击';void fetch('/witness',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({nonce:fixtureNonce,count,note:document.getElementById('note').value,trusted:e.isTrusted})});}};</script>"#;

fn last_tool(body: &Value) -> anyhow::Result<Value> {
    let content = body["messages"].as_array()
        .and_then(|messages| messages.iter().rev().find(|message| message["role"] == "tool"))
        .and_then(|message| message["content"].as_str())
        .ok_or_else(|| anyhow::anyhow!("Browser result missing"))?;
    let value: Value = serde_json::from_str(content)?;
    anyhow::ensure!(value.get("code").is_none(), "Browser returned error: {value}");
    Ok(value)
}

fn native_operation(fixture: &Fixture, body: &Value, step: usize) -> anyhow::Result<Option<Value>> {
    anyhow::ensure!(body["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool| tool["function"]["name"] == "Browser")), "Selected Browser tool missing from model request");
    if step > 0 { last_tool(body)?; }
    let reference = |name: &str| -> anyhow::Result<Value> {
        let observation = last_tool(body)?;
        observation["elements"].as_array()
            .and_then(|elements| elements.iter().find(|element| element["name"] == name))
            .map(|element| element["reference"].clone())
            .ok_or_else(|| anyhow::anyhow!("Fresh observed reference missing: {name}"))
    };
    Ok(Some(match step {
        0 => json!({"operation":"navigate","url":fixture.native_url}),
        1 | 3 | 5 => json!({"operation":"observe"}),
        2 => json!({"operation":"act","action":{"action":"type","element":reference("备注")?,"text":"Agent 主界面真实输入"}}),
        4 => json!({"operation":"act","action":{"action":"click","element":reference("增加计数")?}}),
        6 => {
            anyhow::ensure!(last_tool(body)?.to_string().contains("已收到真实点击"), "Post-click observation omitted the real page result");
            return Ok(None);
        }
        _ => anyhow::bail!("Unexpected model request {step}"),
    }))
}

async fn model(State(fixture): State<Arc<Fixture>>, headers: axum::http::HeaderMap, Json(mut body): Json<Value>) -> axum::response::Response {
    if let Some(live) = &fixture.live {
        if headers.get("authorization").and_then(|value|value.to_str().ok()) != Some(live.local_token.as_str()) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        if !body.is_object() || !body["messages"].is_array() { return axum::http::StatusCode::BAD_REQUEST.into_response(); }
        if fixture.calls.fetch_add(1, Ordering::SeqCst) >= 32 { return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response(); }
        body["model"] = json!("step-3.7-flash");
        body["max_tokens"] = json!(4096);
        body["temperature"] = json!(0);
        body["stream"] = json!(true);
        let response = tokio::select! {
            _=fixture.stop.cancelled()=>return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
            response=live.client.post("https://api.stepfun.com/step_plan/v1/chat/completions").bearer_auth(live.key.as_str()).json(&body).send()=>response,
        };
        return match response {
            Ok(response) if response.status().is_success() => axum::response::Response::builder()
                .header("content-type","text/event-stream")
                .body(axum::body::Body::from_stream(response.bytes_stream())).unwrap(),
            Ok(response) => {
                *fixture.failure.lock().unwrap()=Some(format!("upstream_status_{}",response.status().as_u16()));
                (axum::http::StatusCode::BAD_GATEWAY, Json(json!({"error":{"message":"Live test provider rejected the request"}}))).into_response()
            }
            Err(_) => {
                *fixture.failure.lock().unwrap()=Some("upstream_transport_failed".into());
                axum::http::StatusCode::BAD_GATEWAY.into_response()
            }
        };
    }
    fixture.calls.fetch_add(1, Ordering::SeqCst);
    // Each user turn starts a new sequence; never replay a prior turn's refs.
    let step = body["messages"].as_array().map(|messages| messages.iter().rev()
        .take_while(|message| message["role"] != "user")
        .filter(|message| message["role"] == "tool").count()).unwrap_or(0);
    let operation = if fixture.native_url.is_some() {
        match native_operation(&fixture, &body, step) {
            Ok(operation) => operation,
            Err(error) => {
                *fixture.failure.lock().unwrap() = Some(error.to_string());
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string(),"type":"fixture_error"}}))).into_response();
            }
        }
    } else { None };
    let (delta, reason) = if let Some(operation) = operation {
        (json!({"role":"assistant","tool_calls":[{"index":0,"id":format!("gui-native-{step}"),"type":"function","function":{"name":"Browser","arguments":operation.to_string()}}]}), "tool_calls")
    } else {
        tokio::select! {
            _=fixture.stop.cancelled()=>{},
            permit=fixture.finish.acquire()=>{ if let Ok(permit)=permit { permit.forget(); } },
        }
        (json!({"role":"assistant","content":"本机测试模型已结束。"}), "stop")
    };
    let chunk = |delta: Value, finish: Option<&str>| {
        json!({"id":"gui-fixture","object":"chat.completion.chunk","created":1,"model":"browser-gui-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
    };
    (
        [("content-type", "text/event-stream")],
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            chunk(delta, None),
            chunk(json!({}), Some(reason))
        ),
    ).into_response()
}
async fn api(app: &DesktopServer, path: &str, body: Value) -> anyhow::Result<Value> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()?
        .post(format!("http://127.0.0.1:{}{path}", app.loopback_port()))
        .header("x-nomi-local-trust", app.local_trust_secret())
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let value: Value = response.json().await?;
    anyhow::ensure!(
        status.is_success(),
        "fixture setup {path}: {status} {value}"
    );
    Ok(value["data"].clone())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("new absolute data directory required"))?,
    );
    anyhow::ensure!(
        root.is_absolute() && !root.exists(),
        "refusing an existing or relative data directory"
    );
    std::fs::create_dir(&root)?;
    let live_mode = std::env::args().nth(2).as_deref() == Some("--live-frontend");
    let live = if live_mode {
        use std::io::{IsTerminal, Read};
        anyhow::ensure!(std::env::var_os("NOMIFUN_LIVE_STEPFUN_API_KEY").is_none() && !std::io::stdin().is_terminal(), "Live key must arrive only through stdin");
        let mut key = Zeroizing::new(String::new());
        std::io::stdin().lock().take(16385).read_to_string(&mut key)?;
        anyhow::ensure!(key.len()<=16384 && !key.trim().is_empty() && !key.trim().contains(['\r','\n']), "Invalid live key input");
        let work = root.join("work");
        std::fs::create_dir(&work)?;
        std::fs::write(work.join("app.js"),BROKEN_JS)?;
        Some(LiveFrontend { key:Zeroizing::new(key.trim().to_owned()),local_token:format!("Bearer {}",uuid::Uuid::new_v4()),
            client:reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(90)).build()?,
            work,served:Mutex::new(vec![]) })
    } else { None };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let native_actions = std::env::args().nth(2).as_deref() == Some("--native-actions");
    let fixture = Arc::new(Fixture {
        live,
        calls: AtomicUsize::new(0),
        native_url: native_actions.then(|| format!("http://{address}/")),
        witnesses: Mutex::new(Vec::new()),
        failure: Mutex::new(None),
        finish: Semaphore::new(0),
        stop: CancellationToken::new(),
    });
    let routes = Router::new()
        .route("/", get(|State(f):State<Arc<Fixture>>| async move { ([("cache-control","no-store")],Html(if f.live.is_some() {FRONTEND_PAGE} else {PAGE})) }))
        .route("/app.js", get(|State(f):State<Arc<Fixture>>| async move {
            if let Some(live)=&f.live {
                if let Ok(source)=tokio::fs::read_to_string(live.work.join("app.js")).await {
                    if source.len()<=65536 {
                        let mut served=live.served.lock().unwrap(); if served.len()<64 {served.push(source.clone());}
                        return ([("content-type","text/javascript; charset=utf-8"),("cache-control","no-store")],source).into_response();
                    }
                }
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }))
        .route("/v1/chat/completions", post(model))
        .route("/witness", post(|State(f): State<Arc<Fixture>>, Json(value): Json<Value>| async move {
            let mut witnesses = f.witnesses.lock().unwrap();
            if witnesses.len() < 64 { witnesses.push(value); }
            axum::http::StatusCode::NO_CONTENT
        }))
        .route(
            "/status",
            get(|State(f): State<Arc<Fixture>>| async move {
                let versions=f.live.as_ref().map(|live|live.served.lock().unwrap().clone()).unwrap_or_default();
                Json(json!({"model_calls":f.calls.load(Ordering::SeqCst),"real_provider":f.live.is_some(),"native_actions":f.native_url.is_some(),"failure":*f.failure.lock().unwrap(),"witnesses":*f.witnesses.lock().unwrap(),"served_versions":versions.len(),"changed_source_served":versions.last().is_some_and(|source|source!=BROKEN_JS)}))
            }),
        )
        .route(
            "/finish",
            post(|State(f): State<Arc<Fixture>>| async move {
                f.finish.add_permits(1);
                "released"
            }),
        )
        .route(
            "/shutdown",
            post(|State(f): State<Arc<Fixture>>| async move {
                f.stop.cancel();
                "stopped"
            }),
        )
        .with_state(fixture.clone());
    let stop = fixture.stop.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, routes)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
    });
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".into(),
        port: 0,
        data_dir: root.clone(),
        work_dir: fixture.live.as_ref().map(|live|live.work.clone()),
        app_version: env!("CARGO_PKG_VERSION").into(),
        local: true,
        log_dir: Some(root.join("logs")),
        log_level: Some("off".into()),
        command: None,
    };
    let browser_workspaces = Arc::new(BrowserWorkspaceService::new(Arc::new(
        PreparatoryBrowserFactory,
    )));
    let (app, keep_alive) = DesktopServer::start_with_outcome(
        &cli,
        "",
        None,
        None,
        None,
        DesktopHostServices {
            browser_workspaces: Some(browser_workspaces),
            ..Default::default()
        },
    )
    .await?;
    let prepared = async {
        let local_key=fixture.live.as_ref().map(|live|live.local_token.strip_prefix("Bearer ").unwrap()).unwrap_or("local-fixture-not-a-secret");
        let provider = api(&app,"/api/providers",json!({"platform":"custom","name":if live_mode {"真实模型前端验收"}else{"本机浏览器验收模型"},"base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":[local_key]},"enabled":true,"initial_model":{"model":"browser-gui-fixture","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider = provider["provider_id"].as_str().ok_or_else(|| anyhow::anyhow!("provider missing"))?.to_owned();
        let editor = api(&app,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"浏览器主界面验收","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"browser-gui-fixture"}})).await?;
        let preset = editor["preset"]["preset_id"].as_str().ok_or_else(|| anyhow::anyhow!("preset missing"))?.to_owned();
        let mut draft=editor["draft"].clone();
        draft["document"]["enabled_capabilities"]=json!([
            {"capability":{"id":"browser.observe","version":"1.0.0"}},
            {"capability":{"id":"browser.navigate","version":"1.0.0"}},
            {"capability":{"id":"browser.act","version":"1.0.0"}}
        ]);
        let revision_path=format!("/api/agent-presets/{preset}/revisions");
        let saved=api(&app,&revision_path,json!({"expected_current_revision":draft["current_revision"].clone(),"draft":draft,"reason":"deterministic native Browser GUI acceptance"})).await?;
        anyhow::ensure!(saved["revision"]["document"]["enabled_capabilities"].as_array().is_some_and(|values|values.len()==3),"Browser fixture revision missing selected capabilities");
        let session = api(&app,"/api/agent-sessions",json!({"preset_id":preset,"title":"浏览器主界面验收","model":{"provider_id":provider,"model":"browser-gui-fixture"}})).await?;
        Ok::<_,anyhow::Error>(session["agent_session_id"].clone())
    }.await;
    let cleanup = app.shutdown_all().await;
    drop(app);
    drop(keep_alive);
    if prepared.is_err() || cleanup.is_err() {
        fixture.stop.cancel();
        task.await??;
    }
    cleanup?;
    let session = prepared?;
    println!(
        "BROWSER_GUI_FIXTURE_READY {}",
        json!({"data_dir":root,"work_dir":fixture.live.as_ref().map(|live|&live.work),"real_provider":live_mode,"page":format!("http://{address}/"),"control":format!("http://{address}"),"session_id":session})
    );
    // Keep only the model/page server alive; the real desktop now owns the DB.
    fixture.stop.cancelled().await;
    Ok(())
}

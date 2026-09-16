//! Main-application HTTP -> Nomi model -> real borrowed Chrome conformance.
//! Discovery targets an independently owned disposable Profile. The atomic
//! cancellation variant delays real CDP replies using a test-only relay.
#![cfg(windows)]
#[path = "system_browser_agent/input_proxy.rs"]
mod input_proxy;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chromiumoxide::cdp::{
    browser_protocol::{
        browser::GetVersionParams,
        target::{AttachToTargetParams, CreateTargetParams, GetTargetsParams},
    },
    js_protocol::runtime::EvaluateParams,
};
use futures_util::FutureExt;
use nomifun_app::system_browser::{SystemBrowserService, conformance_connection_factory};
use nomifun_app::{DesktopHostServices, DesktopServer};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const PRIVATE_TAB: &str = "UNAUTHORIZED_BROWSER_TAB_SENTINEL";
const COOKIE: &str = "borrowed_fixture_login=EXISTING_HTTP_ONLY_SESSION_SENTINEL";
const HTML: &str = r#"<!doctype html><title>Borrowed browser fixture</title><body>
<input id="field" aria-label="Input"><button id="submit">Submit</button><div id="result" role="status">Ready</div>
<script>
window.fixtureNonce=crypto.randomUUID();window.events=[];window.submitCount=0;
for(const kind of ['pointerdown','pointerup','click','keydown','keyup','beforeinput','input'])
  document.addEventListener(kind,e=>events.push({type:e.type,target:e.target.id,trusted:e.isTrusted}),true);
fetch('/ready',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({nonce:fixtureNonce})});
submit.onclick=()=>{if(window.confirmNext){window.confirmNext=false;window.lastConfirm=confirm('Submit this fixture?');if(!window.lastConfirm)return;}submitCount++;result.textContent=field.value;fetch('/witness',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({nonce:fixtureNonce,value:field.value,events})})};
</script></body>"#;

struct Model {
    atomic_input: bool,
    mode: AtomicUsize,
    phases: [AtomicUsize; 6],
    calls: AtomicUsize,
    tab: Mutex<String>,
    ready: Semaphore,
    ready_nonce: Mutex<Option<String>>,
    terminal: Semaphore,
    release_terminal: Semaphore,
    held: Semaphore,
    release_held: [Semaphore; 3],
    witnesses: Mutex<Vec<Value>>,
    failure: Mutex<Option<String>>,
    stop: CancellationToken,
}
fn text(mode: usize) -> &'static str {
    if mode == 2 {
        "resumed browser input"
    } else {
        "borrowed browser input"
    }
}
async fn ready(
    State(model): State<Arc<Model>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> StatusCode {
    if headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.contains(COOKIE))
    {
        *model.failure.lock().unwrap() =
            Some("The fixture did not establish its existing HTTP-only login before attach".into());
        return StatusCode::UNAUTHORIZED;
    }
    *model.ready_nonce.lock().unwrap() = body["nonce"].as_str().map(str::to_owned);
    model.ready.add_permits(1);
    StatusCode::NO_CONTENT
}
async fn witness(
    State(model): State<Arc<Model>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> StatusCode {
    if headers
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.contains(COOKIE))
    {
        *model.failure.lock().unwrap() =
            Some("Agent input did not use the browser's pre-existing fixture login".into());
        return StatusCode::UNAUTHORIZED;
    }
    model.witnesses.lock().unwrap().push(body);
    StatusCode::NO_CONTENT
}
fn observation(body: &Value) -> Result<Value, String> {
    let result = last_result(body)?;
    if result.get("observation_id").is_none() {
        return Err("last tool result was not a fresh observation".into());
    }
    Ok(result)
}
fn last_result(body: &Value) -> Result<Value, String> {
    let result = body["messages"]
        .as_array()
        .ok_or("model request missing messages")?
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .and_then(|message| {
            message["content"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
        })
        .ok_or("last real Tool result missing")?;
    result
        .get("result")
        .cloned()
        .ok_or_else(|| format!("real Tool did not succeed: {result}"))
}
fn action(
    body: &Value,
    tab: &str,
    operation: &str,
    label: &str,
    mode: usize,
) -> Result<Value, String> {
    let observed = observation(body)?;
    let reference = observed["elements"]
        .as_array()
        .ok_or("observation elements missing")?
        .iter()
        .find(|element| element["name"] == label)
        .and_then(|element| element["ref_id"].as_str())
        .ok_or("fixture reference missing")?;
    let mut result = json!({"operation":operation,"tab_id":tab,"observation_id":observed["observation_id"],"ref_id":reference});
    if operation == "type" {
        result["text"] = json!(text(mode));
        result["replace"] = json!(true);
    }
    Ok(result)
}
async fn completion(State(model): State<Arc<Model>>, Json(body): Json<Value>) -> Response {
    let mode = model.mode.load(Ordering::SeqCst);
    let step = model.phases[mode].fetch_add(1, Ordering::SeqCst);
    model.calls.fetch_add(1, Ordering::SeqCst);
    let operation = (|| -> Result<Option<Value>, String> {
        let serialized = body.to_string();
        if serialized.contains(PRIVATE_TAB)
            || serialized.contains("EXISTING_HTTP_ONLY_SESSION_SENTINEL")
        {
            return Err(
                "Unselected browser metadata or session credentials reached the model".into(),
            );
        }
        let tools = body["tools"].as_array().ok_or("missing model tools")?;
        if !tools
            .iter()
            .any(|tool| tool["function"]["name"] == "nomi_system_browser")
        {
            return Err("selected System Browser Tool was not registered".into());
        }
        if tools.iter().any(|tool| {
            matches!(
                tool["function"]["name"].as_str(),
                Some("Browser" | "nomi_local_websearch" | "web_search")
            )
        }) {
            return Err("system browser selection leaked another browser/search capability".into());
        }
        let tab = model.tab.lock().unwrap().clone();
        if matches!(mode, 4 | 5) {
            return match step {
                0 => Ok(Some(json!({"operation":"observe","tab_id":tab}))),
                1 => Ok(Some(action(&body, &tab, "click", "Submit", mode)?)),
                2 => {
                    let result = last_result(&body)?;
                    if result["tab_id"] != tab || result["script_dialog"]["kind"] != "confirm" {
                        return Err("real Submit action did not report the expected tab's confirm dialog".into());
                    }
                    let dialog_id = result["script_dialog"]["dialog_id"].as_str()
                        .filter(|id| !id.is_empty() && id.chars().count() <= 128)
                        .ok_or("real confirm dialog identity missing")?;
                    Ok(Some(json!({"operation":"dialog","tab_id":tab,"dialog_id":dialog_id,"accept":mode==5})))
                }
                3 if mode == 4 => {
                    let result = last_result(&body)?;
                    if result["completed"] != true || !result["script_dialog"].is_null() {
                        return Err("dialog reply did not settle its real browser action".into());
                    }
                    Ok(None)
                }
                _ => Err("dialog turn continued beyond its expected model phase".into()),
            };
        }
        if matches!(mode, 1 | 3) {
            return match step {
                0 => Ok(Some(json!({"operation":"observe","tab_id":tab}))),
                1 => Ok(Some(action(
                    &body,
                    &tab,
                    "click",
                    if model.atomic_input {
                        "Input"
                    } else {
                        "Submit"
                    },
                    mode,
                )?)),
                _ => Err("stopped turn continued to call the model".into()),
            };
        }
        match step {
            0 => Ok(Some(json!({"operation":"tabs"}))),
            1 => {
                let listed = last_result(&body)?;
                if listed["tabs"]
                    .as_array()
                    .is_none_or(|tabs| tabs.len() != 1 || tabs[0]["tab_id"] != tab)
                {
                    return Err("Tool Tabs did not contain exactly the authorized tab".into());
                }
                Ok(Some(json!({"operation":"observe","tab_id":tab})))
            }
            3 | 5 => Ok(Some(json!({"operation":"observe","tab_id":tab}))),
            2 => Ok(Some(action(&body, &tab, "type", "Input", mode)?)),
            4 => Ok(Some(action(&body, &tab, "click", "Submit", mode)?)),
            6 => {
                if !observation(&body)?["content"]
                    .as_str()
                    .is_some_and(|content| content.contains(text(mode)))
                {
                    return Err("model did not observe the real click result".into());
                }
                Ok(None)
            }
            _ => Err("unexpected model request".into()),
        }
    })();
    let operation = match operation {
        Ok(value) => value,
        Err(error) => {
            *model.failure.lock().unwrap() = Some(error);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if mode == 5 && step == 2 {
        model.held.add_permits(1);
        tokio::select! {_=model.stop.cancelled()=>return StatusCode::SERVICE_UNAVAILABLE.into_response(),permit=model.release_held[2].acquire()=>{if let Ok(permit)=permit{permit.forget();}}}
    } else if !model.atomic_input && matches!(mode, 1 | 3) && step == 1 {
        model.held.add_permits(1);
        tokio::select! {_=model.stop.cancelled()=>return StatusCode::SERVICE_UNAVAILABLE.into_response(),permit=model.release_held[usize::from(mode==3)].acquire()=>{if let Ok(permit)=permit{permit.forget();}}}
    }
    let (delta, finish) = match operation {
        Some(operation) => (
            json!({"role":"assistant","tool_calls":[{"index":0,"id":format!("system-{mode}-{step}"),"type":"function","function":{"name":"nomi_system_browser","arguments":operation.to_string()}}]}),
            "tool_calls",
        ),
        None => {
            model.terminal.add_permits(1);
            tokio::select! {_=model.stop.cancelled()=>return StatusCode::SERVICE_UNAVAILABLE.into_response(),permit=model.release_terminal.acquire()=>{if let Ok(permit)=permit{permit.forget();}}}
            (
                json!({"role":"assistant","content":"SYSTEM_BROWSER_DONE"}),
                "stop",
            )
        }
    };
    let chunk = |delta: Value, finish: Option<&str>| {
        json!({"id":format!("system-{mode}-{step}"),"object":"chat.completion.chunk","created":1,"model":"system-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
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
async fn api(
    server: &DesktopServer,
    method: &str,
    path: &str,
    body: Value,
) -> Result<Value, String> {
    let response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
        .request(
            method.parse().unwrap(),
            format!("http://127.0.0.1:{}{path}", server.loopback_port()),
        )
        .header("x-nomi-local-trust", server.local_trust_secret())
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("{method} {path}: {status} {value}"));
    }
    Ok(value["data"].clone())
}
async fn permit(model: &Model, semaphore: &Semaphore, label: &str) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(40), semaphore.acquire())
        .await
        .map_err(|_| {
            format!(
                "timeout {label}; calls={}; failure={:?}",
                model.calls.load(Ordering::SeqCst),
                model.failure.lock().unwrap()
            )
        })?
        .map_err(|error| error.to_string())?
        .forget();
    Ok(())
}
async fn finished(server: &DesktopServer, id: &str) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if api(
                server,
                "GET",
                &format!("/api/conversations/{id}"),
                Value::Null,
            )
            .await?["status"]
                == "finished"
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| "conversation did not reach terminal".to_owned())?
}
async fn turn(server: &DesktopServer, id: &str, prompt: &str) -> Result<(), String> {
    api(
        server,
        "POST",
        &format!("/api/agent-sessions/{id}/turns"),
        json!({"input":{"content":prompt},"idempotency_key":uuid::Uuid::new_v4().to_string()}),
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit NOMIFUN_CHROME_BINARY; owns a visible disposable browser and backend"]
async fn system_browser_main_application_model_roundtrip() {
    roundtrip(false).await;
}

#[tokio::test]
#[ignore = "requires explicit NOMIFUN_CHROME_BINARY; real in-flight mouse input, owned visible Chrome"]
async fn system_browser_main_application_atomic_input_settles() {
    roundtrip(true).await;
}

async fn roundtrip(atomic_input: bool) {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("owned-chrome-profile");
    let launched = nomi_browser_engine::launch::launch_chrome(
        &nomi_browser_engine::launch::LaunchConfig {
            chrome_path: std::env::var_os("NOMIFUN_CHROME_BINARY")
                .expect("explicit fixture Chrome")
                .into(),
            user_data_dir: profile.clone(),
            headful: true,
        },
        false,
    )
    .await
    .unwrap();
    let (mut browser_owner, diagnostics) = launched.connect().await.unwrap();
    let relay = if atomic_input {
        Some(input_proxy::InputProxy::start(root.path(), &profile.join("DevToolsActivePort")).await)
    } else {
        None
    };
    let proxy = relay.as_ref().map(|(proxy, _)| proxy.clone());
    let model = Arc::new(Model {
        atomic_input,
        mode: AtomicUsize::new(0),
        phases: std::array::from_fn(|_| AtomicUsize::new(0)),
        calls: AtomicUsize::new(0),
        tab: Mutex::new(String::new()),
        ready: Semaphore::new(0),
        ready_nonce: Mutex::new(None),
        terminal: Semaphore::new(0),
        release_terminal: Semaphore::new(0),
        held: Semaphore::new(0),
        release_held: std::array::from_fn(|_| Semaphore::new(0)),
        witnesses: Mutex::new(vec![]),
        failure: Mutex::new(None),
        stop: CancellationToken::new(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/fixture");
    let fixture = Router::new()
        .route(
            "/fixture",
            get(|| async {
                (
                    [(
                        "set-cookie",
                        format!("{COOKIE}; HttpOnly; SameSite=Lax; Path=/"),
                    )],
                    axum::response::Html(HTML),
                )
            }),
        )
        .route(
            "/private",
            get(|| async {
                axum::response::Html(format!("<title>{PRIVATE_TAB}</title><p>{PRIVATE_TAB}</p>"))
            }),
        )
        .route("/ready", post(ready))
        .route("/witness", post(witness))
        .route("/v1/chat/completions", post(completion))
        .with_state(model.clone());
    let stop = model.stop.clone();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, fixture)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
    });
    let factory = conformance_connection_factory(proxy.as_ref().map_or_else(
        || profile.join("DevToolsActivePort"),
        |proxy| proxy.port_file.clone(),
    ));
    let factory = proxy.as_ref().map_or_else(
        || factory.clone(),
        |proxy| proxy.observing_factory(factory.clone()),
    );
    let system = SystemBrowserService::with_factory(factory);
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
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
    let (app, keep_alive) = DesktopServer::start_with_outcome(
        &cli,
        "",
        None,
        None,
        None,
        DesktopHostServices {
            system_browser: Some(system),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let result=std::panic::AssertUnwindSafe(async {
        let fixture_target=diagnostics.send("",&CreateTargetParams::new(url.clone())).await.unwrap()["targetId"].as_str().unwrap().to_owned();
        diagnostics.send("",&CreateTargetParams::new(format!("http://{address}/private"))).await.unwrap();
        permit(&model,&model.ready,"browser pre-existing session").await?;
        let nonce=model.ready_nonce.lock().unwrap().clone().ok_or("fixture nonce missing")?;
        let mut attach=AttachToTargetParams::new(fixture_target.clone());attach.flatten=Some(true);
        let attached=diagnostics.send("",&attach).await.map_err(|error|error.to_string())?;
        let inspection_session=attached["sessionId"].as_str().ok_or("fixture inspection session missing")?.to_owned();
        diagnostics.registry().register_session(&inspection_session,"page");
        let provider=api(&app,"POST","/api/providers",json!({"platform":"custom","name":"System browser fixture","base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":["local-fixture-not-a-secret"]},"enabled":true,"initial_model":{"model":"system-fixture","enabled":true,"capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider=provider["provider_id"].as_str().ok_or("provider id missing")?;
        let editor=api(&app,"POST","/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"System browser fixture","model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"system-fixture"}})).await?;
        let preset=editor["preset"]["preset_id"].as_str().ok_or("preset id missing")?;
        let catalog=api(&app,"GET","/api/capabilities",Value::Null).await?;
        let capability=catalog.as_array().ok_or("catalog shape")?.iter().find(|item|item["capability"]["id"]=="nomi_system_browser").ok_or("system browser capability missing")?;
        if capability["materialization_state"]!="materialized" {return Err(format!("system browser is not available: {capability}"));}
        let mut draft=editor["draft"].clone();draft["document"]["enabled_capabilities"]=json!([{"capability":capability["capability"],"action_allowlist":[]}]);draft["document"]["skill_bindings"]=json!([]);
        api(&app,"POST",&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"system browser conformance"})).await?;
        let session=api(&app,"POST","/api/agent-sessions",json!({"preset_id":preset,"title":"System browser conformance","model":{"provider_id":provider,"model":"system-fixture"}})).await?;
        let id=session["agent_session_id"].as_str().ok_or("session id missing")?;
        api(&app,"PATCH",&format!("/api/conversations/{id}"),json!({"extra":{"workspace":work.to_string_lossy()}})).await?;
        let base=format!("/api/conversations/{id}/system-browser");
        let connected=api(&app,"POST",&base,json!({})).await?;let incarnation=connected["incarnation"].as_str().ok_or("connection incarnation")?;
        let choice_deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        let choices=loop {
            let choices=api(&app,"POST",&format!("{base}/choices"),json!({"incarnation":incarnation})).await?;
            if choices["tabs"].as_array().is_some_and(|tabs|tabs.iter().any(|tab|tab["title"]==PRIVATE_TAB&&tab["url"]==format!("http://{address}/private"))) {break choices;}
            if tokio::time::Instant::now()>=choice_deadline {return Err("user chooser did not contain the unselected privacy sentinel tab".into());}
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let choice=choices["tabs"].as_array().ok_or("chooser tabs")?.iter().find(|tab|tab["url"]==url).ok_or("fixture choice missing")?;
        let grant=api(&app,"POST",&format!("{base}/tabs"),json!({"incarnation":incarnation,"choice_id":choice["choice_id"]})).await?;
        if grant["tabs"].as_array().is_none_or(|tabs|tabs.len()!=1||tabs[0]["url"]!=url)||grant.to_string().contains(PRIVATE_TAB) {return Err("user grant expanded beyond the selected tab".into());}
        let tab=grant["tabs"][0]["tab_id"].as_str().ok_or("granted tab missing")?.to_owned();*model.tab.lock().unwrap()=tab;
        turn(&app,id,"Use the authorized browser tab: observe, type and submit the fixture.").await?;
        permit(&model,&model.terminal,"first model terminal").await?;
        if api(&app,"GET",&format!("/api/conversations/{id}"),Value::Null).await?["status"]!="running" {return Err("turn became terminal before model completion".into());}
        if api(&app,"DELETE",&base,json!({"incarnation":incarnation})).await.is_ok() {return Err("configuration changed during an active Agent run".into());}
        model.release_terminal.add_permits(1);finished(&app,id).await?;
        // Only fixture conditions are set through this independent diagnostics
        // session; the model's Click/Dialog operations use the production Tool.
        arm_confirm(&diagnostics,&inspection_session).await?;
        model.mode.store(4,Ordering::SeqCst);
        turn(&app,id,"Click Submit and decline the fixture confirmation using its returned dialog identity.").await?;
        permit(&model,&model.terminal,"dialog reply model terminal").await?;
        require_rejected_confirm(&diagnostics,&inspection_session).await?;
        if model.witnesses.lock().unwrap().len()!=1 {return Err("declining the modal emitted an extra submission witness".into());}
        model.release_terminal.add_permits(1);finished(&app,id).await?;
        arm_confirm(&diagnostics,&inspection_session).await?;
        model.mode.store(5,Ordering::SeqCst);
        turn(&app,id,"Click Submit; this run will be stopped while the confirmation reply is pending.").await?;
        permit(&model,&model.held,"late confirm acceptance held by model").await?;
        api(&app,"POST",&format!("/api/conversations/{id}/cancel"),json!({})).await?;
        finished(&app,id).await?;
        require_rejected_confirm(&diagnostics,&inspection_session).await?;
        if model.witnesses.lock().unwrap().len()!=1 {return Err("Stop failed to reject the modal without submitting".into());}
        model.release_held[2].add_permits(1);
        model.mode.store(1,Ordering::SeqCst);
        if let Some(proxy)=&proxy {proxy.arm();}
        turn(&app,id,"Observe and click; this turn will be cancelled.").await?;
        if let Some(proxy)=&proxy {
            permit(&model,&proxy.pressed,"real mouse down acknowledgement").await?;
            require_pointer_state(&diagnostics,&inspection_session,true).await?;
            let active_cancel=proxy.active_cancel();
            if active_cancel.is_cancelled() {return Err("input already cancelled before Stop request".into());}
            let cancel_path=format!("/api/conversations/{id}/cancel");
            let mut cancel=Box::pin(api(&app,"POST",&cancel_path,json!({})));
            let mut early=None;
            tokio::time::timeout(Duration::from_secs(5),async {
                tokio::select! {
                    _=active_cancel.cancelled()=>{},
                    result=&mut cancel=>{early=Some(result);active_cancel.cancelled().await;},
                }
            }).await.map_err(|_|"Stop did not reach the real in-flight invocation")?;
            require_running(&app,id).await?;
            if proxy.events()!=["down"] {return Err(format!("input advanced before down ACK: {:?}",proxy.events()));}
            proxy.allow_pressed.add_permits(1);
            permit(&model,&proxy.released,"real mouse up acknowledgement").await?;
            require_pointer_state(&diagnostics,&inspection_session,false).await?;
            require_running(&app,id).await?;
            if proxy.events()!=["down","down_ack","up"] {return Err("run detached before mouse release was acknowledged".into());}
            proxy.allow_released.add_permits(1);
            match early {Some(result)=>{result?;},None=>{cancel.await?;}}
            finished(&app,id).await?;
            require_settled_input(proxy)?;
        } else {
            permit(&model,&model.held,"held late model action").await?;
            api(&app,"POST",&format!("/api/conversations/{id}/cancel"),json!({})).await?;finished(&app,id).await?;
            model.release_held[0].add_permits(1);
        }
        model.mode.store(2,Ordering::SeqCst);turn(&app,id,"Observe afresh, replace the text and submit on the same page.").await?;
        permit(&model,&model.terminal,"resumed model terminal").await?;
        {
            let witnesses=model.witnesses.lock().unwrap();
            if witnesses.len()!=2 {return Err(format!("expected exactly two real submissions, got {}",witnesses.len()));}
            let mut previous_events=0;
            for (index,proof) in witnesses.iter().enumerate() {
                if proof["nonce"]!=nonce||proof["value"]!=text(if index==0{0}else{2}) {return Err("browser document or input did not survive turn boundaries".into());}
                let events=proof["events"].as_array().ok_or("browser input events missing")?;
                let recent=events.get(previous_events..).ok_or("event history was replaced between turns")?;
                if !recent.iter().all(|event|event["trusted"]==true)||!recent.iter().any(|event|event["type"]=="input"&&event["target"]=="field")||!recent.iter().any(|event|event["type"]=="click"&&event["target"]=="submit") {return Err("this turn did not produce fresh native trusted input".into());}
                previous_events=events.len();
            }
        }
        model.release_terminal.add_permits(1);finished(&app,id).await?;
        let messages=api(&app,"GET",&format!("/api/agent-sessions/{id}/messages?after_seq=0&limit=100"),Value::Null).await?;
        if !messages.to_string().contains("SYSTEM_BROWSER_DONE")||!messages.to_string().contains("nomi_system_browser") {return Err("normal persisted history omitted system-browser tool/result".into());}
        model.mode.store(3,Ordering::SeqCst);
        if let Some(proxy)=&proxy {proxy.arm();}
        turn(&app,id,"Observe and click; the application will shut down.").await?;
        if let Some(proxy)=&proxy {
            permit(&model,&proxy.pressed,"shutdown mouse down acknowledgement").await?;
            require_pointer_state(&diagnostics,&inspection_session,true).await?;
            let active_cancel=proxy.active_cancel();
            if active_cancel.is_cancelled() {return Err("input already cancelled before shutdown".into());}
            let mut shutdown=Box::pin(app.shutdown_all());
            tokio::time::timeout(Duration::from_secs(5),async {
                tokio::select! {
                    _=active_cancel.cancelled()=>Ok(()),
                    result=&mut shutdown=>Err(format!("shutdown completed before cancelling in-flight input: {result:?}")),
                }
            }).await.map_err(|_|"shutdown did not cancel the real in-flight invocation")??;
            if tokio::time::timeout(Duration::from_millis(150),&mut shutdown).await.is_ok() {return Err("application exited with mouse still pressed".into());}
            if proxy.events()!=["down"] {return Err("shutdown detached before input settled".into());}
            proxy.allow_pressed.add_permits(1);
            // Continue polling shutdown concurrently: stopping its future is
            // not evidence that the service deliberately waited for release.
            tokio::select! {
                result=&mut shutdown=>return Err(format!("shutdown completed before mouse release ACK: {result:?}")),
                result=permit(&model,&proxy.released,"shutdown mouse up acknowledgement")=>result?,
            }
            require_pointer_state(&diagnostics,&inspection_session,false).await?;
            if tokio::time::timeout(Duration::from_millis(150),&mut shutdown).await.is_ok() {return Err("application exited before release was acknowledged".into());}
            if proxy.events()!=["down","down_ack","up"] {return Err("shutdown detached before release proof".into());}
            proxy.allow_released.add_permits(1);
            shutdown.await.map_err(|error|error.to_string())?;
            require_settled_input(proxy)?;
        } else {
            permit(&model,&model.held,"model at application shutdown").await?;
            app.shutdown_all().await.map_err(|error|error.to_string())?;model.release_held[1].add_permits(1);
        }
        if browser_owner.child_mut().try_wait().unwrap().is_some() {return Err("application shutdown closed the borrowed browser".into());}
        diagnostics.send("",&GetVersionParams::default()).await.unwrap();
        let targets=diagnostics.send("",&GetTargetsParams::default()).await.unwrap();
        if !targets["targetInfos"].as_array().is_some_and(|targets|targets.iter().any(|target|target["targetId"]==fixture_target)) {return Err("application shutdown closed the borrowed tab".into());}
        let mut inspect=EvaluateParams::new("JSON.stringify({nonce:fixtureNonce,count:submitCount,value:field.value})");inspect.return_by_value=Some(true);
        let inspected=diagnostics.send(&inspection_session,&inspect).await.map_err(|error|error.to_string())?;
        let final_page:Value=serde_json::from_str(inspected["result"]["value"].as_str().ok_or("fixture final page missing")?).map_err(|error|error.to_string())?;
        if final_page["nonce"]!=nonce||final_page["count"]!=2||final_page["value"]!=text(2) {return Err("shutdown or late model response changed the existing page".into());}
        if let Some(error)=model.failure.lock().unwrap().clone() {return Err(error);}
        Ok::<(),String>(())
    }).catch_unwind().await;
    model.stop.cancel();
    model.release_terminal.add_permits(1);
    for held in &model.release_held {
        held.add_permits(1);
    }
    if let Some(proxy) = &proxy {
        proxy.allow_pressed.add_permits(1);
        proxy.allow_released.add_permits(1);
    }
    let app_cleanup = app.shutdown_all().await;
    drop(app);
    drop(keep_alive);
    let fixture_cleanup = tokio::time::timeout(Duration::from_secs(10), server_task).await;
    let proxy_cleanup = if let Some((proxy, task)) = relay {
        proxy.stop();
        Some(tokio::time::timeout(Duration::from_secs(10), task).await)
    } else {
        None
    };
    diagnostics.shutdown().await;
    let browser_cleanup = browser_owner.shutdown().await;
    drop(diagnostics);
    drop(browser_owner);
    let root_cleanup = root.close();
    let result = result.unwrap();
    let fixture_ok = matches!(fixture_cleanup, Ok(Ok(Ok(()))));
    let proxy_ok = matches!(proxy_cleanup, None | Some(Ok(Ok(Ok(())))));
    assert!(
        result.is_ok()
            && root_cleanup.is_ok()
            && app_cleanup.is_ok()
            && fixture_ok
            && proxy_ok
            && browser_cleanup.is_ok(),
        "pipeline={result:?}; app cleanup={app_cleanup:?}; fixture cleanup={fixture_cleanup:?}; proxy cleanup={proxy_cleanup:?}; browser cleanup={browser_cleanup:?}; temporary cleanup={root_cleanup:?}"
    );
    assert_eq!(
        model.witnesses.lock().unwrap().len(),
        2,
        "a late request submitted after final checks"
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 25, "unexpected extra or missing model phase");
    assert_eq!(model.phases.iter().map(|phase|phase.load(Ordering::SeqCst)).collect::<Vec<_>>(), [7,2,7,2,4,3]);
    println!(
        "SYSTEM_BROWSER_MAIN_APP_PASS model_calls={} atomic_input={atomic_input} existing_fixture_login=true trusted_input=true unselected_tab_private=true dialog_reply=true stop_rejects_dialog=true stop_and_resume=true app_shutdown_preserves_browser=true temporary_cleanup=true",
        model.calls.load(Ordering::SeqCst)
    );
}

async fn arm_confirm(
    connection: &nomi_browser_engine::transport::Connection,
    session: &str,
) -> Result<(), String> {
    let mut params = EvaluateParams::new("window.confirmNext=true;window.lastConfirm=null;true");
    params.return_by_value = Some(true);
    let reply = connection.send(session, &params).await.map_err(|error| error.to_string())?;
    if reply.get("exceptionDetails").is_some() || reply["result"]["value"] != true {
        return Err("could not arm the fixture's one-shot confirm condition".into());
    }
    Ok(())
}

async fn require_rejected_confirm(
    connection: &nomi_browser_engine::transport::Connection,
    session: &str,
) -> Result<(), String> {
    let mut params = EvaluateParams::new("JSON.stringify({lastConfirm:window.lastConfirm,count:submitCount,armed:window.confirmNext})");
    params.return_by_value = Some(true);
    let reply = connection.send(session, &params).await.map_err(|error| error.to_string())?;
    let proof: Value = serde_json::from_str(reply["result"]["value"].as_str().ok_or("confirm result witness missing")?)
        .map_err(|error| error.to_string())?;
    if proof["lastConfirm"] != false || proof["count"] != 1 || proof["armed"] != false {
        return Err(format!("real fixture confirm was not rejected without submitting: {proof}"));
    }
    Ok(())
}

async fn require_running(app: &DesktopServer, id: &str) -> Result<(), String> {
    if api(app, "GET", &format!("/api/conversations/{id}"), Value::Null).await?["status"]
        != "running"
    {
        return Err("turn published terminal before atomic input was acknowledged".into());
    }
    Ok(())
}

async fn require_pointer_state(
    connection: &nomi_browser_engine::transport::Connection,
    session: &str,
    down: bool,
) -> Result<(), String> {
    let mut inspect = EvaluateParams::new(
        "JSON.stringify(events.filter(e=>e.type==='pointerdown'||e.type==='pointerup'||e.type==='click').slice(-3))",
    );
    inspect.return_by_value = Some(true);
    let reply = connection
        .send(session, &inspect)
        .await
        .map_err(|error| error.to_string())?;
    let events: Value = serde_json::from_str(
        reply["result"]["value"]
            .as_str()
            .ok_or("missing pointer witness")?,
    )
    .map_err(|error| error.to_string())?;
    let events = events.as_array().ok_or("missing pointer events")?;
    let expected = if down {
        vec!["pointerdown"]
    } else {
        vec!["pointerdown", "pointerup", "click"]
    };
    let tail = events
        .get(events.len().saturating_sub(expected.len())..)
        .ok_or("missing native pointer tail")?;
    if tail.len() != expected.len()
        || tail.iter().zip(expected).any(|(event, kind)| {
            event["type"] != kind || event["target"] != "field" || event["trusted"] != true
        })
    {
        return Err(format!("unexpected physical pointer state: {events:?}"));
    }
    Ok(())
}

fn require_settled_input(proxy: &input_proxy::InputProxy) -> Result<(), String> {
    if proxy.events() != ["down", "down_ack", "up", "up_ack", "detach"] {
        return Err(format!(
            "atomic input not released exactly once before detach: {:?}",
            proxy.events()
        ));
    }
    Ok(())
}

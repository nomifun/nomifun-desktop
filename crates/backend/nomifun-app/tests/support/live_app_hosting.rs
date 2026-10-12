//! Opt-in application page → fixed built-in Agent → real workspace smoke.
//! The project is written only by the real provider through production tools.
//! Persisted evidence contains identities/digests and generated public files;
//! the disposable database, Provider credential and LocalTrust are not retained.

use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use axum::{extract::State, routing::post};
use axum::response::IntoResponse;
use sha2::{Digest, Sha256};

const ARTIFACT_ENV: &str = "NOMIFUN_LIVE_APP_HOSTING_ARTIFACT_DIR";
const HOLD_ENV: &str = "NOMIFUN_LIVE_APP_HOSTING_BROWSER_HOLD";
// Public, disposable fixture credentials. Never used for a user's installation.
const WEB_TEST_USERNAME: &str = "app-workspace-smoke";
const WEB_TEST_PASSWORD: &str = "NomiFunAppSmoke2026!";
const GENERATION_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Default)]
struct UpstreamFailureProbe { active: AtomicUsize, responses: AtomicUsize }

fn fail(phase: &'static str, code: &'static str) -> SmokeFailure {
    SmokeFailure::new(phase, code, 422)
}

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn process_exists(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true, sysinfo::ProcessRefreshKind::nothing());
    system.process(sysinfo::Pid::from_u32(pid)).is_some()
}

fn artifact_root() -> Result<Option<PathBuf>, SmokeFailure> {
    let Some(value) = std::env::var_os(ARTIFACT_ENV) else { return Ok(None); };
    let path = PathBuf::from(value);
    if !path.is_absolute() || path.exists() || path.parent().is_none_or(|parent| !parent.is_dir()) {
        return Err(fail("app_hosting.artifacts", "APP_HOSTING_ARTIFACT_DIRECTORY_INVALID"));
    }
    std::fs::create_dir(&path).map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_ARTIFACT_CREATE_FAILED"))?;
    Ok(Some(path))
}

fn write_evidence(root: Option<&Path>, file: &str, evidence: &Value) -> Result<(), SmokeFailure> {
    if let Some(root) = root {
        let data = serde_json::to_vec_pretty(evidence).map_err(|_|
            fail("app_hosting.artifacts", "APP_HOSTING_EVIDENCE_SERIALIZATION_FAILED"))?;
        std::fs::write(root.join(file), data).map_err(|_|
            fail("app_hosting.artifacts", "APP_HOSTING_EVIDENCE_WRITE_FAILED"))?;
    }
    Ok(())
}

fn copy_generated_files(source: &Path, destination: &Path, secrets: &[&[u8]]) -> Result<Value, SmokeFailure> {
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    let mut visited = 0;
    while let Some((source, destination)) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&source)
            .map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILE_UNREADABLE"))?;
        visited += 1;
        if visited > 256 { return Err(fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILES_TOO_LARGE")); }
        let mut link = metadata.file_type().is_symlink();
        #[cfg(windows)] {
            use std::os::windows::fs::MetadataExt as _;
            link |= metadata.file_attributes() & 0x400 != 0;
        }
        if link {
            return Err(fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_LINK_REJECTED"));
        }
        if metadata.is_dir() {
            std::fs::create_dir_all(&destination)
                .map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_ARTIFACT_CREATE_FAILED"))?;
            for entry in std::fs::read_dir(source)
                .map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILE_UNREADABLE"))? {
                let entry = entry.map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILE_UNREADABLE"))?;
                let name = entry.file_name();
                if matches!(name.to_str(), Some("node_modules" | ".git")) { continue; }
                pending.push((entry.path(), destination.join(name)));
            }
        } else if metadata.is_file() {
            total_bytes = total_bytes.saturating_add(metadata.len());
            if total_bytes > 8 * 1024 * 1024 || files.len() >= 128 {
                return Err(fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILES_TOO_LARGE"));
            }
            let bytes = std::fs::read(&source)
                .map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILE_UNREADABLE"))?;
            if secrets.iter().any(|secret| !secret.is_empty() && bytes.windows(secret.len()).any(|window| window == *secret)) {
                return Err(fail("app_hosting.artifacts", "PLAINTEXT_CREDENTIAL_PERSISTED"));
            }
            std::fs::write(&destination, &bytes)
                .map_err(|_| fail("app_hosting.artifacts", "APP_HOSTING_GENERATED_FILE_COPY_FAILED"))?;
            files.push(json!({"name": destination.file_name().and_then(|name| name.to_str()),
                "bytes":bytes.len(), "sha256":digest(&bytes)}));
        }
    }
    Ok(json!(files))
}

/// Join the committed dispatch, result and exact durable effect by operation ID.
/// Model text and Message projections are not evidence of import/start success.
fn settlements(events: &[Value], secrets: &[&str]) -> Result<Vec<Value>, SmokeFailure> {
    let starts: BTreeMap<&str, &Value> = events.iter()
        .filter(|event| event["kind"] == "tool/call-started")
        .filter_map(|event| event.pointer("/payload/value/operation_id").and_then(Value::as_str)
            .map(|operation| (operation, event))).collect();
    let mut evidence = Vec::new();
    for result in events.iter().filter(|event| event["kind"] == "tool/result-recorded") {
        let payload = &result["payload"]["value"];
        let Some(operation) = payload["operation_id"].as_str() else { continue; };
        let Some(start) = starts.get(operation) else { continue; };
        let Some(action) = start.pointer("/payload/value/action_id").and_then(Value::as_str) else { continue; };
        if !matches!(action, "workspace.files/write" | "workspace.files/patch" |
            "app.hosting/import" | "app.hosting/update" | "app.hosting/preview" | "app.hosting/rebuild" |
            "app.hosting/start" | "app.hosting/stop" | "app.hosting/status" | "app.hosting/list" | "app.hosting/current" |
            "app.hosting/guide") { continue; }
        let native = serde_json::from_value::<nomifun_agent_runtime::AgentToolResult>(payload["output"].clone()).ok();
        let successful = payload["error"].is_null() && native.as_ref().is_some_and(|result| !result.is_error
            && start.pointer("/payload/value/call_id").and_then(Value::as_str) == Some(result.call_id.as_ref()));
        let output = native.as_ref().and_then(|result| serde_json::from_str::<Value>(&result.output_text()).ok());
        let diagnostic = payload["error"].as_str().map(|error| {
            let mut error = nomi_redact::redact_secrets_owned(error.to_owned());
            for secret in secrets { error = error.replace(secret, "[redacted]"); }
            error.chars().take(512).collect::<String>()
        });
        let import_path = if action == "app.hosting/import" {
            events.iter().filter(|event| event["kind"] == "runtime/progress-recorded")
                .filter_map(|event| event.pointer("/payload/value/event"))
                .find(|event| event["event"] == "tool_call_completed"
                    && event.pointer("/call/call_id") == start.pointer("/payload/value/call_id"))
                .and_then(|event| event.pointer("/call/arguments/path")).and_then(Value::as_str)
                .map(|path| {
                    let mut path = path.to_owned();
                    for secret in secrets { path = path.replace(secret, "[redacted]"); }
                    path.chars().take(4096).collect::<String>()
                })
        } else { None };
        let effect = events.iter().find(|event| event["kind"] == "effect/succeeded"
            && event.pointer("/payload/value/operation_id").and_then(Value::as_str) == Some(operation)
            && event.pointer("/payload/value/action_id").and_then(Value::as_str) == Some(action)
            && event["seq"].as_u64().is_some_and(|seq| seq > start["seq"].as_u64().unwrap_or(u64::MAX)
                && seq < result["seq"].as_u64().unwrap_or(0))
            && events.iter().any(|began| began["kind"] == "effect/started"
                && began["event_id"] == event["causation_event_id"]
                && began["causation_event_id"] == start["event_id"]
                && began.pointer("/payload/value/effect_id") == event.pointer("/payload/value/effect_id")
                && began.pointer("/payload/value/operation_id").and_then(Value::as_str) == Some(operation)));
        evidence.push(json!({"action_id":action,"operation_id":operation,"dispatch_seq":start["seq"],
            "result_seq":result["seq"], "result_success":successful, "result_sha256":digest(
                &serde_json::to_vec(&payload["output"]).map_err(|_| fail("app_hosting.events", "APP_HOSTING_OUTPUT_DIGEST_FAILED"))?),
            "effect_succeeded_seq":effect.map(|event|event["seq"].clone()),
            "app_id":output.as_ref().and_then(|value|value.pointer("/application/id")),
            "application_state":output.as_ref().and_then(|value|value.pointer("/application/state")),
            "native_result_is_error":native.as_ref().map(|result|result.is_error),
            "diagnostic":diagnostic,"import_path":import_path}));
    }
    Ok(evidence)
}

async fn wait_for_turn(router: &Router, session: &str, operation: &str) -> Result<Vec<Value>, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + GENERATION_DEADLINE;
    loop {
        let observed = envelope_data("app_hosting.turn", successful_json(router, "app_hosting.turn", Method::GET,
            format!("/api/agent-sessions/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
        if observed.pointer("/head/status").and_then(Value::as_str) == Some("ready") {
            let events = session_events(router, session).await?;
            let terminal = events.iter().rev().find(|event| event["correlation_id"] == operation && matches!(event["kind"].as_str(),
                Some("turn/completed" | "turn/failed" | "turn/cancelled")));
            if terminal.is_some_and(|event| event["kind"] == "turn/completed") { return Ok(events); }
            if let Some(terminal) = terminal {
                let code = terminal.pointer("/payload/value/error/code").and_then(Value::as_str)
                    .unwrap_or("APP_HOSTING_GENERATION_NOT_COMPLETED");
                return Err(SmokeFailure::new("app_hosting.turn", code, 422));
            }
        }
        if observed.pointer("/head/status").and_then(Value::as_str) == Some("paused") {
            return Err(fail("app_hosting.turn", "APP_HOSTING_GENERATION_PAUSED"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(fail("app_hosting.turn", "APP_HOSTING_GENERATION_DEADLINE_EXCEEDED"));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn browser_result(State(state): State<Arc<AtomicUsize>>, axum::Json(payload): axum::Json<Value>) -> StatusCode {
    // This is a test-only callback for independent trusted browser QA. No DOM
    // or storage is changed by the fixture itself.
    let passed = payload["increment"] == true && payload["refresh_persisted"] == true && payload["clear"] == true
        && payload["connected"] == true;
    state.store(if passed { 1 } else { 2 }, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

async fn independent_browser_qa(state: &AtomicUsize, app_url: &str, host_url: &str, session: &str,
    app_id: &str, stage: &str) -> Result<Value,SmokeFailure> {
    if std::env::var(HOLD_ENV).as_deref() != Ok("1") { return Ok(Value::Null); }
    state.store(0,Ordering::SeqCst);
    let ready = json!({"app_url":app_url,"host_url":host_url,"session_id":session,"app_id":app_id,"stage":stage,
        "web_test_username":WEB_TEST_USERNAME,"qa_callback":format!("{host_url}/__app-hosting-smoke/ui-state")});
    eprintln!("NOMIFUN_LIVE_APP_HOSTING_READY {ready}");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    hard_deadline("app_hosting.browser", "APP_HOSTING_BROWSER_QA_DEADLINE_EXCEEDED", Duration::from_secs(900), async {
        loop {
            match state.load(Ordering::SeqCst) {
                1 => return Ok(()),
                2 => return Err(fail("app_hosting.browser", "APP_HOSTING_BROWSER_QA_FAILED")),
                _ => tokio::time::sleep(POLL_INTERVAL).await,
            }
        }
    }).await?;
    Ok(json!({"independent_callback":true,"increment":true,"refresh_persisted":true,"clear":true,"connected":true}))
}

async fn initialize_web_test_account(router: &Router) -> Result<(),SmokeFailure> {
    let owner_before = successful_json(router,"app_hosting.web_auth",Method::GET,"/api/auth/user",None,
        LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    let owner_id = required_string("app_hosting.web_auth",&owner_before,"/user/user_id","APP_HOSTING_AUTH_OWNER_MISSING")?;
    let setup = successful_json(router,"app_hosting.web_auth",Method::POST,"/api/auth/setup",
        Some(json!({"username":WEB_TEST_USERNAME,"password":WEB_TEST_PASSWORD})),LOCAL_API_DEADLINE,&[StatusCode::OK]).await?;
    if setup["success"] != true || setup.pointer("/user/user_id").and_then(Value::as_str) != Some(owner_id.as_str()) {
        return Err(fail("app_hosting.web_auth","APP_HOSTING_SETUP_CHANGED_INSTALLATION_OWNER"));
    }
    Ok(())
}

async fn finish_ui_fixture(State(state): State<Arc<AtomicUsize>>, axum::Json(payload): axum::Json<Value>) -> StatusCode {
    if payload["finished"] != true { return StatusCode::BAD_REQUEST; }
    state.store(1,Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

/// Boot real production APIs for independent UI acceptance. Readiness is not
/// a product pass: this fixture only proves startup and explicit shutdown.
pub(super) async fn run_ui_fixture(router: &Router, application: &NomiCoreApplication, api_key: &str, model: &str) -> Result<(),SmokeFailure> {
    initialize_web_test_account(router).await?;
    let provider = configure_stepfun(router,api_key,STEPFUN_PLAN_BASE_URL,model).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|_|fail("ui_fixture.bootstrap","UI_FIXTURE_LISTENER_FAILED"))?;
    let address = listener.local_addr().map_err(|_|fail("ui_fixture.bootstrap","UI_FIXTURE_ADDRESS_FAILED"))?;
    application.publish_app_hosting_endpoint(address.port(),"127.0.0.1").await;
    let state = Arc::new(AtomicUsize::new(0));
    let served = application.router().merge(Router::new().route("/__app-workspace-fixture/finish",post(finish_ui_fixture)).with_state(state.clone()));
    let server = tokio::spawn(async move { axum::serve(listener,served).await });
    let host_url = format!("http://{address}");
    let ready = json!({"host_url":host_url,"finish_url":format!("{host_url}/__app-workspace-fixture/finish"),
        "web_test_username":WEB_TEST_USERNAME,"provider_id":provider,"model":model,"validation":null});
    eprintln!("NOMIFUN_LIVE_APP_WORKSPACE_UI_READY {ready}");
    use std::io::Write; let _ = std::io::stderr().flush();
    let finished = hard_deadline("ui_fixture.lifecycle","UI_FIXTURE_EXPLICIT_FINISH_DEADLINE_EXCEEDED",Duration::from_secs(1800),async {
        loop { if state.load(Ordering::SeqCst) == 1 { return Ok(()); } tokio::time::sleep(POLL_INTERVAL).await; }
    }).await;
    server.abort(); let _ = server.await;
    finished?;
    eprintln!("NOMIFUN_LIVE_APP_WORKSPACE_UI_CLOSED validation=null");
    Ok(())
}

pub(super) async fn run(router: &Router, application: &NomiCoreApplication, api_key: &str, local_proof: &str, model: &str) -> Result<(), SmokeFailure> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await
        .map_err(|_| fail("app_hosting.host", "APP_HOSTING_HOST_LISTENER_FAILED"))?;
    let address = listener.local_addr().map_err(|_| fail("app_hosting.host", "APP_HOSTING_HOST_ADDRESS_FAILED"))?;
    application.publish_app_hosting_endpoint(address.port(), "127.0.0.1").await;
    let state = Arc::new(AtomicUsize::new(0));
    let host_calls = Arc::new(AtomicUsize::new(0));
    let failure_probe = Arc::new(UpstreamFailureProbe::default());
    let captured = host_calls.clone();
    let captured_failures = failure_probe.clone();
    let expected_proof: Arc<str> = Arc::from(local_proof);
    let real_router = application.router().layer(axum::middleware::from_fn(move |request: Request<Body>, next: axum::middleware::Next| {
        let captured = captured.clone();
        let failure = captured_failures.clone();
        let proof = expected_proof.clone();
        async move {
            let connected_request = request.uri().path() == "/api/apps" && request.headers().get("x-nomi-local-trust")
                .and_then(|value|value.to_str().ok()) == Some(proof.as_ref());
            let response = next.run(request).await;
            if connected_request && response.status() == StatusCode::OK {
                // Test-only fault at the real authenticated HTTP boundary.
                // Cookie-authenticated UI and the internal driver are untouched.
                if failure.active.load(Ordering::SeqCst) == 1 {
                    failure.responses.fetch_add(1,Ordering::SeqCst);
                    return (StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({"success":false,"code":"SMOKE_UPSTREAM_UNAVAILABLE"}))).into_response();
                }
                captured.fetch_add(1, Ordering::SeqCst);
            }
            response
        }
    }));
    let served = real_router.merge(Router::new().route("/__app-hosting-smoke/ui-state", post(browser_result)).with_state(state.clone()));
    let server = tokio::spawn(async move { axum::serve(listener, served).await });
    let host_url = format!("http://{address}");
    let result = run_chain(router, api_key, local_proof, model, &host_url, state, host_calls, failure_probe).await;
    server.abort(); let _ = server.await;
    result
}

async fn run_chain(router: &Router, api_key: &str, local_proof: &str, model: &str,
    host_url: &str, browser_state: Arc<AtomicUsize>, host_calls: Arc<AtomicUsize>, failure_probe: Arc<UpstreamFailureProbe>) -> Result<(), SmokeFailure> {
    let artifacts = artifact_root()?;
    let no_auth = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10)).build()
        .map_err(|_| fail("app_hosting.host", "APP_HOSTING_HTTP_CLIENT_FAILED"))?
        .get(format!("{host_url}/api/apps")).send().await
        .map_err(|_| fail("app_hosting.host", "APP_HOSTING_AUTHENTICATED_HOST_UNAVAILABLE"))?;
    if no_auth.status() != reqwest::StatusCode::FORBIDDEN {
        return Err(fail("app_hosting.host", "APP_HOSTING_EXTERNAL_AUTH_NOT_ENFORCED"));
    }
    // Production setup initializes the existing installation-owner row. It
    // neither changes identity nor creates an alternate application owner.
    initialize_web_test_account(router).await?;
    // The manual page can create a real application without any configured
    // model or Agent Session. The provider is configured only afterward.
    let initial = envelope_data("app_hosting.create", successful_json(router, "app_hosting.create", Method::POST,
        "/api/apps", Some(json!({"name":"新应用","description":"","tags":[]})),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
    let initial_id = required_string("app_hosting.create", &initial, "/id", "APP_HOSTING_ID_MISSING")?;
    let directory = PathBuf::from(required_string("app_hosting.create", &initial, "/directory", "APP_HOSTING_MANAGED_DIRECTORY_MISSING")?);
    if !initial["development_session_id"].is_null() || initial["state"] != "stopped"
        || !initial["pid"].is_null() || std::fs::read_dir(&directory).map_err(|_| fail("app_hosting.create","APP_HOSTING_PROJECT_UNREADABLE"))?
            .next().is_some() {
        return Err(fail("app_hosting.create", "APP_HOSTING_MANUAL_CREATE_NOT_EMPTY"));
    }
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let opened = envelope_data("app_hosting.session", successful_json(router, "app_hosting.session", Method::POST,
        format!("/api/apps/{initial_id}/development-session"), Some(json!({"model":{"provider_id":provider,"model":model}})),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
    let session = required_string("app_hosting.session", &opened, "/conversation/conversation_id", "APP_HOSTING_SESSION_ID_MISSING")?;
    let conversation = &opened["conversation"];
    if opened["app"]["id"] != initial_id || opened["app"]["development_session_id"] != session
        || conversation["session_purpose"] != "conversation" || conversation.pointer("/extra/official_template_key") != Some(&json!("app.developer"))
        || opened.pointer("/app/settings/exclusive_session_ids").is_some() {
        return Err(fail("app_hosting.session", "APP_HOSTING_FIXED_ORDINARY_SESSION_MISMATCH"));
    }
    let binding = require_value("app_hosting.session", conversation, "/agent_snapshot/canonical_binding", "APP_HOSTING_SESSION_BINDING_MISSING")?;
    verify_resource_selections_in_workspace(&binding, &coding_resource_selections(), Some(&directory))?;
    let selected_workspace = required_string("app_hosting.session", conversation, "/extra/workspace", "APP_HOSTING_SESSION_WORKSPACE_MISSING")?;
    if std::fs::canonicalize(selected_workspace).ok() != std::fs::canonicalize(&directory).ok() {
        return Err(fail("app_hosting.session", "APP_HOSTING_CURRENT_WORKSPACE_MISMATCH"));
    }
    let actions = conversation.pointer("/agent_snapshot/enabled_capability_actions/app.hosting").and_then(Value::as_array)
        .ok_or_else(|| fail("app_hosting.session", "APP_HOSTING_BUILTIN_ACTIONS_MISSING"))?;
    let actual = actions.iter().filter_map(Value::as_str).collect::<std::collections::BTreeSet<_>>();
    if actual != std::collections::BTreeSet::from(["app.hosting/guide","app.hosting/current","app.hosting/import","app.hosting/list",
        "app.hosting/update","app.hosting/preview","app.hosting/rebuild","app.hosting/start","app.hosting/status","app.hosting/stop"]) {
        return Err(fail("app_hosting.session", "APP_HOSTING_BUILTIN_ACTION_SET_MISMATCH"));
    }
    let preset = required_string("app_hosting.session", &binding, "/preset_revision_ref/preset_id", "APP_HOSTING_PRESET_ID_MISSING")?;
    let revision_number = binding.pointer("/preset_revision_ref/revision").and_then(Value::as_u64)
        .ok_or_else(|| fail("app_hosting.session", "APP_HOSTING_REVISION_MISSING"))?;
    let revision = envelope_data("app_hosting.session", successful_json(router, "app_hosting.session", Method::GET,
        format!("/api/agent-presets/{preset}/revisions/{revision_number}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
    ensure_single_model_route(&revision["document"], &provider, model)?;
    assert_session_runtime(router, &session).await?;
    let app_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|_| fail("app_hosting.generate", "APP_HOSTING_PORT_SELECTION_FAILED"))?;
    let app_port = app_listener.local_addr().map_err(|_| fail("app_hosting.generate", "APP_HOSTING_PORT_SELECTION_FAILED"))?.port();
    drop(app_listener);
    let prompt = format!("请在这个应用工作区做一个我能实际使用、可连接 NomiFun 的中文计数器应用。先通过 read_app_guide 阅读当前平台指南，再用 current_app 确认当前应用。这里已经有应用记录，授权工作区就是该应用自己的空托管目录：在根目录从零写 index.html 和 server.cjs，不要新建counter-app子目录，不要导入或创建另一个应用。前端展示标题‘我的计数器’，初始数字0，有‘+1’、‘清空’按钮，点击即时更新，刷新仍保存计数（localStorage）；页面加载请求自己后端的 /api/nomifun-status，展示‘NomiFun已连接’及托管应用数量。server.cjs使用Node内置http/fs服务index.html和/api/nomifun-status；后端按指南读取自动注入的NOMIFUN_HOST_URL和NOMIFUN_LOCAL_TRUST，真实fetch NomiFun /api/apps（x-nomi-local-trust头），成功仅返回JSON {{connected:true,app_count:响应data数组长度}}，失败返回真实错误状态及connected:false。凭据只进后端请求头，不能写入文件、前端、JSON响应或日志。项目不依赖CDN、第三方包、SDK或Manifest。\n请实际写齐根目录index.html和server.cjs两个文件，读取确认并检查脚本语法。用update_app更新current_app返回的同一应用：名称‘我的计数器’，填写简短描述和‘工具’标签，import_kind为source，project_directory为.，install_command与build_command为null，preview_command和start_command均为node server.cjs，environment PORT为字符串{app_port}，web_url为http://127.0.0.1:{app_port}；Node服务监听127.0.0.1和process.env.PORT。再通过preview_app（或start_app）启动同一应用，读取状态，真实HTTP检查网页和/api/nomifun-status确认可达及平台已连接。不要用start_process另行拉起服务器。完成后给应用名称、网页地址及实际检查结果，不要只给代码或计划，不要提交。");
    write_evidence(artifacts.as_deref(), "request.json", &json!({"schema_version":3,"prompt":prompt,
        "template":"app.developer","fixed_app_agent":true,"manual_create_before_model_configuration":true,
        "provider":"stepfun-plan","model":model,"session_id":session,"app_id":initial_id,"current_workspace":directory}))?;
    let operation = start_session_turn(router, "app_hosting.generate", &session,
        &uuid::Uuid::now_v7().to_string(), prompt).await?;
    eprintln!("NOMIFUN_LIVE_APP_HOSTING_STAGE stage=generate status=running");
    let generated = wait_for_turn(router, &session, &operation).await;
    let events = session_events(router, &session).await?;
    let trace = settlements(&events, &[api_key,local_proof])?;
    write_evidence(artifacts.as_deref(), "canonical-tool-settlements.json", &json!(trace))?;
    for failed in trace.iter().filter(|settlement| settlement["result_success"] != true) {
        if let Some(error) = failed["diagnostic"].as_str() {
            for phase in ["domain", "principal", "capability", "action", "store", "session_read", "session_owner",
                "resolved_snapshot_ref", "resource_key", "replay_read", "canonical_tool_causation", "effect_started_append",
                "import_source_canonicalization", "effect_replay_read"] {
                if error.contains(phase) { eprintln!("NOMIFUN_LIVE_APP_HOSTING_REJECTION phase={phase}"); }
            }
        }
    }
    if let Some(root) = &artifacts {
        copy_generated_files(&directory, &root.join("generated-project"), &[api_key.as_bytes(),local_proof.as_bytes()])?;
    }
    generated?;
    for action in ["workspace.files/write", "app.hosting/update"] {
        if !trace.iter().any(|settlement| settlement["action_id"] == action && settlement["result_success"] == true
            && !settlement["effect_succeeded_seq"].is_null()) {
            return Err(fail("app_hosting.events", "APP_HOSTING_REQUIRED_SUCCESSFUL_SETTLEMENT_MISSING"));
        }
    }
    if !trace.iter().any(|settlement| settlement["action_id"] == "app.hosting/guide" && settlement["result_success"] == true) {
        return Err(fail("app_hosting.events", "APP_HOSTING_GUIDE_NOT_READ"));
    }
    if trace.iter().any(|settlement| settlement["action_id"] == "app.hosting/import") {
        return Err(fail("app_hosting.events", "APP_HOSTING_UNEXPECTED_DUPLICATE_IMPORT"));
    }
    if !trace.iter().any(|settlement| settlement["action_id"] == "app.hosting/current" && settlement["result_success"] == true
        && settlement["app_id"] == initial_id) {
        return Err(fail("app_hosting.events", "APP_HOSTING_CURRENT_APPLICATION_NOT_IDENTIFIED"));
    }
    if !trace.iter().any(|settlement| matches!(settlement["action_id"].as_str(), Some("app.hosting/start" | "app.hosting/preview"))
        && settlement["result_success"] == true && !settlement["effect_succeeded_seq"].is_null()) {
        return Err(fail("app_hosting.events", "APP_HOSTING_REQUIRED_RUN_SETTLEMENT_MISSING"));
    }
    let apps = envelope_data("app_hosting.list", successful_json(router, "app_hosting.list", Method::GET,
        "/api/apps", None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
    let apps = apps.as_array().filter(|apps| apps.len() == 1)
        .ok_or_else(|| fail("app_hosting.list", "APP_HOSTING_EXPECTED_ONE_APPLICATION"))?;
    let mut app = apps[0].clone();
    let app_id = required_string("app_hosting.list", &app, "/id", "APP_HOSTING_ID_MISSING")?;
    if app_id != initial_id || app["development_session_id"] != session || app["name"] != "我的计数器"
        || app["description"].as_str().is_none_or(|value|value.trim().is_empty())
        || app["tags"].as_array().is_none_or(|tags|!tags.contains(&json!("工具"))) {
        return Err(fail("app_hosting.list", "APP_HOSTING_CURRENT_RECORD_NOT_UPDATED"));
    }
    if app["state"] != "running" || app["pid"].as_u64().is_none_or(|pid| pid == 0) {
        return Err(fail("app_hosting.list", "APP_HOSTING_APPLICATION_NOT_RUNNING"));
    }
    if !trace.iter().any(|settlement| settlement["action_id"] == "app.hosting/update" && settlement["app_id"] == app_id)
        || !trace.iter().any(|settlement| matches!(settlement["action_id"].as_str(),Some("app.hosting/start" | "app.hosting/preview")) && settlement["app_id"] == app_id) {
        return Err(fail("app_hosting.events", "APP_HOSTING_SETTLEMENT_REGISTRATION_MISMATCH"));
    }
    let current_directory = PathBuf::from(required_string("app_hosting.list", &app, "/directory", "APP_HOSTING_MANAGED_DIRECTORY_MISSING")?);
    if std::fs::canonicalize(&current_directory).ok() != std::fs::canonicalize(&directory).ok() {
        return Err(fail("app_hosting.files", "APP_HOSTING_WORKSPACE_WAS_REPLACED"));
    }
    let source_path = directory.join("index.html");
    let mut source = std::fs::read(&source_path).map_err(|_| fail("app_hosting.files", "APP_HOSTING_SOURCE_HTML_MISSING"))?;
    let html = std::str::from_utf8(&source).map_err(|_| fail("app_hosting.files", "APP_HOSTING_HTML_INVALID_UTF8"))?;
    if !html.contains("localStorage") || !html.contains("+1") || !html.contains("清空") || !html.contains("我的计数器")
        || html.contains("NOMIFUN_LOCAL_TRUST") || html.contains("x-nomi-local-trust") {
        return Err(fail("app_hosting.files", "APP_HOSTING_HTML_REQUIREMENTS_MISSING"));
    }
    verify_game_script_syntax(html)?;
    let backend_source = directory.join("server.cjs");
    let mut backend = std::fs::read(&backend_source).map_err(|_| fail("app_hosting.files", "APP_HOSTING_BACKEND_MISSING"))?;
    let backend_text = std::str::from_utf8(&backend).map_err(|_| fail("app_hosting.files", "APP_HOSTING_BACKEND_INVALID_UTF8"))?;
    if !backend_text.contains("NOMIFUN_HOST_URL") || !backend_text.contains("NOMIFUN_LOCAL_TRUST") || !backend_text.contains("x-nomi-local-trust") {
        return Err(fail("app_hosting.connection", "APP_HOSTING_BACKEND_CONNECTION_ENVIRONMENT_MISSING"));
    }
    let mut check = std::process::Command::new("node");
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt as _;
        check.creation_flags(0x08000000);
    }
    if !check.arg("--check").arg(&backend_source).stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null()).status().map_err(|_| fail("app_hosting.files", "APP_HOSTING_BACKEND_SYNTAX_CHECK_UNAVAILABLE"))?.success() {
        return Err(fail("app_hosting.files", "APP_HOSTING_BACKEND_SYNTAX_INVALID"));
    }
    let web_url = required_string("app_hosting.list", &app, "/settings/web_url", "APP_HOSTING_WEB_URL_MISSING")?;
    let parsed = reqwest::Url::parse(&web_url).map_err(|_| fail("app_hosting.http", "APP_HOSTING_WEB_URL_INVALID"))?;
    if parsed.scheme() != "http" || parsed.host_str() != Some("127.0.0.1") || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(fail("app_hosting.http", "APP_HOSTING_WEB_URL_NOT_LOOPBACK"));
    }
    let http = reqwest::Client::builder().no_proxy().pool_max_idle_per_host(0).redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10)).build().map_err(|_| fail("app_hosting.http", "APP_HOSTING_HTTP_CLIENT_FAILED"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let response = loop {
        match http.get(&web_url).send().await {
            Ok(response) => break response,
            Err(_) if tokio::time::Instant::now() < deadline => tokio::time::sleep(POLL_INTERVAL).await,
            Err(_) => return Err(fail("app_hosting.http", "APP_HOSTING_REAL_HTTP_UNAVAILABLE")),
        }
    };
    if response.status() != reqwest::StatusCode::OK { return Err(fail("app_hosting.http", "APP_HOSTING_REAL_HTTP_FAILED")); }
    let served = response.bytes().await.map_err(|_| fail("app_hosting.http", "APP_HOSTING_REAL_HTTP_BODY_FAILED"))?;
    if served.as_ref() != source { return Err(fail("app_hosting.http", "APP_HOSTING_HTTP_GENERATED_HTML_MISMATCH")); }
    let host_calls_before = host_calls.load(Ordering::SeqCst);
    let connected = http.get(format!("{}/api/nomifun-status", web_url.trim_end_matches('/'))).send().await
        .map_err(|_| fail("app_hosting.connection", "APP_HOSTING_CONNECTION_ROUTE_UNAVAILABLE"))?;
    if connected.status() != reqwest::StatusCode::OK { return Err(fail("app_hosting.connection", "APP_HOSTING_LOCALTRUST_REQUEST_FAILED")); }
    let connection = connected.json::<Value>().await
        .map_err(|_| fail("app_hosting.connection", "APP_HOSTING_CONNECTION_RESPONSE_INVALID"))?;
    if !object_has_only_keys(&connection, &["connected", "app_count"]) || connection["connected"] != true || connection["app_count"] != 1 {
        return Err(fail("app_hosting.connection", "APP_HOSTING_CONNECTION_RESULT_MISMATCH"));
    }
    let verified_host_requests = host_calls.load(Ordering::SeqCst).saturating_sub(host_calls_before);
    if verified_host_requests == 0 {
        return Err(fail("app_hosting.connection", "APP_HOSTING_NO_ACTUAL_AUTHENTICATED_HOST_REQUEST"));
    }
    let mut evidence = json!({"schema_version":3,"provider":"stepfun-plan","model":model,"template":"app.developer",
        "default_hosting_grant":true,"fixed_app_agent":true,"manual_create_before_model_configuration":true,
        "ordinary_session":true,"current_workspace":directory,"current_workspace_is_managed_directory":true,"application_count":1,"application_id_preserved":true,
        "session_id":session,"operation_id":operation,"app_id":app_id,"app_name":app["name"],"app_url":web_url,
        "managed_pid":app["pid"],"preview":app["preview"],"runtime_terminal":"turn/completed", "settlements":trace,
        "turns":[{"operation_id":operation,"runtime_terminal":"turn/completed","settlements":trace}],
        "source_sha256":digest(&source),"source_bytes":source.len(),"backend_sha256":digest(&backend),
        "script_syntax_checked":true,"http_status":200,"http_bytes_equal":true,"browser_qa":null,
        "external_unauthenticated_status":403,"connection":connection,"verified_host_requests":verified_host_requests,
        "generated_project":artifacts.as_ref().map(|root|root.join("generated-project"))});
    write_evidence(artifacts.as_deref(), "evidence.json", &evidence)?;
    evidence["browser_qa"] = independent_browser_qa(&browser_state,&web_url,host_url,&session,&app_id,"generated").await?;
    write_evidence(artifacts.as_deref(), "evidence.json", &evidence)?;

    let first_pid = app["pid"].as_u64().and_then(|pid|u32::try_from(pid).ok())
        .ok_or_else(|| fail("app_hosting.repair","APP_HOSTING_INITIAL_PID_MISSING"))?;
    let first_backend_digest = digest(&backend);
    let repair_prompt = "请继续编辑当前这个应用，修复后端错误语义：目前server.cjs把NomiFun连接失败吞成connected:false却仍返回HTTP200。仍使用current_app确认同一应用和工作区，先用stop_app停止它，再通过普通文件工具修改根server.cjs，不新建应用、不import、不另开会话。NomiFun上游/api/apps返回非成功HTTP状态时，你的/api/nomifun-status必须保留该HTTP错误状态，例如503必须返回503；上游网络/协议错误用502或504。成功仅在HTTP成功、平台success不是false且data确实是应用数组时返回200和{connected:true,app_count:实际数量}；失败JSON只含{connected:false,app_count:0}。凭据只放后端请求头，不能输出到响应、日志或文件；不要写特殊测试分支。保留页面计数功能、名称、描述、标签和现有PORT/web_url配置。检查语法后通过preview_app或rebuild_app重新启动当前同一个应用，实际检查正常连接，并报告修复结果。不要用start_process另起服务器，不要提交。".to_owned();
    write_evidence(artifacts.as_deref(),"repair-request.json",&json!({"schema_version":3,"prompt":repair_prompt,
        "session_id":session,"app_id":app_id,"current_workspace":directory}))?;
    let repair_operation = start_session_turn(router,"app_hosting.repair",&session,&uuid::Uuid::now_v7().to_string(),repair_prompt).await?;
    eprintln!("NOMIFUN_LIVE_APP_HOSTING_STAGE stage=repair status=running");
    let repaired = wait_for_turn(router,&session,&repair_operation).await;
    let all_events = session_events(router,&session).await?;
    if all_events.len() < events.len() || all_events[..events.len()] != events {
        return Err(fail("app_hosting.history","APP_HOSTING_PRIOR_CANONICAL_HISTORY_CHANGED"));
    }
    let repair_trace = settlements(&all_events[events.len()..],&[api_key,local_proof])?;
    write_evidence(artifacts.as_deref(),"repair-canonical-tool-settlements.json",&json!(repair_trace))?;
    if let Some(root) = &artifacts { copy_generated_files(&directory,&root.join("generated-project-repaired"),&[api_key.as_bytes(),local_proof.as_bytes()])?; }
    repaired?;
    let stopped_call = repair_trace.iter().find(|settlement| settlement["action_id"] == "app.hosting/stop"
        && settlement["result_success"] == true && !settlement["effect_succeeded_seq"].is_null() && settlement["app_id"] == app_id)
        .ok_or_else(|| fail("app_hosting.repair","APP_HOSTING_REPAIR_STOP_NOT_SETTLED"))?;
    let write_call = repair_trace.iter().find(|settlement| matches!(settlement["action_id"].as_str(),Some("workspace.files/patch" | "workspace.files/write"))
        && settlement["result_success"] == true && !settlement["effect_succeeded_seq"].is_null()
        && settlement["dispatch_seq"].as_u64() > stopped_call["result_seq"].as_u64())
        .ok_or_else(|| fail("app_hosting.repair","APP_HOSTING_REPAIR_WRITE_NOT_SETTLED"))?;
    if !repair_trace.iter().any(|settlement| matches!(settlement["action_id"].as_str(),Some("app.hosting/preview" | "app.hosting/rebuild" | "app.hosting/start"))
        && settlement["result_success"] == true && !settlement["effect_succeeded_seq"].is_null() && settlement["app_id"] == app_id
        && settlement["dispatch_seq"].as_u64() > write_call["result_seq"].as_u64())
        || repair_trace.iter().any(|settlement|settlement["action_id"] == "app.hosting/import") {
        return Err(fail("app_hosting.repair","APP_HOSTING_REPAIR_RUN_NOT_SETTLED"));
    }
    app = envelope_data("app_hosting.repair",successful_json(router,"app_hosting.repair",Method::GET,
        format!("/api/apps/{app_id}"),None,LOCAL_API_DEADLINE,&[StatusCode::OK]).await?)?;
    if app["id"] != initial_id || app["development_session_id"] != session || app["state"] != "running"
        || app["pid"].as_u64().is_none_or(|pid|pid == 0) || process_exists(first_pid) {
        return Err(fail("app_hosting.repair","APP_HOSTING_REPAIR_CURRENT_PROCESS_MISMATCH"));
    }
    backend = std::fs::read(&backend_source).map_err(|_|fail("app_hosting.repair","APP_HOSTING_REPAIRED_BACKEND_MISSING"))?;
    if digest(&backend) == first_backend_digest { return Err(fail("app_hosting.repair","APP_HOSTING_REPAIR_SOURCE_UNCHANGED")); }
    source = std::fs::read(&source_path).map_err(|_|fail("app_hosting.repair","APP_HOSTING_REPAIRED_HTML_MISSING"))?;
    verify_game_script_syntax(std::str::from_utf8(&source).map_err(|_|fail("app_hosting.repair","APP_HOSTING_HTML_INVALID_UTF8"))?)?;
    let mut check = std::process::Command::new("node");
    #[cfg(windows)] { use std::os::windows::process::CommandExt as _; check.creation_flags(0x08000000); }
    if !check.arg("--check").arg(&backend_source).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .status().map_err(|_|fail("app_hosting.repair","APP_HOSTING_BACKEND_SYNTAX_CHECK_UNAVAILABLE"))?.success() {
        return Err(fail("app_hosting.repair","APP_HOSTING_REPAIRED_BACKEND_SYNTAX_INVALID"));
    }
    let failure_count_before = failure_probe.responses.load(Ordering::SeqCst);
    failure_probe.active.store(1,Ordering::SeqCst);
    let failed_response = http.get(format!("{web_url}/api/nomifun-status")).send().await;
    failure_probe.active.store(0,Ordering::SeqCst);
    let failed_response = failed_response.map_err(|_|fail("app_hosting.failure","APP_HOSTING_ERROR_ROUTE_UNAVAILABLE"))?;
    let failed_status = failed_response.status().as_u16();
    let failed_body = failed_response.json::<Value>().await.map_err(|_|fail("app_hosting.failure","APP_HOSTING_ERROR_RESPONSE_INVALID"))?;
    let injected_requests = failure_probe.responses.load(Ordering::SeqCst).saturating_sub(failure_count_before);
    if failed_status != 503 || failed_body != json!({"connected":false,"app_count":0}) || injected_requests == 0 {
        return Err(fail("app_hosting.failure","APP_HOSTING_REAL_UPSTREAM_FAILURE_NOT_PROPAGATED"));
    }
    let successful_before = host_calls.load(Ordering::SeqCst);
    let recovered = http.get(format!("{web_url}/api/nomifun-status")).send().await
        .map_err(|_|fail("app_hosting.failure","APP_HOSTING_RECOVERY_ROUTE_UNAVAILABLE"))?;
    if recovered.status() != reqwest::StatusCode::OK || recovered.json::<Value>().await.ok() != Some(json!({"connected":true,"app_count":1}))
        || host_calls.load(Ordering::SeqCst) <= successful_before {
        return Err(fail("app_hosting.failure","APP_HOSTING_REAL_UPSTREAM_RECOVERY_FAILED"));
    }
    let recovered_requests = host_calls.load(Ordering::SeqCst).saturating_sub(successful_before);
    let served = http.get(&web_url).send().await.map_err(|_|fail("app_hosting.repair","APP_HOSTING_REPAIRED_HTTP_UNAVAILABLE"))?;
    if served.status() != reqwest::StatusCode::OK || served.bytes().await.ok().as_deref() != Some(source.as_slice()) {
        return Err(fail("app_hosting.repair","APP_HOSTING_REPAIRED_HTTP_BYTES_MISMATCH"));
    }
    evidence["backend_sha256"] = json!(digest(&backend)); evidence["source_sha256"] = json!(digest(&source));
    evidence["source_bytes"] = json!(source.len()); evidence["managed_pid"] = app["pid"].clone(); evidence["preview"] = app["preview"].clone();
    evidence["repair"] = json!({"operation_id":repair_operation,"runtime_terminal":"turn/completed","app_id_preserved":true,
        "session_id_preserved":true,"prior_history_unchanged":true,"source_changed":true,"original_pid_gone":true,"managed_pid":app["pid"],
        "settlements":repair_trace,"upstream_failure":{"injected_http_status":503,"app_http_status":failed_status,"body":failed_body,
            "verified_injected_requests":injected_requests,"recovered_success":true,"recovered_verified_requests":recovered_requests}});
    evidence["turns"].as_array_mut().unwrap().push(json!({"operation_id":repair_operation,"runtime_terminal":"turn/completed","settlements":repair_trace}));
    evidence["repaired_project"] = json!(artifacts.as_ref().map(|root| root.join("generated-project-repaired")));
    write_evidence(artifacts.as_deref(),"evidence.json",&evidence)?;
    evidence["repair"]["browser_qa"] = independent_browser_qa(&browser_state,&web_url,host_url,&session,&app_id,"repaired").await?;
    write_evidence(artifacts.as_deref(),"evidence.json",&evidence)?;
    let stopped = envelope_data("app_hosting.stop", successful_json(router, "app_hosting.stop", Method::POST,
        format!("/api/apps/{app_id}/operate"), Some(json!({"operation":"stop"})), LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
    if stopped["state"] != "stopped" || !stopped["pid"].is_null() {
        return Err(fail("app_hosting.stop", "APP_HOSTING_PROCESS_DID_NOT_STOP"));
    }
    let root_pid = app["pid"].as_u64().and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| fail("app_hosting.stop", "APP_HOSTING_MANAGED_PID_MISSING"))?;
    let root_pid_gone = !process_exists(root_pid);
    let port = parsed.port().ok_or_else(|| fail("app_hosting.stop", "APP_HOSTING_WEB_PORT_MISSING"))?;
    let tcp_connect = tokio::time::timeout(Duration::from_secs(1), tokio::net::TcpStream::connect(("127.0.0.1",port)))
        .await.is_ok_and(|result| result.is_ok());
    let after_stop_response = match http.get(&web_url).send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.bytes().await.ok();
            Some(json!({"status":status,"bytes":body.as_ref().map(|body|body.len()),
                "sha256":body.as_ref().map(|body|digest(body)),
                "matches_generated_html":body.as_deref() == Some(source.as_slice())}))
        },
        Err(_) => None,
    };
    let stop = json!({"returned_state":stopped["state"],"returned_pid":stopped["pid"],"observed_root_pid":root_pid,
        "root_pid_gone":root_pid_gone,"direct_tcp_connect":tcp_connect,"no_proxy":true,"fresh_http_connection":true,
        "after_stop_response":after_stop_response});
    evidence["stop"] = stop.clone();
    write_evidence(artifacts.as_deref(), "evidence.json", &evidence)?;
    if !root_pid_gone || tcp_connect || after_stop_response.is_some() {
        eprintln!("NOMIFUN_LIVE_APP_HOSTING_STOP_DIAGNOSTIC {}", json!({"app_url":web_url,"host_url":host_url,"stop":stop}));
        // Keep the same authoritative fixture alive briefly for an external OS
        // process/listener inspection before ordinary Host cleanup can hide it.
        tokio::time::sleep(Duration::from_secs(10)).await;
        return Err(fail("app_hosting.stop", "APP_HOSTING_STOP_PROOF_FAILED"));
    }
    evidence["stop_verified"] = json!(true);
    write_evidence(artifacts.as_deref(), "evidence.json", &evidence)?;
    eprintln!("NOMIFUN_LIVE_APP_HOSTING_EVIDENCE {evidence}");
    Ok(())
}

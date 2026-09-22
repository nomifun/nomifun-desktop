//! Prepare a NEW disposable main-app dataset and a loopback-only held model.
//! No real provider credentials, user dataset, or browser profile is read.
//! Usage: cargo run -p nomifun-app --example browser_gui_fixture -- <new-data-dir> [--native-actions|--computer-denied|--computer-input]
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
    workspace::BrowserResourceService,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    computer_denied: bool,
    computer_input: bool,
    computer_file: Option<PathBuf>,
    a11y_observed: AtomicBool,
    screen_denied: AtomicBool,
    input_verified: AtomicBool,
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

fn browser_tool(body: &Value, action: &str) -> anyhow::Result<String> {
    let tools = body["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("model request has no Tool surface"))?;
    let marker = format!("Action: {action}.");
    tools
        .iter()
        .find(|tool| {
            tool["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains(&marker))
        })
        .and_then(|tool| tool["function"]["name"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            let available = tools
                .iter()
                .filter_map(|tool| tool["function"]["description"].as_str())
                .filter(|description| description.contains("Browser"))
                .collect::<Vec<_>>();
            anyhow::anyhow!("Selected Browser action {action} missing from model request: {available:?}")
        })
}

fn native_operation(
    fixture: &Fixture,
    body: &Value,
    step: usize,
) -> anyhow::Result<Option<(String, Value)>> {
    let reference = |name: &str| -> anyhow::Result<Value> {
        let observation = last_tool(body)?;
        observation["elements"].as_array()
            .and_then(|elements| elements.iter().find(|element| element["name"] == name))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Fresh observed reference missing: {name}"))
    };
    let (action, operation) = match step {
        // The first effect attempt activates the engine-owned task ledger. It
        // must fail closed before update_plan is exposed, then replan.
        0 => ("browser/navigate", json!({"url":fixture.native_url})),
        1 => return Ok(Some(("update_plan".into(), json!({
            "explanation":"The guarded first effect activated the task ledger; retry through the exact Browser actions.",
            "requirements":[{
                "id":"req-native-browser",
                "description":"Verify the packaged native Browser surface.",
                "source":{"input":0,"quote":"Verify the packaged native Browser surface."}
            }],
            "plan":[
                {"step":"Verify packaged native Browser navigation and input","status":"in_progress"},
                {"step":"Verify packaged native Browser page result","status":"pending"}
            ]
        })))),
        2 => ("browser/navigate", json!({"url":fixture.native_url})),
        3 | 5 | 7 => {
            last_tool(body)?;
            ("browser/observe", json!({}))
        }
        4 => ("browser/act", json!({"action":"type","element":reference("备注")?,"text":"Agent 主界面真实输入"})),
        6 => ("browser/act", json!({"action":"click","element":reference("增加计数")?})),
        8 => {
            anyhow::ensure!(last_tool(body)?.to_string().contains("已收到真实点击"), "Post-click observation omitted the real page result");
            return Ok(Some(("update_plan".into(), json!({
                "explanation":"The packaged native page reflects the exact typed text and trusted click result.",
                "plan":[
                {"step":"Verify packaged native Browser navigation and input","status":"completed"},
                {"step":"Verify packaged native Browser page result","status":"completed"}
            ]}))));
        }
        9 => return Ok(Some(("report_completion".into(), json!({
            "summary":"The packaged native Browser navigated, typed Unicode text and delivered a trusted click witness.",
            "criteria":[
                {
                    "step":"Verify packaged native Browser navigation and input",
                    "disposition":"supported",
                    "evidence_call_ids":["gui-native-7"],
                    "rationale":"The final observation reflects the native type action result after all effects settled.",
                    "requirement_ids":[]
                },
                {
                    "step":"Verify packaged native Browser page result",
                    "disposition":"supported",
                    "evidence_call_ids":["gui-native-7"],
                    "rationale":"The final observation contains the trusted click result.",
                    "requirement_ids":["req-native-browser"]
                }
            ]
        })))),
        10 => {
            return Ok(None);
        }
        _ => anyhow::bail!("Unexpected model request {step}"),
    };
    Ok(Some((browser_tool(body, action)?, operation)))
}

fn computer_operation(
    fixture: &Fixture,
    body: &Value,
) -> anyhow::Result<Option<(String, String, Value)>> {
    let result = |call_id: &str| {
        body["messages"].as_array().and_then(|messages| {
            messages.iter().find(|message| {
                message["role"] == "tool"
                    && message["tool_call_id"].as_str() == Some(call_id)
            })
        })
    };
    if result("gui-computer-report").is_some() {
        return Ok(None);
    }
    if result("gui-computer-plan").is_some() {
        return Ok(Some((
            "gui-computer-report".into(),
            "report_completion".into(),
            json!({
                "summary":"Accessibility observation succeeded and Screen Recording was denied through the canonical host-provider failure.",
                "criteria":[
                    {
                        "step":"Obtain an Accessibility snapshot",
                        "disposition":"supported",
                        "evidence_call_ids":["gui-computer-a11y"],
                        "rationale":"The signed product returned an actionable Accessibility snapshot.",
                        "requirement_ids":["req-computer-a11y"]
                    },
                    {
                        "step":"Attempt a Screen Recording screenshot",
                        "disposition":"unverified",
                        "evidence_call_ids":[],
                        "rationale":"macOS denied Screen Recording and the runtime surfaced ROLE_HOST_PROVIDER_FAILURE without retrying.",
                        "requirement_ids":["req-computer-screen"]
                    }
                ]
            }),
        )));
    }
    if let Some(message) = result("gui-computer-screen") {
        let result = message["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Computer denial result missing"))?;
        anyhow::ensure!(
            result.starts_with(
                "Capability Kernel rejected Agent Runtime Tool (ROLE_HOST_PROVIDER_FAILURE)"
            ),
            "Computer provider failure was not projected through its canonical model error"
        );
        fixture.screen_denied.store(true, Ordering::SeqCst);
        return Ok(Some((
            "gui-computer-plan".into(),
            "update_plan".into(),
            json!({
                "explanation":"Record the successful Accessibility observation and the terminal Screen Recording permission denial.",
                "requirements":[
                    {
                        "id":"req-computer-a11y",
                        "description":"Obtain an Accessibility snapshot.",
                        "source":{"input":0,"quote":"obtain an accessibility snapshot"}
                    },
                    {
                        "id":"req-computer-screen",
                        "description":"Attempt a Screen Recording screenshot.",
                        "source":{"input":0,"quote":"attempt a screenshot"}
                    }
                ],
                "plan":[
                    {"step":"Obtain an Accessibility snapshot","status":"completed"},
                    {"step":"Attempt a Screen Recording screenshot","status":"completed"}
                ]
            }),
        )));
    }
    if let Some(message) = result("gui-computer-a11y") {
        let result = message["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Accessibility result missing"))?;
        anyhow::ensure!(
            result.contains("Accessibility snapshot") && !result.contains("[tool error]"),
            "Accessibility observation failed"
        );
        fixture.a11y_observed.store(true, Ordering::SeqCst);
        return Ok(Some((
            "gui-computer-screen".into(),
            browser_tool(body, "computer/observe")?,
            json!({"action":"screenshot"}),
        )));
    }
    Ok(Some((
        "gui-computer-a11y".into(),
        browser_tool(body, "computer/a11y.observe")?,
        json!({"action":"observe"}),
    )))
}

fn computer_tool_result<'a>(body: &'a Value, call_id: &str) -> Option<&'a Value> {
    body["messages"].as_array().and_then(|messages| {
        messages.iter().find(|message| {
            message["role"] == "tool" && message["tool_call_id"].as_str() == Some(call_id)
        })
    })
}

fn computer_result_text<'a>(body: &'a Value, call_id: &str) -> anyhow::Result<&'a str> {
    computer_tool_result(body, call_id)
        .and_then(|message| message["content"].as_str())
        .ok_or_else(|| anyhow::anyhow!("Computer result missing for {call_id}"))
}

fn accessibility_snapshot_text(content: &str) -> anyhow::Result<String> {
    if let Ok(value) = serde_json::from_str::<Value>(content)
        && let Some(text) = value["result"]["text"].as_str()
    {
        return Ok(text.to_owned());
    }
    anyhow::ensure!(
        content.contains("Accessibility snapshot"),
        "Accessibility snapshot text missing"
    );
    Ok(content.to_owned())
}

fn computer_result_generation(content: &str) -> anyhow::Result<u64> {
    let value: Value = serde_json::from_str(content)?;
    value["generation"]
        .as_u64()
        .filter(|generation| *generation > 0)
        .ok_or_else(|| anyhow::anyhow!("Computer observation generation missing"))
}

fn require_computer_success(body: &Value, call_id: &str) -> anyhow::Result<()> {
    let result = computer_result_text(body, call_id)?;
    anyhow::ensure!(
        !result.contains("Capability Kernel rejected")
            && !result.contains("plan needs reconsideration")
            && !result.contains("[tool error]"),
        "Computer action {call_id} failed"
    );
    Ok(())
}

fn computer_input_call(
    body: &Value,
    call_id: &str,
    observation_call_id: &str,
    action: &str,
    parameters: Value,
) -> anyhow::Result<(String, String, Value)> {
    let generation = computer_result_generation(computer_result_text(
        body,
        observation_call_id,
    )?)?;
    let mut parameters = parameters
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Computer input parameters must be an object"))?;
    parameters.insert("action".into(), Value::String(action.to_owned()));
    parameters.insert("expected_generation".into(), json!(generation));
    Ok((
        call_id.to_owned(),
        browser_tool(body, "computer/input")?,
        Value::Object(parameters),
    ))
}

fn text_editor_ref(content: &str) -> anyhow::Result<u32> {
    let text = accessibility_snapshot_text(content)?;
    anyhow::ensure!(
        text.contains("computer-input.txt"),
        "Disposable TextEdit fixture is not the foreground accessibility window"
    );
    text.lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            let is_editor = [
                "text area",
                "text entry",
                "textarea",
                "textfield",
                "text field",
            ]
            .iter()
            .any(|role| lower.contains(role));
            if !is_editor {
                return None;
            }
            line.trim_start()
                .strip_prefix('[')?
                .split_once(']')?
                .0
                .parse()
                .ok()
        })
        .ok_or_else(|| anyhow::anyhow!("TextEdit accessibility text area ref missing"))
}

fn computer_input_operation(
    fixture: &Fixture,
    body: &Value,
) -> anyhow::Result<Option<(String, String, Value)>> {
    let file = fixture
        .computer_file
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Computer input fixture file missing"))?;
    let call = |id| computer_tool_result(body, id).is_some();
    if call("gui-computer-input-report") {
        return Ok(None);
    }
    if call("gui-computer-input-plan-finish") {
        return Ok(Some((
            "gui-computer-input-report".into(),
            "report_completion".into(),
            json!({
                "summary":"The disposable TextEdit fixture was launched, edited with Command, Option and Control modifiers, and saved.",
                "criteria":[
                    {
                        "step":"Launch the disposable TextEdit fixture",
                        "disposition":"supported",
                        "evidence_call_ids":["gui-computer-input-observe-after-save"],
                        "rationale":"The latest post-save Accessibility observation proves the disposable TextEdit document is the active target.",
                        "requirement_ids":[]
                    },
                    {
                        "step":"Verify Command, Option and Control input",
                        "disposition":"supported",
                        "evidence_call_ids":["gui-computer-input-observe-after-save"],
                        "rationale":"The same latest observation contains the expected modifier-derived value and no edited-state marker.",
                        "requirement_ids":["req-computer-input"]
                    }
                ]
            }),
        )));
    }
    if call("gui-computer-input-observe-after-save") {
        let snapshot = accessibility_snapshot_text(computer_result_text(
            body,
            "gui-computer-input-observe-after-save",
        )?)?;
        anyhow::ensure!(
            snapshot.contains("alpha XbetaY") && !snapshot.contains("已编辑"),
            "Post-save TextEdit observation did not prove the saved modifier-derived value"
        );
        return Ok(Some((
            "gui-computer-input-plan-finish".into(),
            "update_plan".into(),
            json!({
                "explanation":"The disposable editor reflected the expected modifier-derived value and its post-save state.",
                "plan":[
                    {"step":"Launch the disposable TextEdit fixture","status":"completed"},
                    {"step":"Verify Command, Option and Control input","status":"completed"}
                ]
            }),
        )));
    }
    if call("gui-computer-input-save") {
        require_computer_success(body, "gui-computer-input-save")?;
        return Ok(Some((
            "gui-computer-input-observe-after-save".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-verify") {
        let snapshot = accessibility_snapshot_text(computer_result_text(
            body,
            "gui-computer-input-verify",
        )?)?;
        anyhow::ensure!(
            snapshot.contains("alpha XbetaY"),
            "Modifier-derived TextEdit value missing from final Accessibility snapshot"
        );
        fixture.input_verified.store(true, Ordering::SeqCst);
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-save",
            "gui-computer-input-verify",
            "key",
            json!({"key":"cmd+s"}),
        )?));
    }
    if call("gui-computer-input-type-y") {
        require_computer_success(body, "gui-computer-input-type-y")?;
        return Ok(Some((
            "gui-computer-input-verify".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe-after-control") {
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-type-y",
            "gui-computer-input-observe-after-control",
            "type",
            json!({"text":"Y"}),
        )?));
    }
    if call("gui-computer-input-control") {
        require_computer_success(body, "gui-computer-input-control")?;
        return Ok(Some((
            "gui-computer-input-observe-after-control".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe-after-x") {
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-control",
            "gui-computer-input-observe-after-x",
            "key",
            json!({"key":"ctrl+e"}),
        )?));
    }
    if call("gui-computer-input-type-x") {
        require_computer_success(body, "gui-computer-input-type-x")?;
        return Ok(Some((
            "gui-computer-input-observe-after-x".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe-after-option") {
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-type-x",
            "gui-computer-input-observe-after-option",
            "type",
            json!({"text":"X"}),
        )?));
    }
    if call("gui-computer-input-option") {
        require_computer_success(body, "gui-computer-input-option")?;
        return Ok(Some((
            "gui-computer-input-observe-after-option".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe-after-command") {
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-option",
            "gui-computer-input-observe-after-command",
            "key",
            json!({"key":"option+left"}),
        )?));
    }
    if call("gui-computer-input-command") {
        require_computer_success(body, "gui-computer-input-command")?;
        return Ok(Some((
            "gui-computer-input-observe-after-command".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe-after-set") {
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-command",
            "gui-computer-input-observe-after-set",
            "key",
            json!({"key":"cmd+right"}),
        )?));
    }
    if call("gui-computer-input-set") {
        require_computer_success(body, "gui-computer-input-set")?;
        return Ok(Some((
            "gui-computer-input-observe-after-set".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe-4") {
        let reference = text_editor_ref(computer_result_text(
            body,
            "gui-computer-input-observe-4",
        )?)?;
        fixture.a11y_observed.store(true, Ordering::SeqCst);
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-set",
            "gui-computer-input-observe-4",
            "set_element_value",
            json!({"ref":reference,"text":"alpha beta"}),
        )?));
    }
    if call("gui-computer-input-observe-3") {
        let result = computer_result_text(body, "gui-computer-input-observe-3")?;
        if accessibility_snapshot_text(result)
            .ok()
            .is_none_or(|text| !text.contains("computer-input.txt"))
        {
            std::thread::sleep(Duration::from_millis(250));
            return Ok(Some((
                "gui-computer-input-observe-4".into(),
                browser_tool(body, "computer/a11y.observe")?,
                json!({"action":"observe"}),
            )));
        }
        let reference = text_editor_ref(computer_result_text(
            body,
            "gui-computer-input-observe-3",
        )?)?;
        fixture.a11y_observed.store(true, Ordering::SeqCst);
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-set",
            "gui-computer-input-observe-3",
            "set_element_value",
            json!({"ref":reference,"text":"alpha beta"}),
        )?));
    }
    if call("gui-computer-input-observe-2") {
        let result = computer_result_text(body, "gui-computer-input-observe-2")?;
        if accessibility_snapshot_text(result)
            .ok()
            .is_some_and(|text| text.contains("computer-input.txt"))
        {
            let reference = text_editor_ref(result)?;
            fixture.a11y_observed.store(true, Ordering::SeqCst);
            return Ok(Some(computer_input_call(
                body,
                "gui-computer-input-set",
                "gui-computer-input-observe-2",
                "set_element_value",
                json!({"ref":reference,"text":"alpha beta"}),
            )?));
        }
        std::thread::sleep(Duration::from_millis(250));
        return Ok(Some((
            "gui-computer-input-observe-3".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-observe") {
        let result = computer_result_text(body, "gui-computer-input-observe")?;
        if accessibility_snapshot_text(result)
            .ok()
            .is_none_or(|text| !text.contains("computer-input.txt"))
        {
            std::thread::sleep(Duration::from_millis(250));
            return Ok(Some((
                "gui-computer-input-observe-2".into(),
                browser_tool(body, "computer/a11y.observe")?,
                json!({"action":"observe"}),
            )));
        }
        let reference = text_editor_ref(computer_result_text(
            body,
            "gui-computer-input-observe",
        )?)?;
        fixture.a11y_observed.store(true, Ordering::SeqCst);
        return Ok(Some(computer_input_call(
            body,
            "gui-computer-input-set",
            "gui-computer-input-observe",
            "set_element_value",
            json!({"ref":reference,"text":"alpha beta"}),
        )?));
    }
    if call("gui-computer-input-launch") {
        return Ok(Some((
            "gui-computer-input-observe".into(),
            browser_tool(body, "computer/a11y.observe")?,
            json!({"action":"observe"}),
        )));
    }
    if call("gui-computer-input-plan-start") {
        return Ok(Some((
            "gui-computer-input-launch".into(),
            browser_tool(body, "computer/launch")?,
            json!({"action":"launch","target":file,"app":"TextEdit"}),
        )));
    }
    if call("gui-computer-input-launch-guard") {
        anyhow::ensure!(
            computer_result_text(body, "gui-computer-input-launch-guard")?
                .contains("update_plan"),
            "Computer launch did not activate the Engine plan gate"
        );
        return Ok(Some((
            "gui-computer-input-plan-start".into(),
            "update_plan".into(),
            json!({
                "explanation":"Record the exact disposable desktop-control task before retrying the guarded launch effect.",
                "requirements":[{
                    "id":"req-computer-input",
                    "description":"Launch the disposable TextEdit fixture and verify Command, Option and Control modifiers.",
                    "source":{"input":0,"quote":"verify Command, Option and Control modifiers"}
                }],
                "plan":[
                    {"step":"Launch the disposable TextEdit fixture","status":"in_progress"},
                    {"step":"Verify Command, Option and Control input","status":"pending"}
                ]
            }),
        )));
    }
    Ok(Some((
        "gui-computer-input-launch-guard".into(),
        browser_tool(body, "computer/launch")?,
        json!({"action":"launch","target":file,"app":"TextEdit"}),
    )))
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
    let operation: Option<(String, String, Value)> = if fixture.native_url.is_some() {
        match native_operation(&fixture, &body, step) {
            Ok(operation) => operation.map(|(tool, arguments)| {
                (format!("gui-native-{step}"), tool, arguments)
            }),
            Err(error) => {
                *fixture.failure.lock().unwrap() = Some(error.to_string());
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string(),"type":"fixture_error"}}))).into_response();
            }
        }
    } else if fixture.computer_denied {
        match computer_operation(&fixture, &body) {
            Ok(operation) => operation,
            Err(error) => {
                *fixture.failure.lock().unwrap() = Some(error.to_string());
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string(),"type":"fixture_error"}}))).into_response();
            }
        }
    } else if fixture.computer_input {
        match computer_input_operation(&fixture, &body) {
            Ok(operation) => operation,
            Err(error) => {
                *fixture.failure.lock().unwrap() = Some(error.to_string());
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string(),"type":"fixture_error"}}))).into_response();
            }
        }
    } else { None };
    let (delta, reason) = if let Some((call_id, tool, operation)) = operation {
        (json!({"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function","function":{"name":tool,"arguments":operation.to_string()}}]}), "tool_calls")
    } else {
        if !fixture.computer_denied && !fixture.computer_input {
            tokio::select! {
                _=fixture.stop.cancelled()=>{},
                permit=fixture.finish.acquire()=>{ if let Ok(permit)=permit { permit.forget(); } },
            }
        }
        let content = if fixture.computer_denied {
            "Accessibility 已授权；Screen Recording 被 macOS 拒绝（ROLE_HOST_PROVIDER_FAILURE）。"
        } else if fixture.computer_input {
            "已通过正式 Computer Actions 验证 TextEdit 启动、Command/Option/Control 输入与保存。"
        } else {
            "本机测试模型已结束。"
        };
        (json!({"role":"assistant","content":content}), "stop")
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
    let mode = std::env::args().nth(2);
    let native_actions = mode.as_deref() == Some("--native-actions");
    let computer_denied = mode.as_deref() == Some("--computer-denied");
    let computer_input = mode.as_deref() == Some("--computer-input");
    anyhow::ensure!(
        mode.is_none() || live_mode || native_actions || computer_denied || computer_input,
        "unsupported fixture mode"
    );
    let computer_file = if computer_input {
        let path = root.join("computer-input.txt");
        std::fs::write(&path, "seed")?;
        Some(path)
    } else {
        None
    };
    let fixture = Arc::new(Fixture {
        live,
        calls: AtomicUsize::new(0),
        native_url: native_actions.then(|| format!("http://{address}/")),
        computer_denied,
        computer_input,
        computer_file,
        a11y_observed: AtomicBool::new(false),
        screen_denied: AtomicBool::new(false),
        input_verified: AtomicBool::new(false),
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
                Json(json!({"model_calls":f.calls.load(Ordering::SeqCst),"real_provider":f.live.is_some(),"native_actions":f.native_url.is_some(),"computer_denied":f.computer_denied,"computer_input":f.computer_input,"computer_file":f.computer_file.as_ref(),"a11y_observed":f.a11y_observed.load(Ordering::SeqCst),"screen_denied":f.screen_denied.load(Ordering::SeqCst),"input_verified":f.input_verified.load(Ordering::SeqCst),"failure":*f.failure.lock().unwrap(),"witnesses":*f.witnesses.lock().unwrap(),"served_versions":versions.len(),"changed_source_served":versions.last().is_some_and(|source|source!=BROKEN_JS)}))
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
    let browser_resources = Arc::new(BrowserResourceService::new(Arc::new(
        PreparatoryBrowserFactory,
    )));
    let (app, keep_alive) = DesktopServer::start_with_outcome(
        &cli,
        "",
        None,
        None,
        None,
        DesktopHostServices {
            browser_resources: Some(browser_resources),
            ..Default::default()
        },
    )
    .await?;
    let prepared = async {
        let local_key=fixture.live.as_ref().map(|live|live.local_token.strip_prefix("Bearer ").unwrap()).unwrap_or("local-fixture-not-a-secret");
        let provider = api(&app,"/api/providers",json!({"platform":"custom","name":if live_mode {"真实模型前端验收"}else{"本机浏览器验收模型"},"base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":[local_key]},"enabled":true,"initial_model":{"model":"browser-gui-fixture","enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","output_limit":4096}]}})).await?;
        let provider = provider["provider_id"].as_str().ok_or_else(|| anyhow::anyhow!("provider missing"))?.to_owned();
        let display_name = if computer_denied {
            "Computer 权限拒绝验收"
        } else if computer_input {
            "Computer 物理输入验收"
        } else {
            "浏览器主界面验收"
        };
        let editor = api(&app,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":display_name,"model_route_refs":{},"chat_route_records":{},"model":{"provider_id":provider,"model":"browser-gui-fixture"}})).await?;
        let preset = editor["preset"]["preset_id"].as_str().ok_or_else(|| anyhow::anyhow!("preset missing"))?.to_owned();
        let mut draft=editor["draft"].clone();
        if live_mode {
            draft["document"]["persona"] = json!("You are a precise local Browser repair acceptance agent.");
            draft["document"]["instructions"] = json!("Use only the selected real Browser and Workspace Actions. For browser/act click, send exactly {\"action\":\"click\",\"element\":ELEMENT}, where ELEMENT is the complete {reference, role, name, focused} object copied unchanged from the latest browser/observe result. Do not add top-level reference, role, name, focused, or target fields, and do not stringify nested objects. Use the selected Workspace read and patch Actions for source changes inside the bound workspace; do not try to edit source through the page.");
        }
        draft["document"]["enabled_capabilities"] = if computer_denied {
            json!([{
                "capability":{"id":"computer"},
                "action_allowlist":["computer/observe","computer/a11y.observe"]
            }])
        } else if computer_input {
            json!([{
                "capability":{"id":"computer"},
                "action_allowlist":["computer/a11y.observe","computer/input","computer/launch"]
            }])
        } else if live_mode {
            json!([{
                "capability":{"id":"browser"},
                "action_allowlist":["browser/observe","browser/navigate","browser/act"]
            },{
                "capability":{"id":"workspace.files"},
                "action_allowlist":["workspace.files/read","workspace.files/patch"]
            }])
        } else {
            json!([{
                "capability":{"id":"browser"},
                "action_allowlist":["browser/observe","browser/navigate","browser/act"]
            }])
        };
        let revision_path=format!("/api/agent-presets/{preset}/revisions");
        let saved=api(&app,&revision_path,json!({"expected_current_revision":draft["current_revision"].clone(),"draft":draft,"reason":"deterministic native Browser GUI acceptance"})).await?;
        let expected_capabilities=if live_mode {2}else{1};
        anyhow::ensure!(saved["revision"]["document"]["enabled_capabilities"].as_array().is_some_and(|values|values.len()==expected_capabilities),"Browser fixture revision missing selected Module");
        let resources = if computer_denied || computer_input {
            json!([{"resource_kind":"computer","resource_id":"local-desktop"}])
        } else if live_mode {
            json!([
                {"resource_kind":"browser","resource_id":"managed-browser"},
                {"resource_kind":"workspace","resource_id":"default-workspace"}
            ])
        } else {
            json!([{"resource_kind":"browser","resource_id":"managed-browser"}])
        };
        let session = api(&app,"/api/agent-sessions",json!({"preset_id":preset,"title":display_name,"resource_selections":resources,"model":{"provider_id":provider,"model":"browser-gui-fixture"}})).await?;
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
        json!({"data_dir":root,"work_dir":fixture.live.as_ref().map(|live|&live.work),"real_provider":live_mode,"native_actions":native_actions,"computer_denied":computer_denied,"computer_input":computer_input,"computer_file":fixture.computer_file.as_ref(),"page":format!("http://{address}/"),"control":format!("http://{address}"),"session_id":session})
    );
    // Keep only the model/page server alive; the real desktop now owns the DB.
    fixture.stop.cancelled().await;
    Ok(())
}

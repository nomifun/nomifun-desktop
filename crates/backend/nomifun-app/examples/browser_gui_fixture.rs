//! Prepare a NEW disposable main-app dataset and a loopback-only held model.
//! Deterministic modes read no user dataset or real provider credential.
//! Opt-in --live-frontend / --live-commands / --live-general-commands /
//! --live-default-general-commands accept the live key only via stdin.
//! --wk-actions prepares a deterministic real-App WK acceptance task. See
//! scripts/validation/run-macos-wk-app-acceptance.mjs for the authenticated driver.
//! --user-browser-only prepares a normal chat Session with no Browser Module
//! or Browser Resource binding, for testing the independent user sidebar.
//! After that owned GUI dataset's desktop has normally exited, append a second
//! empty Session with: <existing-data-dir> --append-user-browser-session <first-session-id>.
//! The append mode uses only the normal owner-scoped API and starts no model server.
//! Usage: cargo run -p nomifun-app --example browser_gui_fixture -- <new-data-dir> [--user-browser-only|--native-actions|--native-pause|--wk-actions]
//! Launch the real desktop EXE with NOMIFUN_DATA_DIR set to the printed path.
use axum::{
    Json, Router,
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use futures_util::StreamExt;
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
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

#[path = "support/wk_browser_fixture.rs"]
mod wk_browser_fixture;

struct LiveFrontend {
    model: LiveModelSpec,
    key: Zeroizing<String>,
    local_token: String,
    client: reqwest::Client,
    work: PathBuf,
    source: Mutex<PathBuf>,
    served: Mutex<Vec<String>>,
    budget: LiveBudget,
    started: Mutex<Option<Instant>>,
    upstream_requests: AtomicUsize,
    stopped_status: AtomicUsize,
    trace_dir: PathBuf,
    trace_sequence: AtomicUsize,
}

#[derive(Clone, Copy)]
struct LiveModelSpec {
    name: &'static str,
    context_limit: Option<u32>,
}

impl LiveModelSpec {
    fn parse(name: Option<&str>) -> anyhow::Result<Self> {
        match name {
            None | Some("step-3.7-flash") => Ok(Self { name: "step-3.7-flash", context_limit: None }),
            // StepFun's published 1M input window; keep this conservative
            // decimal ceiling, and freeze output/time separately below.
            Some("step-5-preview") => Ok(Self { name: "step-5-preview", context_limit: Some(1_000_000) }),
            _ => anyhow::bail!("unsupported live GUI model selection"),
        }
    }

    fn configure_capability(self, mut capability: Value) -> Value {
        if let Some(limit) = self.context_limit {
            capability["context_limit"] = json!(limit);
            capability["traits"] = json!(["vision_input", "video_input"]);
        }
        capability
    }
}

#[derive(Clone, Copy)]
struct LiveBudget {
    calls: Option<usize>,
    output_tokens: Option<u64>,
    seconds: u64,
}

impl LiveBudget {
    fn parse(
        calls: Option<&str>,
        tokens: Option<&str>,
        seconds: Option<&str>,
    ) -> anyhow::Result<Self> {
        let budget = Self {
            calls: match calls { Some("unlimited") => None, _ => Some(calls.unwrap_or("32").parse()?) },
            output_tokens: match tokens {Some("provider")=>None,_=>Some(tokens.unwrap_or("4096").parse()?)},
            seconds: seconds.unwrap_or("1800").parse()?,
        };
        anyhow::ensure!(
            budget.calls.is_none_or(|calls| (1..=32).contains(&calls))
                && budget.output_tokens.is_none_or(|tokens|(128..=4096).contains(&tokens))
                && (1..=1800).contains(&budget.seconds),
            "live GUI budget outside bounded acceptance range"
        );
        Ok(budget)
    }

    fn reserve(
        &self,
        started: &Mutex<Option<Instant>>,
        calls: &AtomicUsize,
        stopped: &AtomicUsize,
        now: Instant,
    ) -> Result<Duration, axum::http::StatusCode> {
        let mut started = started.lock().unwrap();
        let status = stopped.load(Ordering::SeqCst);
        if status != 0 {
            return Err(axum::http::StatusCode::from_u16(status as u16).unwrap());
        }
        let elapsed = now.saturating_duration_since(*started.get_or_insert(now));
        if elapsed >= Duration::from_secs(self.seconds)
            || self.calls.is_some_and(|limit| calls.load(Ordering::SeqCst) >= limit)
        {
            return Err(stop_live_once(
                stopped,
                axum::http::StatusCode::TOO_MANY_REQUESTS,
            ));
        }
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(Duration::from_secs(self.seconds)
            .saturating_sub(elapsed)
            .min(Duration::from_secs(90)))
    }
}

fn stop_live_once(stopped: &AtomicUsize, status: axum::http::StatusCode) -> axum::http::StatusCode {
    let first = stopped
        .compare_exchange(
            0,
            status.as_u16() as usize,
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .unwrap_or_else(|first| first);
    if first == 0 {
        status
    } else {
        axum::http::StatusCode::from_u16(first as u16).unwrap()
    }
}

fn live_command_template(option: Option<&str>) -> Option<&'static str> {
    match option {
        Some("--live-commands") => Some("coding.codex"),
        Some("--live-general-commands") => Some("assistant.general"),
        Some("--live-default-general-commands") => Some("assistant.general"),
        _ => None,
    }
}

fn command_general_document(mut document: Value, preserve_default: bool) -> anyhow::Result<Value> {
    if preserve_default {
        return Ok(document);
    }
    let selections = document["enabled_capabilities"].as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("General template has no exact capability selections"))?;
    selections.retain(|selection| !matches!(selection["capability"]["id"].as_str(),
        Some("computer" | "automation.schedule")));
    Ok(document)
}

fn command_resource_selections(preserve_default_general: bool) -> Value {
    let mut resources = vec![
        json!({"resource_kind":"workspace","resource_id":"default-workspace"}),
        json!({"resource_kind":"process_session","resource_id":"managed-process-session"}),
        json!({"resource_kind":"project_memory","resource_id":"default-project-memory"}),
    ];
    if preserve_default_general {
        resources.push(json!({"resource_kind":"computer","resource_id":"local-desktop"}));
        resources.push(json!({"resource_kind":"scheduler","resource_id":"installation-scheduler"}));
    }
    Value::Array(resources)
}

#[cfg(test)]
mod live_budget_tests {
    use super::*;
    #[test]
    fn live_model_selection_keeps_default_and_declares_step5_without_budget_override() {
        let default=LiveModelSpec::parse(None).unwrap();
        assert_eq!(default.name,"step-3.7-flash");assert_eq!(default.context_limit,None);
        let step5=LiveModelSpec::parse(Some("step-5-preview")).unwrap();
        assert_eq!(step5.name,"step-5-preview");assert_eq!(step5.context_limit,Some(1_000_000));
        for invalid in ["","step-5","STEP-5-PREVIEW","step-5-preview\n","provider/step-5-preview"] {
            assert!(LiveModelSpec::parse(Some(invalid)).is_err());
        }
        let budget=LiveBudget::parse(Some("unlimited"),Some("4096"),Some("360")).unwrap();
        assert_eq!(budget.output_tokens,Some(4096));assert_eq!(budget.seconds,360);
        let capability = step5.configure_capability(json!({
            "task":"chat", "traits":[], "protocol":"openai.chat_text",
            "connection_role":"default", "output_limit":4096
        }));
        let input: nomifun_api_types::ProviderModelCapabilityInput =
            serde_json::from_value(capability).unwrap();
        assert_eq!(input.context_limit, Some(1_000_000));
        assert_eq!(input.output_limit, Some(4096));
        assert_eq!(input.traits, vec![
            nomifun_api_types::ModelTrait::VisionInput,
            nomifun_api_types::ModelTrait::VideoInput,
        ]);
    }
    #[test]
    fn command_mode_selects_only_the_exact_official_template() {
        assert_eq!(live_command_template(Some("--live-commands")), Some("coding.codex"));
        assert_eq!(live_command_template(Some("--live-general-commands")), Some("assistant.general"));
        assert_eq!(live_command_template(Some("--live-default-general-commands")), Some("assistant.general"));
        for option in [None, Some("--native-actions"), Some("--live-frontend"), Some("--unknown")] {
            assert_eq!(live_command_template(option), None);
        }
    }

    #[test]
    fn general_command_scope_only_removes_unselected_host_resource_modules() {
        let original = json!({"persona":"unchanged general persona", "instructions":"unchanged instructions",
            "model_route_refs":{"chat":"exact-route"}, "enabled_capabilities":[
                {"capability":{"id":"automation.schedule"},"action_allowlist":["automation.schedule/list"]},
                {"capability":{"id":"workspace.process"},"action_allowlist":["workspace.process/start"]}
            ]});
        let scoped = command_general_document(original.clone(), false).unwrap();
        assert_eq!(scoped["enabled_capabilities"], json!([original["enabled_capabilities"][1]]));
        for field in ["persona","instructions","model_route_refs"] { assert_eq!(scoped[field], original[field]); }
        assert_eq!(command_general_document(original.clone(), true).unwrap(), original,
            "default General mode must preserve the entire document, not just selected fields");
    }

    #[test]
    fn default_general_binds_required_resources_without_expanding_scoped_modes() {
        let scoped = command_resource_selections(false);
        let full = command_resource_selections(true);
        assert_eq!(scoped.as_array().unwrap().len(), 3);
        assert_eq!(&full.as_array().unwrap()[..3], scoped.as_array().unwrap());
        assert_eq!(&full.as_array().unwrap()[3..], &[
            json!({"resource_kind":"computer","resource_id":"local-desktop"}),
            json!({"resource_kind":"scheduler","resource_id":"installation-scheduler"}),
        ]);
    }

    #[test]
    fn explicit_budget_is_bounded_and_zero_or_widened_budgets_fail_closed() {
        let budget = LiveBudget::parse(Some("6"), Some("1024"), Some("180")).unwrap();
        assert_eq!(
            (budget.calls, budget.output_tokens, budget.seconds),
            (Some(6), Some(1024), 180)
        );
        for values in [
            ("0", "1024", "180"),
            ("33", "1024", "180"),
            ("6", "0", "180"),
            ("6", "127", "180"),
            ("6", "4097", "180"),
            ("6", "1024", "0"),
            ("6", "1024", "1801"),
            ("invalid", "1024", "180"),
        ] {
            assert!(LiveBudget::parse(Some(values.0), Some(values.1), Some(values.2)).is_err());
        }
    }

    #[test]
    fn call_limit_is_exact_and_local_retries_do_not_reset_it() {
        let budget = LiveBudget::parse(Some("6"), Some("1024"), Some("180")).unwrap();
        let started = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let stopped = AtomicUsize::new(0);
        let now = Instant::now();
        for _ in 0..6 {
            assert_eq!(
                budget.reserve(&started, &calls, &stopped, now),
                Ok(Duration::from_secs(90))
            );
        }
        for _ in 0..20 {
            assert_eq!(
                budget.reserve(&started, &calls, &stopped, now),
                Err(axum::http::StatusCode::TOO_MANY_REQUESTS)
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        assert_eq!(*started.lock().unwrap(), Some(now));
    }

    #[test]
    fn explicitly_unlimited_calls_keep_the_time_and_output_safety_limits() {
        let budget=LiveBudget::parse(Some("unlimited"),Some("4096"),Some("360")).unwrap();
        assert_eq!(budget.calls,None);
        let started=Mutex::new(None);let calls=AtomicUsize::new(0);let stopped=AtomicUsize::new(0);let now=Instant::now();
        for _ in 0..64 {assert!(budget.reserve(&started,&calls,&stopped,now).is_ok());}
        assert_eq!(calls.load(Ordering::SeqCst),64);
        assert!(budget.reserve(&started,&calls,&stopped,now+Duration::from_secs(360)).is_err());
        assert!(LiveBudget::parse(Some("unlimited"),Some("4097"),Some("360")).is_err());
        assert!(LiveBudget::parse(Some("unlimited"),Some("4096"),Some("1801")).is_err());
    }
    #[test]
    fn provider_output_default_is_explicit_and_keeps_only_the_time_boundary() {
        let budget=LiveBudget::parse(Some("unlimited"),Some("provider"),Some("360")).unwrap();
        assert_eq!(budget.calls,None);assert_eq!(budget.output_tokens,None);assert_eq!(budget.seconds,360);
    }

    #[test]
    fn remaining_timeout_and_expiration_use_the_original_monotonic_start() {
        let budget = LiveBudget::parse(Some("6"), Some("1024"), Some("180")).unwrap();
        let started = Mutex::new(None);
        let calls = AtomicUsize::new(0);
        let stopped = AtomicUsize::new(0);
        let now = Instant::now();
        assert_eq!(
            budget.reserve(&started, &calls, &stopped, now),
            Ok(Duration::from_secs(90))
        );
        assert_eq!(
            budget.reserve(&started, &calls, &stopped, now + Duration::from_secs(179)),
            Ok(Duration::from_secs(1))
        );
        assert_eq!(
            budget.reserve(&started, &calls, &stopped, now + Duration::from_secs(180)),
            Err(axum::http::StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(*started.lock().unwrap(), Some(now));
    }

    #[test]
    fn first_upstream_rejection_is_preserved_without_forwarding_local_retries() {
        for code in [401, 403, 429, 502, 503] {
            let budget = LiveBudget::parse(Some("6"), Some("1024"), Some("180")).unwrap();
            let started = Mutex::new(None);
            let calls = AtomicUsize::new(0);
            let stopped = AtomicUsize::new(0);
            let now = Instant::now();
            assert!(budget.reserve(&started, &calls, &stopped, now).is_ok());
            let rejection = axum::http::StatusCode::from_u16(code).unwrap();
            assert_eq!(stop_live_once(&stopped, rejection), rejection);
            assert_eq!(
                stop_live_once(&stopped, axum::http::StatusCode::TOO_MANY_REQUESTS),
                rejection
            );
            for _ in 0..20 {
                assert_eq!(
                    budget.reserve(&started, &calls, &stopped, now),
                    Err(rejection)
                );
            }
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
}
const BROKEN_JS: &str = "function nextCount(value) { return value + 2; }\n";
fn install_live_source(work: &std::path::Path, session_id: &str) -> anyhow::Result<PathBuf> {
    nomifun_common::validate_uuidv7(session_id)?;
    let conversations=work.join("conversations");
    let session_work=conversations.join(session_id);
    std::fs::create_dir_all(&session_work)?;
    let canonical_conversations=conversations.canonicalize()?;
    let canonical_session=session_work.canonicalize()?;
    anyhow::ensure!(canonical_session.parent()==Some(canonical_conversations.as_path()),"live source workspace is not the exact managed Session child");
    let source=canonical_session.join("app.js");
    std::fs::write(&source,BROKEN_JS)?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_source_uses_the_exact_managed_session_workspace() {
        let root=tempfile::tempdir().unwrap();
        let work=root.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let session=uuid::Uuid::now_v7().to_string();
        let source=install_live_source(&work,&session).unwrap();
        let expected=work.join("conversations").join(&session).canonicalize().unwrap();
        assert_eq!(source.parent(),Some(expected.as_path()));
        assert_eq!(std::fs::read_to_string(source).unwrap(),BROKEN_JS);
        assert!(!work.join("app.js").exists());
    }

}

const FRONTEND_PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>Counter app</title>
<style>body{font:20px system-ui;padding:36px}button{font:inherit;padding:12px 24px}output{display:block;font-size:32px;margin:24px 0}</style>
<h1>Counter app</h1><label for=count>Count</label><output id=count>0</output><button id=increment>Increment</button>
<script src=/app.js></script><script>(()=>{let value=0;const nonce=crypto.randomUUID();const send=window.fetch.bind(window);increment.onclick=e=>{if(!e.isTrusted)return;value=nextCount(value);document.getElementById('count').textContent=String(value);void send('/witness',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({nonce,count:value,trusted:e.isTrusted})});};})();</script>"#;

struct Fixture {
    live: Option<LiveFrontend>,
    calls: AtomicUsize,
    native_url: Option<String>,
    native_pause: bool,
    wk_actions: bool,
    wk_evidence: Mutex<serde_json::Map<String, Value>>,
    wk_screenshot: Option<PathBuf>,
    witnesses: Mutex<Vec<Value>>,
    failure: Mutex<Option<String>>,
    finish: Semaphore,
    stop: CancellationToken,
}

/// Declares the native Browser provider while the disposable dataset is being
/// compiled, but can never execute a page. The subsequently launched desktop
/// replaces this preparatory host with the real platform factory before any
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
    if fixture.native_pause && step >= 8 {
        return Ok(Some((browser_tool(body, "browser/observe")?, json!({}))));
    }
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
            "summary":"One initial guarded Browser effect was rejected before planning. The packaged native Browser then navigated, typed Unicode text and delivered a trusted click witness.",
            "observed_tool_error_count":1,
            "criteria":[
                {
                    "step":"Verify packaged native Browser navigation and input",
                    "disposition":"supported",
                    "evidence_call_ids":["gui-native-7"],
                    "rationale":"The final observation reflects the native type action result after all effects settled."
                },
                {
                    "step":"Verify packaged native Browser page result",
                    "disposition":"supported",
                    "evidence_call_ids":["gui-native-7"],
                    "rationale":"The final observation contains the trusted click result."
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

fn tool_result<'a>(body: &'a Value, call_id: &str) -> Option<&'a Value> {
    body["messages"].as_array().and_then(|messages| messages.iter().find(|message| {
        message["role"] == "tool" && message["tool_call_id"].as_str() == Some(call_id)
    }))
}

fn tool_result_text<'a>(body: &'a Value, call_id: &str) -> anyhow::Result<&'a str> {
    let message = tool_result(body, call_id)
        .ok_or_else(|| anyhow::anyhow!("Tool result missing for {call_id}"))?;
    message["content"].as_str().or_else(|| {
        message["content"].as_array().and_then(|parts| parts.iter().find_map(|part| {
            (part["type"].as_str() == Some("text")).then(|| part["text"].as_str()).flatten()
        }))
    }).ok_or_else(|| anyhow::anyhow!("Tool text result missing for {call_id}"))
}

#[cfg(test)]
fn contains_inline_png(value: &Value) -> bool {
    match value {
        Value::String(value) => value.starts_with("data:image/png;base64,"),
        Value::Array(values) => values.iter().any(contains_inline_png),
        Value::Object(values) => values.values().any(contains_inline_png),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

async fn model(State(fixture): State<Arc<Fixture>>, headers: axum::http::HeaderMap, Json(mut body): Json<Value>) -> axum::response::Response {
    if let Some(live) = &fixture.live {
        if headers.get("authorization").and_then(|value|value.to_str().ok()) != Some(live.local_token.as_str()) {
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
        if !body.is_object() || !body["messages"].is_array() { return axum::http::StatusCode::BAD_REQUEST.into_response(); }
        let timeout = match live.budget.reserve(&live.started, &fixture.calls, &live.stopped_status, Instant::now()) {
            Ok(timeout) => timeout,
            Err(status) => return status.into_response(),
        };
        body["model"] = json!(live.model.name);
        if let Some(tokens)=live.budget.output_tokens {
            body["max_tokens"] = json!(tokens);
            body.as_object_mut().unwrap().remove("max_completion_tokens");
        }
        body["temperature"] = json!(0);
        body["stream"] = json!(true);
        let number = live.trace_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let request_path = live.trace_dir.join(format!("request-{number:02}.json"));
        let response_path = live.trace_dir.join(format!("response-{number:02}.sse"));
        let trace = std::fs::write(request_path, serde_json::to_vec(&body).unwrap())
            .and_then(|_| std::fs::OpenOptions::new().write(true).create_new(true).open(response_path));
        let mut trace = match trace {
            Ok(trace) => trace,
            Err(_) => {
                *fixture.failure.lock().unwrap() = Some("model_trace_unavailable".into());
                return stop_live_once(&live.stopped_status, axum::http::StatusCode::INTERNAL_SERVER_ERROR).into_response();
            }
        };
        live.upstream_requests.fetch_add(1, Ordering::SeqCst);
        let response = tokio::select! {
            _=fixture.stop.cancelled()=>return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
            response=live.client.post("https://api.stepfun.com/step_plan/v1/chat/completions")
                .timeout(timeout)
                .bearer_auth(live.key.as_str()).json(&body).send()=>response,
        };
        return match response {
            Ok(response) if response.status().is_success() => {
                let tracing_fixture = Arc::clone(&fixture);
                let mut bytes = 0usize;
                let stream = response.bytes_stream().map(move |part| {
                    use std::io::Write;
                    let result = part.map_err(std::io::Error::other).and_then(|part| {
                        if bytes.saturating_add(part.len()) > 8 * 1024 * 1024 {
                            return Err(std::io::Error::other("model response trace exceeded bounded limit"));
                        }
                        trace.write_all(&part)?;
                        bytes += part.len();
                        Ok(part)
                    });
                    if result.is_err() {
                        *tracing_fixture.failure.lock().unwrap() = Some("model_stream_or_trace_failed".into());
                        stop_live_once(&tracing_fixture.live.as_ref().unwrap().stopped_status, axum::http::StatusCode::BAD_GATEWAY);
                    }
                    result
                });
                axum::response::Response::builder().header("content-type","text/event-stream")
                    .body(axum::body::Body::from_stream(stream)).unwrap()
            },
            Ok(response) => {
                let status = response.status().as_u16();
                let stopped = stop_live_once(&live.stopped_status, axum::http::StatusCode::from_u16(status).unwrap());
                *fixture.failure.lock().unwrap()=Some(format!("upstream_status_{status}"));
                (stopped, Json(json!({"error":{"message":"Live test provider rejected the request"}}))).into_response()
            }
            Err(_) => {
                let stopped = stop_live_once(&live.stopped_status, axum::http::StatusCode::BAD_GATEWAY);
                *fixture.failure.lock().unwrap()=Some("upstream_transport_failed".into());
                stopped.into_response()
            }
        };
    }
    let call = fixture.calls.fetch_add(1, Ordering::SeqCst);
    if fixture.wk_actions && body["messages"].as_array().and_then(|messages| messages.iter().rev().find(|message|message["role"]=="user"))
        .is_some_and(|message|message["content"].to_string().contains("WK_UPGRADE_PRESERVATION_SEED")) {
        // A real completed canonical Turn predates the new App launch. No
        // Browser call or historical Agent-data import is needed to seed it.
        let chunk=|delta:Value,finish:Option<&str>|json!({"id":"wk-upgrade-seed","object":"chat.completion.chunk","created":1,"model":"browser-gui-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string();
        return ([("content-type","text/event-stream")],format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",chunk(json!({"role":"assistant","content":"WK_UPGRADE_PRESERVED_REPLY"}),None),chunk(json!({}),Some("stop")))).into_response();
    }
    // Local-only protocol fault: enough canonical events to put the pause past
    // the first 500-event consumer page, followed by the normal rate-limit path.
    if fixture.native_pause && call >= 45 {
        return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if !body["tools"].is_array() {
        let chunk = |delta: Value, finish: Option<&str>| {
            json!({"id":"gui-fixture-compact","object":"chat.completion.chunk","created":1,"model":"browser-gui-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
        };
        return (
            [("content-type", "text/event-stream")],
            format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                chunk(json!({"role":"assistant","content":"Preserve completed tool results and continue the requested task without replaying prior actions."}), None),
                chunk(json!({}), Some("stop"))
            ),
        ).into_response();
    }
    // Ordinary fixtures reset per Turn. The one-Turn pause fixture keeps its
    // monotonic request counter across context compaction.
    let step = if fixture.wk_actions { wk_browser_fixture::step(&body) } else if fixture.native_pause { call } else { body["messages"].as_array().map(|messages| messages.iter().rev()
        .take_while(|message| message["role"] != "user")
        .filter(|message| message["role"] == "tool").count()).unwrap_or(0) };
    let operation: Option<(String, String, Value)> = if fixture.wk_actions {
        match wk_browser_fixture::operation(&fixture, &body, step) {
            Ok(operation) => operation.map(|(tool, arguments)| (wk_browser_fixture::call_id(&body,step), tool, arguments)),
            Err(error) => {
                fixture.failure.lock().unwrap().get_or_insert_with(|| error.to_string());
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string(),"type":"wk_fixture_error"}}))).into_response();
            }
        }
    } else if fixture.native_url.is_some() {
        match native_operation(&fixture, &body, step) {
            Ok(operation) => operation.map(|(tool, arguments)| {
                (format!("gui-native-{step}"), tool, arguments)
            }),
            Err(error) => {
                *fixture.failure.lock().unwrap() = Some(error.to_string());
                return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string(),"type":"fixture_error"}}))).into_response();
            }
        }
    } else { None };
    let (delta, reason) = if let Some((call_id, tool, operation)) = operation {
        (json!({"role":"assistant","tool_calls":[{"index":0,"id":call_id,"type":"function","function":{"name":tool,"arguments":operation.to_string()}}]}), "tool_calls")
    } else {
        tokio::select! {
            _=fixture.stop.cancelled()=>{},
            permit=fixture.finish.acquire()=>{ if let Ok(permit)=permit { permit.forget(); } },
        }
        let content = "本机测试模型已结束。";
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

async fn fixture_read(app: &DesktopServer, path: &str) -> anyhow::Result<Value> {
    let response=reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build()?
        .get(format!("http://127.0.0.1:{}{path}",app.loopback_port()))
        .header("x-nomi-local-trust",app.local_trust_secret()).send().await?;
    anyhow::ensure!(response.status().is_success(),"Fixture read failed: {path}");
    let value:Value=response.json().await?;
    Ok(value["data"].clone())
}

fn require_empty_user_browser_session(
    observed: &Value,
    session_id: &str,
    title: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        observed["session"]["agent_session_id"] == session_id
            && observed["head"]["session_id"] == session_id
            && observed["session"]["metadata"]["title"] == title
            && observed["session"]["metadata"]["archived"] == false
            && observed["session"]["owner_ref"]["principal_kind"] == "user"
            && observed["session"]["remote_binding_provenance"].is_null()
            && observed["session"]["parent_session_id"].is_null(),
        "Expected the exact owned standalone user-browser fixture Session"
    );
    anyhow::ensure!(
        observed["session"]["agent_binding"]["typed_resource_bindings"]
            .as_array().is_some_and(Vec::is_empty)
            && observed["head"]["active_turn_id"].is_null()
            && observed["head"]["active_set_generation"] == 0,
        "User-browser fixture must have no resources or active Turn; resource_count={:?}, active_turn_present={}, generation={:?}, message_count={:?}",
        observed["session"]["agent_binding"]["typed_resource_bindings"].as_array().map(Vec::len),
        !observed["head"]["active_turn_id"].is_null(),
        observed["head"]["active_set_generation"].as_u64(),
        observed["messages"].as_array().map(Vec::len)
    );
    let events = observed["events"].as_array()
        .ok_or_else(|| anyhow::anyhow!("Canonical fixture events missing"))?;
    anyhow::ensure!(
        !events.is_empty() && events.len() <= 3
            && events.iter().enumerate().all(|(index, event)| {
                event["seq"].as_u64() == Some(index as u64 + 1)
                    && matches!(event["kind"].as_str(), Some(
                        "session/opening" | "capability/active-set-committed" | "session/ready"
                    ))
            })
            && events[0]["kind"] == "session/opening"
            && observed["head"]["last_seq"].as_u64() == Some(events.len() as u64)
            && observed["session"]["next_seq"].as_u64() == Some(events.len() as u64 + 1),
        "Fixture has non-initial canonical history; zero Turns cannot be proved"
    );
    // The current registry projects opening into session_status; ready only
    // reduces the head. Its status row is initialization, not chat or a Turn.
    let opening = &events[0];
    let correlation = opening["correlation_id"].as_str()
        .ok_or_else(|| anyhow::anyhow!("Opening status correlation missing"))?;
    let identity = format!("session:{correlation}");
    let opening_id = opening["event_id"].as_str()
        .ok_or_else(|| anyhow::anyhow!("Opening event identity missing"))?;
    let expected = json!({"projection_id":identity,"correlation_id":correlation,
        "presentation_intent":"session_status","state":"opening",
        "reference":{"last_event_id":opening_id}});
    let messages = observed["messages"].as_array()
        .ok_or_else(|| anyhow::anyhow!("Canonical status projections missing"))?;
    anyhow::ensure!(messages.len() == 1, "Fixture has additional non-initial message projections");
    let message = &messages[0];
    anyhow::ensure!(message["session_id"] == session_id && message["projection_id"] == identity
        && message["first_seq"] == 1 && message["last_seq"] == 1
        && message["presentation_intent"] == "session_status"
        && message["message_type"].is_null() && message["message_status"].is_null()
        && message["projection"] == expected,
        "Fixture projection differs from exact canonical initialization status");
    Ok(())
}

async fn append_user_browser_session(root: PathBuf, first_id: &str) -> anyhow::Result<()> {
    nomifun_common::validate_uuidv7(first_id)?;
    anyhow::ensure!(root.is_absolute() && root.is_dir(), "Existing absolute fixture data directory required");
    // This opt-in branch accepts the exact previously prepared GUI test root,
    // not an arbitrary user dataset. The normal server still owns its dataset
    // lock and startup validation. The caller must first prove the desktop exit.
    let launch_path = root.parent().ok_or_else(|| anyhow::anyhow!("Fixture parent missing"))?
        .join("gui-launch-evidence.json");
    let launch: Value = serde_json::from_slice(&std::fs::read(launch_path)?)?;
    let prepared_root = PathBuf::from(launch["data_directory"].as_str()
        .ok_or_else(|| anyhow::anyhow!("Prepared GUI data root missing"))?);
    let work_root = PathBuf::from(launch["work_directory"].as_str()
        .ok_or_else(|| anyhow::anyhow!("Prepared GUI work root missing"))?);
    anyhow::ensure!(
        launch["scope"] == "production-main-source-debug-vite-test-namespace"
            && launch["canonical_session_id"] == first_id
            && std::fs::canonicalize(&prepared_root)? == std::fs::canonicalize(&root)?
            && work_root.is_absolute() && work_root.is_dir()
            && root.join("nomifun-backend.db").is_file(),
        "Append requires the exact previously owned GUI fixture root and first Session"
    );
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".into(), port: 0, data_dir: root.clone(),
        work_dir: Some(work_root), app_version: env!("CARGO_PKG_VERSION").into(),
        local: true, log_dir: Some(root.join("logs")), log_level: Some("off".into()), command: None,
    };
    let browser_resources = Arc::new(BrowserResourceService::new(Arc::new(PreparatoryBrowserFactory)));
    let (app, keep_alive) = match DesktopServer::start_with_outcome(&cli, "", None, None, None,
        DesktopHostServices { browser_resources: Some(browser_resources), ..Default::default() }).await {
        Ok(started) => started,
        Err(mut failure) => {
            if !failure.cleanup_verified() {
                let Some(retained) = failure.take_retained_keep_alive() else {
                    eprintln!("GUI fixture startup cleanup is unverified without retry authority; retaining runtime");
                    std::future::pending::<()>().await;
                    return Err(failure.into_inner());
                };
                loop {
                    match retained.shutdown_after_startup_failure().await {
                        Ok(()) => break,
                        Err(error) => {
                            eprintln!("GUI fixture startup cleanup retry pending: {error:#}");
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
                drop(retained);
            }
            return Err(failure.into_inner());
        }
    };
    let prepared = async {
        const FIRST_TITLE: &str = "内置浏览器独立用户入口验收";
        const SECOND_TITLE: &str = "内置浏览器第二独立会话验收";
        let first_path = format!("/api/agent-sessions/{first_id}");
        let before = fixture_read(&app, &first_path).await?;
        require_empty_user_browser_session(&before, first_id, FIRST_TITLE)?;
        let reference = before["session"]["agent_binding"]["preset_revision_ref"].clone();
        let preset_id = reference["preset_id"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Fixture Preset identity missing"))?;
        let editor = fixture_read(&app, &format!("/api/agent-presets/{preset_id}/editor")).await?;
        let grants = editor["draft"]["document"]["enabled_capabilities"].as_array()
            .ok_or_else(|| anyhow::anyhow!("Fixture capability grants missing"))?;
        let capability_ids = grants.iter().map(|grant| grant["capability"]["id"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Fixture capability identity missing")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        anyhow::ensure!(
            editor["preset"]["display_name"] == FIRST_TITLE
                && editor["draft"]["current_revision"] == reference
                && !capability_ids.contains(&"browser"),
            "Fixture Preset guard failed; title_matches={}, reference_matches={}, capability_ids={:?}",
            editor["preset"]["display_name"] == FIRST_TITLE,
            editor["draft"]["current_revision"] == reference,
            capability_ids
        );
        // This is the same product model selection the UI sends, derived by the
        // API from the saved canonical binding. No projection text is used as
        // Runtime history, authority or a fallback for canonical facts.
        let projection = fixture_read(&app, &format!("{first_path}/projection")).await?;
        anyhow::ensure!(
            projection["conversation_id"] == first_id
                && projection["agent_snapshot"]["canonical_binding"] == before["session"]["agent_binding"]
                && projection["agent_snapshot"]["enabled_capabilities"] == json!(capability_ids),
            "Fixture model presentation does not match its canonical binding"
        );
        let model = json!({
            "provider_id": projection["model"]["provider_id"].as_str()
                .ok_or_else(|| anyhow::anyhow!("Fixture model Provider missing"))?,
            "model": projection["model"]["model"].as_str()
                .ok_or_else(|| anyhow::anyhow!("Fixture model name missing"))?
        });
        anyhow::ensure!(model["model"] == "browser-gui-fixture", "Append requires the local synthetic GUI model");
        let response = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build()?
            .post(format!("http://127.0.0.1:{}/api/agent-sessions", app.loopback_port()))
            .header("x-nomi-local-trust", app.local_trust_secret())
            .header("idempotency-key", format!("browser-gui-secondary:{first_id}"))
            .json(&json!({"preset_id":preset_id,"title":SECOND_TITLE,"model":model,"resource_selections":[]}))
            .send().await?;
        anyhow::ensure!(response.status().is_success(), "Formal secondary Session creation failed");
        let created: Value = response.json().await?;
        let second_id = created["data"]["agent_session_id"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Secondary Session identity missing"))?;
        anyhow::ensure!(second_id != first_id, "Secondary Session reused the first identity");
        let second = fixture_read(&app, &format!("/api/agent-sessions/{second_id}")).await?;
        require_empty_user_browser_session(&second, second_id, SECOND_TITLE)?;
        let second_projection = fixture_read(&app, &format!("/api/agent-sessions/{second_id}/projection")).await?;
        anyhow::ensure!(
            second["session"]["agent_binding"]["preset_revision_ref"] == reference
                && second["session"]["owner_ref"] == before["session"]["owner_ref"]
                && second_projection["model"]["provider_id"] == model["provider_id"]
                && second_projection["model"]["model"] == model["model"]
                && second_projection["agent_snapshot"]["enabled_capabilities"] == projection["agent_snapshot"]["enabled_capabilities"],
            "Secondary Session did not preserve the exact fixture Preset/model contract"
        );
        let after = fixture_read(&app, &first_path).await?;
        anyhow::ensure!(before == after, "Preparing the secondary Session changed the first canonical Session");
        Ok::<_, anyhow::Error>(json!({"data_dir":root,"first_session_id":first_id,"secondary_session_id":second_id,
            "preset_revision":reference,"model":model,"first_session_unchanged":true,
            "browser_resource_bindings":0,"browser_capability_grants":0,"enabled_capability_ids":capability_ids,
            "turns":0,"model_server_started":false}))
    }.await;
    // Always run the normal lifecycle, including validation failures. The real
    // desktop may take ownership again only after this succeeds.
    loop {
        match app.shutdown_all().await {
            Ok(()) => break,
            Err(error) => {
                eprintln!("GUI fixture cleanup retry pending; retaining server ownership: {error:#}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    drop(app);
    drop(keep_alive);
    println!("BROWSER_GUI_SECONDARY_SESSION_READY {}", prepared?);
    Ok(())
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("new absolute data directory required"))?,
    );
    if std::env::args().nth(2).as_deref() == Some("--append-user-browser-session") {
        let first_id = std::env::args().nth(3)
            .ok_or_else(|| anyhow::anyhow!("Exact first GUI Session identity required"))?;
        anyhow::ensure!(std::env::args().nth(4).is_none(), "Unexpected append fixture arguments");
        return append_user_browser_session(root, &first_id).await;
    }
    anyhow::ensure!(
        root.is_absolute() && !root.exists(),
        "refusing an existing or relative data directory"
    );
    let command_option = std::env::args().nth(2);
    let command_template = live_command_template(command_option.as_deref());
    let preserve_default_general = command_option.as_deref() == Some("--live-default-general-commands");
    anyhow::ensure!(command_template != Some("assistant.general") || cfg!(feature = "computer-use"),
        "general command fixture requires computer-use provider registration; build with browser-use,computer-use (system permissions are unchanged)");
    std::fs::create_dir(&root)?;
    let live_commands = command_template.is_some();
    let live_mode = live_commands || std::env::args().nth(2).as_deref() == Some("--live-frontend");
    let live = if live_mode {
        use std::io::{IsTerminal, Read};
        anyhow::ensure!(std::env::var_os("NOMIFUN_LIVE_STEPFUN_API_KEY").is_none() && !std::io::stdin().is_terminal(), "Live key must arrive only through stdin");
        let mut key = Zeroizing::new(String::new());
        std::io::stdin().lock().take(16385).read_to_string(&mut key)?;
        anyhow::ensure!(key.len()<=16384 && !key.trim().is_empty() && !key.trim().contains(['\r','\n']), "Invalid live key input");
        let work = root.join("work");
        std::fs::create_dir(&work)?;
        let source = work.join("app.js");
        let trace_dir = root.join("model-traces");
        std::fs::create_dir(&trace_dir)?;
        let budget = LiveBudget::parse(
            std::env::var("NOMIFUN_LIVE_GUI_CALL_LIMIT").ok().as_deref(),
            std::env::var("NOMIFUN_LIVE_GUI_OUTPUT_LIMIT").ok().as_deref(),
            std::env::var("NOMIFUN_LIVE_GUI_SECONDS").ok().as_deref(),
        )?;
        let model=LiveModelSpec::parse(std::env::var("NOMIFUN_LIVE_GUI_MODEL").ok().as_deref())?;
        Some(LiveFrontend { model,key:Zeroizing::new(key.trim().to_owned()),local_token:format!("Bearer {}",uuid::Uuid::new_v4()),
            client:reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(90)).build()?,
            work,source:Mutex::new(source),served:Mutex::new(vec![]), budget, started:Mutex::new(None),
            upstream_requests:AtomicUsize::new(0),stopped_status:AtomicUsize::new(0),trace_dir,trace_sequence:AtomicUsize::new(0) })
    } else { None };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let mode = std::env::args().nth(2);
    let user_browser_only = mode.as_deref() == Some("--user-browser-only");
    let native_pause = mode.as_deref() == Some("--native-pause");
    let wk_actions = mode.as_deref() == Some("--wk-actions");
    let native_actions = mode.as_deref() == Some("--native-actions") || native_pause || wk_actions;
    anyhow::ensure!(
        mode.is_none()
            || user_browser_only
            || live_mode
            || native_actions,
        "unsupported fixture mode"
    );
    let fixture = Arc::new(Fixture {
        live,
        calls: AtomicUsize::new(0),
        native_url: native_actions.then(|| format!("http://{address}/")),
        native_pause,
        wk_actions,
        wk_evidence: Mutex::new(serde_json::Map::new()),
        wk_screenshot: wk_actions.then(||root.join(format!("wk-visible-page-{}.png",uuid::Uuid::new_v4()))),
        witnesses: Mutex::new(Vec::new()),
        failure: Mutex::new(None),
        finish: Semaphore::new(0),
        stop: CancellationToken::new(),
    });
    let routes = Router::new()
        .route("/", get(|State(f):State<Arc<Fixture>>| async move {
            eprintln!("BROWSER_GUI_FIXTURE_HTTP_GET timestamp_ms={}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
            ([("cache-control","no-store")],Html(if f.live.is_some() {FRONTEND_PAGE} else if f.wk_actions {wk_browser_fixture::PAGE} else {PAGE}))
        }))
        .route("/app.js", get(|State(f):State<Arc<Fixture>>| async move {
            if let Some(live)=&f.live {
                let source_path=live.source.lock().unwrap().clone();
                if let Ok(source)=tokio::fs::read_to_string(source_path).await {
                    if source.len()<=65536 {
                        let mut served=live.served.lock().unwrap(); if served.len()<64 {served.push(source.clone());}
                        return ([("content-type","text/javascript; charset=utf-8"),("cache-control","no-store")],source).into_response();
                    }
                }
            }
            axum::http::StatusCode::NOT_FOUND.into_response()
        }))
        .route("/v1/chat/completions", post(model))
        .route("/download", get(|| async {
            ([("content-type", "text/plain; charset=utf-8"), ("content-disposition", "attachment; filename=wk-acceptance.txt")], "NomiFun WK user download fixture\n")
        }))
        .route("/popup", get(||async {Html("<!doctype html><meta charset=utf-8><title>WK ordinary popup</title><style>body{font:20px system-ui;margin:32px}button,input{font:inherit;margin:12px;padding:8px}</style><h1>普通网页弹窗已打开</h1><label for=popup-field>弹窗输入</label><input id=popup-field><button onclick=\"document.getElementById('popup-result').textContent=document.getElementById('popup-field').value\">弹窗回显</button><output id=popup-result></output>")}))
        .route("/witness", post(|State(f): State<Arc<Fixture>>, Json(value): Json<Value>| async move {
            let mut value=value;
            if f.wk_actions && value.is_object() {
                value["received_at_ms"]=json!(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
            }
            let mut witnesses = f.witnesses.lock().unwrap();
            if witnesses.len() < 64 { witnesses.push(value); }
            axum::http::StatusCode::NO_CONTENT
        }))
        .route(
            "/status",
            get(|State(f): State<Arc<Fixture>>| async move {
                let versions=f.live.as_ref().map(|live|live.served.lock().unwrap().clone()).unwrap_or_default();
                let mut status=serde_json::Map::new();
                let sections=[
                    json!({"model_calls":f.calls.load(Ordering::SeqCst),"real_provider":f.live.is_some(),
                        "live_model":f.live.as_ref().map(|live|live.model.name),
                        "live_context_limit":f.live.as_ref().and_then(|live|live.model.context_limit),
                        "upstream_requests":f.live.as_ref().map(|live| live.upstream_requests.load(Ordering::SeqCst)),
                        "live_call_limit":f.live.as_ref().map(|live|live.budget.calls),
                        "live_output_limit":f.live.as_ref().map(|live|live.budget.output_tokens),
                        "live_seconds":f.live.as_ref().map(|live|live.budget.seconds),
                        "live_stopped_status":f.live.as_ref().map(|live|live.stopped_status.load(Ordering::SeqCst)),
                        "native_actions":f.native_url.is_some(),"native_pause":f.native_pause,"wk_actions":f.wk_actions}),
                    json!({
                        "failure":f.failure.lock().unwrap().clone(),"witnesses":f.witnesses.lock().unwrap().clone(),"served_versions":versions.len(),
                        "changed_source_served":versions.last().is_some_and(|source|source!=BROKEN_JS),
                        "wk_evidence":f.wk_evidence.lock().unwrap().clone()})
                ];
                for section in sections { if let Value::Object(fields)=section { status.extend(fields); } }
                Json(Value::Object(status))
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
        if wk_actions {
            // This file exists only inside the newly created disposable root.
            // The driver logs in normally; no desktop trust secret is exported.
            let credentials = json!({"username":"wkfixture","password":uuid::Uuid::new_v4().to_string()});
            api(&app,"/api/auth/setup",credentials.clone()).await?;
            let mut options=std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)] {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            let mut file=options.open(root.join("wk-acceptance-auth.json"))?;
            file.write_all(&serde_json::to_vec(&credentials)?)?;
        }
        let local_key=fixture.live.as_ref().map(|live|live.local_token.strip_prefix("Bearer ").unwrap()).unwrap_or("local-fixture-not-a-secret");
        let model_traits = if wk_actions { json!(["vision_input"]) } else { json!([]) };
        let model_spec=fixture.live.as_ref().map(|live|live.model);
        let model_name=if live_commands {model_spec.expect("live command model frozen").name}else{"browser-gui-fixture"};
        let mut chat_capability=json!({"task":"chat","traits":model_traits,"protocol":"openai.chat_text","connection_role":"default","output_limit":4096});
        if live_commands {
            chat_capability = model_spec.expect("live command model frozen")
                .configure_capability(chat_capability);
            match fixture.live.as_ref().expect("live budget frozen").budget.output_tokens {
                Some(tokens)=>chat_capability["output_limit"]=json!(tokens),
                None=>{chat_capability.as_object_mut().unwrap().remove("output_limit");},
            }
        }
        let provider = api(&app,"/api/providers",json!({"platform":if live_commands {"stepfun-plan"}else{"custom"},"name":if live_mode {"真实模型前端验收"}else{"本机浏览器验收模型"},"base_url":format!("http://{address}/v1"),"auth_scheme":"bearer","credentials":{"api_keys":[local_key]},"enabled":true,"initial_model":{"model":model_name,"enabled":true,"capabilities":[chat_capability]}})).await?;
        let provider = provider["provider_id"].as_str().ok_or_else(|| anyhow::anyhow!("provider missing"))?.to_owned();
        if live_commands {
            let model = json!({"provider_id":provider,"model":model_name});
            let template = command_template.expect("live command mode has an exact template");
            let general = template == "assistant.general";
            let editor = api(&app,&format!("/api/agent-presets/from-template/{template}"),json!({
                "reuse_existing":false,"display_name":if preserve_default_general {"MAC-GEN 默认通用命令验收"}else if general {"MAC-GEN 通用命令验收"}else{"MAC-A 编程命令验收"},"model":model
            })).await?;
            let selected = if general {
                let document = command_general_document(editor["draft"]["document"].clone(), preserve_default_general)?;
                if preserve_default_general {
                    anyhow::ensure!(document == editor["draft"]["document"], "Default General document changed");
                    editor
                } else {
                    api(&app,"/api/agent-presets",json!({"display_name":"MAC-GEN 通用命令减权验收",
                        "document":document})).await?
                }
            } else { editor };
            let preset = selected["preset"]["preset_id"].as_str().ok_or_else(||anyhow::anyhow!("Official-derived command preset missing"))?;
            let session = api(&app,"/api/agent-sessions",json!({
                "preset_id":preset,"title":if preserve_default_general {"MAC-GEN 默认通用结果交付"}else if general {"MAC-GEN 目录观察"}else{"MAC-A 观察、只读、小测试"},"model":model,
                "resource_selections":command_resource_selections(preserve_default_general)
            })).await?;
            return Ok::<_,anyhow::Error>(Value::String(session["agent_session_id"].as_str()
                .ok_or_else(||anyhow::anyhow!("Official command Session missing"))?.to_owned()));
        }
        let display_name = if user_browser_only {
            "内置浏览器独立用户入口验收"
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
        draft["document"]["enabled_capabilities"] = if user_browser_only {
            json!([])
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
        let expected_capabilities=if user_browser_only {0}else if live_mode {2}else{1};
        anyhow::ensure!(saved["revision"]["document"]["enabled_capabilities"].as_array().is_some_and(|values|values.len()==expected_capabilities),"Browser fixture revision missing selected Module");
        let resources = if user_browser_only {
            json!([])
        } else if live_mode {
            json!([
                {"resource_kind":"browser","resource_id":"managed-browser"},
                {"resource_kind":"workspace","resource_id":"default-workspace"}
            ])
        } else {
            json!([{"resource_kind":"browser","resource_id":"managed-browser"}])
        };
        let session = api(&app,"/api/agent-sessions",json!({"preset_id":preset,"title":display_name,"resource_selections":resources,"model":{"provider_id":provider,"model":"browser-gui-fixture"}})).await?;
        let session_id=session["agent_session_id"].as_str().ok_or_else(||anyhow::anyhow!("session id missing"))?.to_owned();
        if wk_actions {
            let secondary=api(&app,"/api/agent-sessions",json!({"preset_id":preset,"title":"WK 隔离对照会话","resource_selections":resources,"model":{"provider_id":provider,"model":"browser-gui-fixture"}})).await?;
            api(&app,&format!("/api/agent-sessions/{session_id}/turns"),json!({"input":{"content":"WK_UPGRADE_PRESERVATION_SEED"},"idempotency_key":uuid::Uuid::now_v7().to_string()})).await?;
            let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
            let seed_messages=loop {
                let messages=fixture_read(&app,&format!("/api/agent-sessions/{session_id}/messages?after_seq=0&limit=100")).await?;
                let events=fixture_read(&app,&format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=1000")).await?;
                if messages.to_string().contains("WK_UPGRADE_PRESERVED_REPLY") && events.to_string().contains("turn/completed") {break messages;}
                anyhow::ensure!(tokio::time::Instant::now()<deadline,"Canonical upgrade seed turn did not complete");
                tokio::time::sleep(Duration::from_millis(100)).await;
            };
            let preserved_session=fixture_read(&app,&format!("/api/agent-sessions/{session_id}")).await?;
            let owner=preserved_session["session"]["owner_ref"]["principal_id"].as_str().ok_or_else(||anyhow::anyhow!("Fixture canonical owner missing"))?;
            let key=nomifun_browser_platform::workspace::managed_workspace_key(owner,&session_id)?;
            let nomifun_browser_platform::runtime::BrowserProfile::Persistent(legacy_profile)=
                nomifun_browser_platform::runtime::BrowserProfile::for_agent_session(&root,&key,false) else {anyhow::bail!("Directory profile fixture identity missing")};
            let legacy_sentinel=legacy_profile.join("Default/retained-user-profile.txt");
            std::fs::create_dir_all(legacy_sentinel.parent().unwrap())?;
            std::fs::write(&legacy_sentinel,b"Legacy CEF user profile remains user-owned. Do not migrate or delete.\n")?;
            let providers=fixture_read(&app,"/api/providers").await?;
            let preserved_provider=providers.as_array().and_then(|items|items.iter().find(|item|item["provider_id"]==provider))
                .cloned().ok_or_else(||anyhow::anyhow!("Fixture provider missing from authoritative list"))?;
            std::fs::write(root.join("wk-upgrade-preservation.json"),serde_json::to_vec_pretty(&json!({
                "provider_id":provider,"provider":preserved_provider,"session":preserved_session,"messages":seed_messages,
                "legacy_sentinel":legacy_sentinel.strip_prefix(&root)?,
                "legacy_sentinel_text":"Legacy CEF user profile remains user-owned. Do not migrate or delete.\n"
            }))?)?;
            std::fs::write(root.join("wk-acceptance.json"),serde_json::to_vec_pretty(&json!({
                "mode":"wk-actions","data_dir":root,"control":format!("http://{address}"),"page":format!("http://{address}/"),
                "session_id":session_id,"secondary_session_id":secondary["agent_session_id"],
                "prompt":"Verify the packaged WK Browser surface."
            }))?)?;
        }
        if let Some(live)=&fixture.live {
            let source=install_live_source(&live.work,&session_id)?;
            *live.source.lock().unwrap()=source;
        }
        Ok::<_,anyhow::Error>(Value::String(session_id))
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
        json!({"data_dir":root,"work_dir":fixture.live.as_ref().map(|live|&live.work),"real_provider":live_mode,"native_actions":native_actions,"page":format!("http://{address}/"),"control":format!("http://{address}"),"session_id":session})
    );
    // Keep only the model/page server alive; the real desktop now owns the DB.
    fixture.stop.cancelled().await;
    Ok(())
}

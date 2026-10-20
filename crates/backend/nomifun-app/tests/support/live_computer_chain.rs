//! Real Step Plan model through the product HTTP/Runtime/Kernel/Computer path.
//! Build this target first, then run only this ignored test binary with one
//! credential line on stdin. Never place a credential in argv or environment.
//! Existing opt-in paths: NOMIFUN_COMPUTER_WORKER, NOMIFUN_COMPUTER_FIXTURE_EXE,
//! NOMIFUN_COMPUTER_ARTIFACT_DIR (fresh). The owned fixture source/compilation
//! recipe is documented beside native_computer_chain.rs.
//!
//! The Full case uses two phases; its AX-only instruction is not a NoImage
//! route. The NoImage case keeps advisory vision metadata and uses an explicit
//! model-interface input constraint, verified against the frozen actual route.
use super::*;
use futures_util::FutureExt;
use nomifun_agent_runtime::AgentEngineEvent;
use nomifun_agent_session::{AgentEffectState, AgentSessionStore, TurnReceiptStatus};
use nomifun_chat_model_broker::ChatToolResultPart;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Instant,
};
use zeroize::Zeroizing;

const MODEL: &str = "step-5-preview";
const BASE_URL: &str = "https://api.stepfun.com/step_plan/v1";
const TEXT: &str = "NomiFun Computer Use owned test\r\nCalculator 2 + 3 = 5; 中文文本。";
type LiveResult<T> = Result<T, String>;
fn secret_line() -> LiveResult<Zeroizing<String>> {
    let mut secret = Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .take(16_385)
        .read_line(&mut secret)
        .map_err(|_| "credential stdin read failed")?;
    while secret.ends_with(['\r', '\n']) {
        secret.pop();
    }
    if secret.is_empty()
        || secret.len() > 16_384
        || secret.trim() != secret.as_str()
        || secret.chars().any(char::is_control)
    {
        return Err("invalid credential stdin line".into());
    }
    Ok(secret)
}
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn file_sha(path: &Path) -> LiveResult<String> {
    std::fs::read(path)
        .map(|bytes| sha(&bytes))
        .map_err(|_| "identity file read failed".into())
}
fn save(root: &Path, name: &str, value: &Value) -> LiveResult<()> {
    std::fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).map_err(|_| "report encoding failed")?,
    )
    .map_err(|_| "report write failed".into())
}
fn read_json(path: &Path) -> LiveResult<Value> {
    serde_json::from_slice(&std::fs::read(path).map_err(|_| "owned receipt missing")?)
        .map_err(|_| "owned receipt invalid".into())
}
async fn wait_file(path: &Path) -> LiveResult<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "owned fixture receipt deadline exceeded".into())
}
fn clean_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}
struct OwnedWindow {
    root: PathBuf,
    child: Option<Child>,
}
impl Drop for OwnedWindow {
    fn drop(&mut self) {
        let _ = std::fs::write(self.root.join("close.request"), b"close only this fixture");
    }
}
impl OwnedWindow {
    async fn close(&mut self) -> LiveResult<bool> {
        std::fs::write(self.root.join("close.request"), b"close only this fixture")
            .map_err(|_| "owned close request failed")?;
        let Some(child) = self.child.as_mut() else {
            return Ok(true);
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if child
                .try_wait()
                .map_err(|_| "owned child wait failed")?
                .is_some()
            {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
// A local transport helper deliberately excludes response/provider bodies from
// error messages, assertions and reports, including credential registration.
async fn api(router: &Router, method: &str, path: &str, body: Value) -> LiveResult<Value> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .map_err(|_| "local request construction failed")?,
        )
        .await
        .map_err(|_| "local request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "local {method} request returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .map_err(|_| "local response read failed")?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "local response invalid")?;
    value
        .get("data")
        .cloned()
        .ok_or_else(|| "local response envelope missing".into())
}
async fn restricted_preset(router: &Router, provider: &str) -> LiveResult<(String, Value)> {
    let model = json!({"provider_id":provider,"model":MODEL});
    let editor = api(
        router,
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({"reuse_existing":false,"display_name":"Owned Computer real model","model":model}),
    )
    .await?;
    let id = editor
        .pointer("/preset/preset_id")
        .and_then(Value::as_str)
        .ok_or("preset identity missing")?
        .to_owned();
    let mut draft = editor.get("draft").cloned().ok_or("preset draft missing")?;
    let actions = ["get_window_state", "click", "type_text"]
        .into_iter()
        .map(|name| {
            nomifun_computer::surface::tool(name)
                .expect("frozen tool")
                .action_id
                .clone()
        })
        .collect::<Vec<_>>();
    let document = draft
        .pointer_mut("/document")
        .and_then(Value::as_object_mut)
        .ok_or("preset document missing")?;
    document.insert(
        "enabled_capabilities".into(),
        json!([{"capability":{"id":"computer"},"action_allowlist":actions}]),
    );
    document.insert("skill_bindings".into(), json!([]));
    document.insert("persona".into(),json!("Operate only the exact task-owned native test window. Never interact with another application."));
    document.insert("instructions".into(),json!("Use only the authorized Computer tools and ToolSearch if a required schema is deferred. Use fresh accessibility element tokens and background delivery. Never use desktop capture, application/window enumeration, clipboard, browser, files, shell, delegation, or another window. Stop on unknown effects; do not automatically repeat a potentially delivered input."));
    let saved=api(router,"POST",&format!("/api/agent-presets/{id}/revisions"),json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft,"reason":"explicit controlled real-model Computer acceptance"})).await?;
    let selected = saved
        .pointer("/revision/document/enabled_capabilities")
        .and_then(Value::as_array)
        .ok_or("saved capability selection missing")?;
    if selected.len() != 1 || selected[0]["capability"]["id"] != "computer" {
        return Err("saved preset authority differs".into());
    }
    let route = saved
        .pointer("/revision/document/chat_route_records/agent_chat")
        .ok_or("saved Chat route missing")?;
    if route["failovers"]
        .as_array()
        .is_none_or(|items| !items.is_empty())
    {
        return Err("controlled route unexpectedly has failovers".into());
    }
    let primary = &route["primary"];
    Ok((
        id,
        json!({"route_id":primary["model_route_id"],"route_revision":primary["model_route_revision"],"input_constraints":primary["input_constraints"],"features":primary["features"]}),
    ))
}
async fn turn(
    router: &Router,
    store: &AgentSessionStore,
    sid: &nomifun_agent_contracts::AgentSessionId,
    label: &str,
    prompt: String,
) -> LiveResult<(nomifun_agent_contracts::OperationId, Value)> {
    let started = Instant::now();
    let accepted = api(
        router,
        "POST",
        &format!("/api/agent-sessions/{}/turns", sid.as_ref()),
        json!({"idempotency_key":label,"input":{"content":prompt}}),
    )
    .await?;
    let operation = nomifun_agent_contracts::OperationId::from(
        accepted["operation_id"]
            .as_str()
            .ok_or("turn identity missing")?
            .to_owned(),
    );
    let receipt = tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let receipt = store
                .read_turn_receipt(sid, &operation)
                .await
                .map_err(|_| "canonical Turn receipt unavailable")?;
            if matches!(
                receipt.status,
                TurnReceiptStatus::Completed
                    | TurnReceiptStatus::Failed
                    | TurnReceiptStatus::Cancelled
            ) {
                break Ok::<_, String>(receipt.status);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "real-model Turn observation deadline exceeded")??;
    Ok((
        operation,
        json!({"terminal":receipt,"observer_elapsed_ms":started.elapsed().as_millis(),"timing_source":"test_observer_wall_clock"}),
    ))
}
fn structured(text: &str) -> Option<Value> {
    serde_json::from_str::<Value>(text)
        .ok()
        .filter(|value| value.get("elements").is_some())
        .or_else(|| {
            text.lines().find_map(|line| {
                serde_json::from_str::<Value>(line)
                    .ok()
                    .filter(|value| value.get("elements").is_some())
            })
        })
}
async fn evidence(
    store: &AgentSessionStore,
    sid: &nomifun_agent_contracts::AgentSessionId,
    operation: &nomifun_agent_contracts::OperationId,
    pid: u64,
    window: u64,
    images_required: bool,
    visual_code: &str,
) -> LiveResult<Value> {
    let facts = store
        .turn_output_facts(sid, operation)
        .await
        .map_err(|_| "canonical Turn facts unavailable")?;
    let mut proposed = BTreeMap::new();
    let mut started = BTreeMap::new();
    let mut tokens = BTreeMap::<String, String>::new();
    let mut owned_snapshots = std::collections::BTreeSet::<String>::new();
    let mut calls = Vec::new();
    let mut canonical_raw_images = 0;
    let mut captures = Vec::<Value>::new();
    let mut clicked = Vec::new();
    let mut successful_owned_window_clicks = 0;
    let mut observations = 0;
    let mut verified_five = false;
    let mut verified_text = false;
    let mut model_steps = 0;
    let mut typed_terminal = Value::Null;
    let mut public_text = BTreeMap::<u16, String>::new();
    let mut last_capture_step = 0;
    let mut marker_absent_from_ax = true;
    let mut used_routes = BTreeMap::<String, Value>::new();
    for event in &facts.events {
        if event.kind.0 == "context/model-visible-applied" {
            model_steps += 1;
            let identity = facts
                .event_payloads
                .get(event.event_id.as_ref())
                .and_then(|payload| payload.get("route_identity"))
                .ok_or("actual model route identity missing")?;
            let id = identity["route_id"]
                .as_str()
                .ok_or("actual model route ID missing")?;
            let revision = identity["route_revision"]
                .as_u64()
                .ok_or("actual model route revision missing")?;
            used_routes.insert(
                format!("{id}@{revision}"),
                json!({"route_id":id,"route_revision":revision}),
            );
        }
        if event.kind.0 != "runtime/progress-recorded"
            || event.producer_id.as_ref() != "runtime_supervisor"
            || event.correlation_id.as_ref() != operation.as_ref()
        {
            continue;
        }
        let payload = facts
            .event_payloads
            .get(event.event_id.as_ref())
            .ok_or("canonical progress payload missing")?;
        let Ok(progress) = serde_json::from_value::<AgentEngineEvent>(payload["event"].clone())
        else {
            continue;
        };
        match progress {
            AgentEngineEvent::TurnCompleted {
                model_steps,
                finish_reason,
            } => {
                typed_terminal = json!({"model_steps":model_steps,"finish_reason":finish_reason});
            }
            AgentEngineEvent::OutputTextDelta { step, text, .. } => {
                public_text.entry(step).or_default().push_str(&text);
            }
            AgentEngineEvent::CompletionDelivered { step, text } => {
                public_text.insert(step, text);
            }
            AgentEngineEvent::ToolCallCompleted { call, .. } => {
                proposed.insert(call.call_id.as_ref().to_owned(), call.arguments.0);
            }
            AgentEngineEvent::ToolStarted {
                step,
                call_id,
                capability_id,
                action_id,
            } if capability_id.as_ref() == "computer" => {
                started.insert(
                    call_id.as_ref().to_owned(),
                    (action_id.as_ref().to_owned(), step, event.seq),
                );
            }
            AgentEngineEvent::ToolCompleted {
                result,
                runtime_hold,
                ..
            } => {
                let Some((action, step, start_seq)) = started.remove(result.call_id.as_ref())
                else {
                    continue;
                };
                let success = !result.is_error && runtime_hold.is_none();
                let args = proposed
                    .get(result.call_id.as_ref())
                    .ok_or("Computer proposal missing")?;
                if !success && runtime_hold.is_none() {
                    let content = result
                        .output
                        .iter()
                        .filter_map(|part| {
                            if let ChatToolResultPart::Text { text } = part {
                                Some(json!({"type":"text","text":text}))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    if nomifun_computer::result::normalize(
                        &json!({"isError":true,"content":content}),
                        false,
                    )
                    .effect
                        != nomifun_computer::result::ResultEffectOutcome::Failed
                    {
                        return Err(
                            "Computer outcome was not proven to be a zero-effect refusal".into(),
                        );
                    }
                }
                if success {
                    if action == "computer/get_window_state" {
                        if args["pid"] != pid || args["window_id"] != window {
                            return Err("Computer observation targeted another window".into());
                        }
                        if !images_required && args.get("include_screenshot") != Some(&json!(false))
                        {
                            return Err("AX-only phase requested an image".into());
                        }
                    } else if matches!(action.as_str(), "computer/click" | "computer/type_text") {
                        if images_required {
                            return Err(
                                "image verification phase unexpectedly mutated the window".into()
                            );
                        }
                        let token = args["element_token"]
                            .as_str()
                            .ok_or("input was not addressed by AX token")?;
                        if !token
                            .split_once(':')
                            .is_some_and(|(snapshot, _)| owned_snapshots.contains(snapshot))
                        {
                            return Err(
                                "successful input had no owned-window native snapshot".into()
                            );
                        }
                        if args
                            .get("delivery_mode")
                            .is_some_and(|value| value != "background")
                            || args.get("x").is_some()
                            || args.get("y").is_some()
                        {
                            return Err("input was not background AX delivery".into());
                        }
                        if let Some(target) = args.get("target") {
                            if target["kind"] != "window"
                                || target.get("pid").is_some_and(|value| value != &json!(pid))
                                || target
                                    .get("window_id")
                                    .is_some_and(|value| value != &json!(window))
                            {
                                return Err(
                                    "successful input explicitly targeted another surface".into()
                                );
                            }
                        }
                        let native_ax = result
                            .output
                            .iter()
                            .filter_map(|part| {
                                if let ChatToolResultPart::Text { text } = part {
                                    serde_json::from_str::<Value>(text).ok()
                                } else {
                                    None
                                }
                            })
                            .any(|value| {
                                value["route"] == "accessibility"
                                    && value.pointer("/delivery/mode") == Some(&json!("background"))
                            });
                        if !native_ax {
                            return Err(
                                "successful input lacked a native background accessibility receipt"
                                    .into(),
                            );
                        }
                        if action == "computer/click" {
                            successful_owned_window_clicks += 1;
                            if let Some(label) = tokens.get(token) {
                                clicked.push(label.clone());
                            }
                        }
                    } else {
                        return Err("unexpected successful Computer action".into());
                    }
                }
                if success && action == "computer/get_window_state" {
                    for part in &result.output {
                        match part {
                            ChatToolResultPart::Image { .. } => canonical_raw_images += 1,
                            ChatToolResultPart::Text { text } => {
                                if let Some(state) = structured(text) {
                                    if state["pid"] != pid || state["window_id"] != window {
                                        return Err(
                                            "native snapshot differed from the owned window".into(),
                                        );
                                    }
                                    if let Some(invalidated) =
                                        state["invalidated_snapshot_ids"].as_array()
                                    {
                                        for id in invalidated.iter().filter_map(Value::as_str) {
                                            owned_snapshots.remove(id);
                                        }
                                    }
                                    if let Some(snapshot) = state["snapshot_id"].as_str() {
                                        owned_snapshots.insert(snapshot.to_owned());
                                    }
                                    // Canonical history intentionally replaces binary media with
                                    // bounded replay descriptors. Use the native capture's real
                                    // metadata plus independently verified later visual reading;
                                    // never treat those descriptors as retained raw Image parts.
                                    let requested_capture = proposed
                                        .get(result.call_id.as_ref())
                                        .and_then(|args| args.get("include_screenshot"))
                                        .and_then(Value::as_bool)
                                        == Some(true);
                                    if requested_capture
                                        && state["pid"] == pid
                                        && state["window_id"] == window
                                        && state["screenshot_mime_type"].as_str().is_some_and(
                                            |mime| {
                                                matches!(
                                                    mime,
                                                    "image/png" | "image/jpeg" | "image/webp"
                                                )
                                            },
                                        )
                                        && state["screenshot_width"]
                                            .as_u64()
                                            .is_some_and(|size| size > 0)
                                        && state["screenshot_height"]
                                            .as_u64()
                                            .is_some_and(|size| size > 0)
                                        && !captures.iter().any(|capture| {
                                            capture["call_id"] == result.call_id.as_ref()
                                        })
                                    {
                                        last_capture_step = last_capture_step.max(step);
                                        captures.push(json!({"turn_operation_id":operation,"event_seq":event.seq,"call_id":result.call_id,"action":action,"model_step":step,"pid":pid,"window_id":window,"media_type":state["screenshot_mime_type"],"width":state["screenshot_width"],"height":state["screenshot_height"],"capture_id":state["capture_id"]}));
                                    }
                                    let elements = state["elements"]
                                        .as_array()
                                        .ok_or("AX elements missing")?;
                                    if !elements.is_empty() {
                                        observations += 1;
                                    }
                                    for element in elements {
                                        if ["label", "value"].iter().any(|key| {
                                            element[*key].as_str().is_some_and(|text| {
                                                text.to_ascii_uppercase().contains(visual_code)
                                            })
                                        }) {
                                            marker_absent_from_ax = false;
                                        }
                                        if let (Some(token), Some(label)) = (
                                            element["element_token"].as_str(),
                                            element["label"].as_str(),
                                        ) {
                                            tokens.insert(token.into(), label.into());
                                        }
                                        if element["label"] == "Owned calculation"
                                            && element["value"] == "5"
                                        {
                                            verified_five = true;
                                        }
                                        if element["label"] == "Owned editor"
                                            && element["value"].as_str().is_some_and(|text| {
                                                clean_newlines(text) == clean_newlines(TEXT)
                                            })
                                        {
                                            verified_text = true;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                calls.push(json!({"call_id":result.call_id,"action":action,"model_step":step,"start_seq":start_seq,"result_seq":event.seq,"succeeded":success}));
            }
            _ => {}
        }
    }
    let effects = store
        .list_effects(sid)
        .await
        .map_err(|_| "canonical effects unavailable")?
        .into_iter()
        .filter(|effect| {
            effect.turn_id == *operation && effect.capability_module.as_ref() == "computer"
        })
        .collect::<Vec<_>>();
    let succeeded = calls
        .iter()
        .filter(|call| call["succeeded"] == true)
        .count();
    let effect_rows=effects.iter().map(|effect|json!({"effect_id":effect.effect_id,"action":effect.action_id,"state":effect.state,"latency_ms":effect.settled_at.and_then(|end|end.checked_sub(effect.created_at)),"latency_source":"canonical_effect_recorded_at"})).collect::<Vec<_>>();
    let visual_step = public_text.iter().find_map(|(step, text)| {
        (*step > last_capture_step
            && text
                .to_ascii_uppercase()
                .split(|ch: char| !ch.is_ascii_alphanumeric())
                .any(|token| token == visual_code))
        .then_some(*step)
    });
    let mut evidence = json!({"operation_id":operation,"computer_calls":calls.len(),"successful_computer_calls":succeeded,"computer_call_success_rate":if calls.is_empty(){0.0}else{succeeded as f64/calls.len() as f64},"model_steps":model_steps,"typed_turn_completed":typed_terminal,"marker_absent_from_ax":marker_absent_from_ax,"visual_code_match":visual_step.is_some(),"public_visual_reading_step":visual_step,"last_capture_step":last_capture_step,"calls":calls,"effects":effect_rows,"all_effects_returned":effects.iter().all(|effect|effect.state==AgentEffectState::Returned),"ax_observations":observations,"ax_verified_five":verified_five,"ax_verified_text":verified_text,"clicked_controls":clicked,"successful_owned_window_clicks":successful_owned_window_clicks,"canonical_raw_images":canonical_raw_images,"canonical_media_policy":"binary images omitted from bounded replay history","native_window_captures":captures,"read_only_call_latency_ms":null,"read_only_latency_note":"canonical tool events contain sequences, not wall-clock timestamps"});
    evidence["used_route_identities"] = json!(used_routes.into_values().collect::<Vec<_>>());
    Ok(evidence)
}
fn no_plaintext_secret(root: &Path, secret: &[u8]) -> LiveResult<()> {
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| "credential audit metadata unavailable")?;
        if metadata.file_type().is_symlink() {
            return Err("credential audit encountered a symlink".into());
        }
        if metadata.is_dir() {
            for entry in
                std::fs::read_dir(path).map_err(|_| "credential audit directory unavailable")?
            {
                pending.push(
                    entry
                        .map_err(|_| "credential audit entry unavailable")?
                        .path(),
                );
            }
        } else if metadata.is_file() {
            let bytes = std::fs::read(path).map_err(|_| "credential audit file unavailable")?;
            if bytes.windows(secret.len()).any(|value| value == secret) {
                return Err("plaintext credential unexpectedly persisted".into());
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real Step Plan usage and an owned native window; credential only on stdin; run one exact test"]
async fn stepfun_full_route_ax_then_window_image() {
    run_case(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "real Step Plan usage and an owned native window; credential only on stdin; run one exact test"]
async fn stepfun_explicit_no_image_route_completes_ax_tasks() {
    run_case(true).await;
}

async fn run_case(no_image: bool) {
    let route_name = if no_image { "NoImage" } else { "Full" };
    let secret = secret_line().expect("one bounded credential line is required on stdin");
    let root = PathBuf::from(
        std::env::var_os("NOMIFUN_COMPUTER_ARTIFACT_DIR").expect("new isolated artifact directory"),
    );
    let fixture = PathBuf::from(
        std::env::var_os("NOMIFUN_COMPUTER_FIXTURE_EXE").expect("owned fixture executable"),
    );
    let worker =
        PathBuf::from(std::env::var_os("NOMIFUN_COMPUTER_WORKER").expect("test worker executable"));
    assert!(
        root.is_absolute()
            && fixture.is_absolute()
            && fixture.is_file()
            && worker.is_absolute()
            && worker.is_file()
    );
    std::fs::create_dir(&root).expect("artifact directory must be fresh");
    let mut owned = OwnedWindow {
        root: root.clone(),
        child: None,
    };
    let config = AppConfig {
        data_dir: root.join("data"),
        work_dir: root.join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(TRUST.into()),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let database = nomifun_db::init_database(&config.database_path())
        .await
        .unwrap();
    let mut app = AppServices::from_config(database, &config).await.unwrap();
    app.browser_resources = Some(Arc::new(
        nomifun_browser_platform::workspace::BrowserResourceService::new(Arc::new(
            BindingOnlyBrowser(Arc::new(AtomicUsize::new(0))),
        )),
    ));
    let router = create_router(&app).await;
    let mut report = json!({"ok":false,"evidence_kind":"real_model_product_computer","provider":"stepfun-plan","model":MODEL,"route":route_name,"preset_template":"chat.minimal","authority":"Computer only: frozen get_window_state/click/type_text","model_was_scripted":false,"task_total":2,"task_success_count":0});
    let outcome=std::panic::AssertUnwindSafe(async{
        let mut capability=json!({"task":"chat","traits":["vision_input"],"protocol":"openai.chat_text","connection_role":"default","provider_params":{"temperature":0.0},"context_limit":1048576,"output_limit":8192});
        if no_image { capability["input_constraints"]=json!({"image_input":false}); }
        let provider=api(&router,"POST","/api/providers",json!({"platform":"stepfun-plan","name":"Controlled Computer real model","base_url":BASE_URL,"auth_scheme":"bearer","credentials":{"api_keys":[secret.as_str()]},"enabled":true,"initial_model":{"model":MODEL,"enabled":true,"capabilities":[capability]}})).await?;
        let provider_id=provider["provider_id"].as_str().ok_or("provider identity missing")?;
        let (preset,frozen_route)=restricted_preset(&router,provider_id).await?;
        let features=frozen_route["features"].as_array().ok_or("frozen model features missing")?;
        if !features.iter().any(|feature|feature=="tool_calls")||features.iter().any(|feature|feature=="image_input")==no_image {
            return Err("frozen model features did not enforce the declared input contract".into());
        }
        if no_image&&frozen_route.pointer("/input_constraints/image_input")!=Some(&json!(false)) {
            return Err("explicit image denial did not reach the frozen candidate".into());
        }
        let expected_identity=json!({"route_id":frozen_route["route_id"],"route_revision":frozen_route["route_revision"]});
        report["frozen_model_route"]=frozen_route;
        owned.child=Some(Command::new(&fixture).arg("functional").arg(&root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|_|"owned fixture spawn failed")?);
        wait_file(&root.join("ready.json")).await?;
        let ready=read_json(&root.join("ready.json"))?;
        let pid=ready["pid"].as_u64().ok_or("owned PID missing")?;let window=ready["window"].as_str().ok_or("owned HWND missing")?.parse::<u64>().map_err(|_|"owned HWND invalid")?;
        if owned.child.as_ref().map(Child::id)!=Some(pid as u32){return Err("owned fixture process identity differs".into())}
        report["app_pid"]=json!(pid);report["window_id"]=json!(window);report["dpi"]=ready["dpi"].clone();
        let marker=read_json(&root.join("visual-marker.json"))?;let visual_code=marker["code"].as_str().ok_or("visual marker missing")?;
        if marker["pid"]!=pid||visual_code.len()!=8||!visual_code.bytes().all(|byte|byte.is_ascii_hexdigit()){return Err("owned visual marker identity invalid".into())}
        report["visual_marker_sha256"]=json!(sha(visual_code.as_bytes()));
        let session=api(&router,"POST","/api/agent-sessions",json!({"preset_id":preset,"model":{"provider_id":provider_id,"model":MODEL},"resource_selections":[{"resource_kind":"computer","resource_id":"local-desktop"}]})).await?;
        let sid=nomifun_agent_contracts::AgentSessionId::from(session["agent_session_id"].as_str().ok_or("Session identity missing")?.to_owned());
        report["session_id"]=json!(sid);
        let store=AgentSessionStore::from_pool(app.database.pool().clone()).await.map_err(|_|"canonical store unavailable")?;
        let common=format!("This is an authorized controlled Computer Use acceptance. The only allowed application is an already running dedicated WinForms test window, exact pid={pid}, window_id={window}. Never launch/find/list other applications, never use desktop capture, browser, clipboard, filesystem or shell. Every get_window_state must use precisely that pid and window_id. Use only fresh AX element_token inputs with delivery_mode background. ToolSearch may load an authorized schema if needed. Do not claim success from your own text; take fresh AX observations to verify final values.");
        let prompt=format!("{common}\nThis interface uses the {route_name} route. Complete the following task using AX only. Always set include_screenshot:false, tree_format:elements. Click the actual calculator buttons Owned two, Owned add, Owned three, Owned equals in order so the Owned calculation display is 5. Then use type_text on the Owned editor token to enter exactly the two raw text lines between the markers below. There is one actual line break between these lines. Do not type the markers, a code fence, quotation marks, or literal backslash escape characters. Click Save owned test text afterwards.\nBEGIN_OWNED_TEXT\n{TEXT}\nEND_OWNED_TEXT\nDo not use pixel coordinates. Read a fresh AX snapshot after saving and verify calculation 5 and the complete editor text. Finish only after these actual actions and observations.");
        if prompt.to_ascii_uppercase().contains(visual_code){return Err("visual marker accidentally appeared in AX prompt".into())}
        let (first,timing)=turn(&router,&store,&sid,"real-computer-ax-phase",prompt).await?;
        let ax=evidence(&store,&sid,&first,pid,window,false,visual_code).await?;
        report["ax_phase"]=json!({"route":route_name,"instruction":if no_image{"explicit interface image denial; genuine NoImage candidate"}else{"AX-only instruction on a Full route"},"timing":timing,"canonical":ax});
        if report.pointer("/ax_phase/canonical/used_route_identities")!=Some(&json!([expected_identity.clone()])) {
            return Err("actual model route differs from the frozen controlled candidate".into());
        }
        let calculation=read_json(&root.join("calculation.json")).ok();
        let calc_ok=calculation.as_ref().is_some_and(|value|value["expression"]=="2+3"&&value["result"]=="5")&&report.pointer("/ax_phase/canonical/ax_verified_five")==Some(&json!(true));
        let text_ok=std::fs::read_to_string(root.join("saved.txt")).is_ok_and(|text|clean_newlines(&text)==clean_newlines(TEXT))&&report.pointer("/ax_phase/canonical/ax_verified_text")==Some(&json!(true));
        report["task_success_count"]=json!(usize::from(calc_ok)+usize::from(text_ok));report["task_success_rate"]=json!((usize::from(calc_ok)+usize::from(text_ok)) as f64/2.0);
        if report.pointer("/ax_phase/timing/terminal")!=Some(&json!("completed")){return Err("AX real-model Turn did not complete".into())}
        if report.pointer("/ax_phase/canonical/typed_turn_completed/finish_reason")!=Some(&json!("completed"))||report.pointer("/ax_phase/canonical/marker_absent_from_ax")!=Some(&json!(true)){return Err("AX Turn lacked normal typed completion or leaked the visual marker".into())}
        if !calc_ok||!text_ok||report.pointer("/ax_phase/canonical/canonical_raw_images")!=Some(&json!(0))||report.pointer("/ax_phase/canonical/native_window_captures").and_then(Value::as_array).is_none_or(|captures|!captures.is_empty())||report.pointer("/ax_phase/canonical/all_effects_returned")!=Some(&json!(true)){return Err("AX task postconditions were not proven".into())}
        let clicked=report.pointer("/ax_phase/canonical/clicked_controls").and_then(Value::as_array).ok_or("clicked-control evidence missing")?;
        let expected=["Owned two","Owned add","Owned three","Owned equals"];let mut next=0;for label in clicked{if next<expected.len()&&label==expected[next]{next+=1;}}
        if next!=expected.len()||report.pointer("/ax_phase/canonical/successful_owned_window_clicks").and_then(Value::as_u64).unwrap_or(0)<5{return Err("actual calculator sequence and native Save click were not proven".into())}
        if no_image {
            report["image_phase"]=json!({"skipped":true,"reason":"the model interface explicitly disallows image input"});
            report["marker_absent_from_prompts"]=json!(true);
        } else {
        let image_prompt=format!("{common}\nPhase 2: the task values must remain unchanged. Use get_window_state for only pid={pid}, window_id={window}, with include_screenshot:true and tree_format:elements. Inspect the returned image: read the random eight-character code painted in the lower-right corner, which is deliberately absent from the AX tree. Your final public answer must contain that exact code. Also verify calculation 5 and saved editor text. Do not click, type or capture the desktop. Read the code from the actual window image; do not guess it or use files.");
        if image_prompt.to_ascii_uppercase().contains(visual_code){return Err("visual marker accidentally appeared in image prompt".into())}
        report["marker_absent_from_prompts"]=json!(true);
        let (second,timing)=turn(&router,&store,&sid,"real-computer-image-phase",image_prompt).await?;
        let image=evidence(&store,&sid,&second,pid,window,true,visual_code).await?;
        report["image_phase"]=json!({"route":"Full","timing":timing,"canonical":image});
        if report.pointer("/image_phase/canonical/used_route_identities")!=Some(&json!([expected_identity])) {
            return Err("image Turn used a different model candidate".into());
        }
        if report.pointer("/image_phase/timing/terminal")!=Some(&json!("completed")){return Err("image real-model Turn did not complete".into())}
        if report.pointer("/image_phase/canonical/typed_turn_completed/finish_reason")!=Some(&json!("completed"))||report.pointer("/image_phase/canonical/visual_code_match")!=Some(&json!(true))||report.pointer("/image_phase/canonical/marker_absent_from_ax")!=Some(&json!(true)){return Err("image Turn lacked normal completion or independently verified visual reading".into())}
        if report.pointer("/image_phase/canonical/native_window_captures").and_then(Value::as_array).is_none_or(Vec::is_empty)||report.pointer("/image_phase/canonical/computer_calls").and_then(Value::as_u64).unwrap_or(0)==0{return Err("Full route produced no successful native window capture".into())}
        report["image_evidence"]=json!("real_provider_visual_challenge_after_native_window_capture");
        }
        let status=api(&router,"GET","/api/computer/status",Value::Null).await?;
        report["worker_status_before_shutdown"]=json!({"running":status["running"],"process_id":status["process_id"],"worker_version":status["worker_version"],"phase":status["phase"]});
        report["worker_sha256"]=json!(file_sha(&worker)?);report["fixture_sha256"]=json!(file_sha(&fixture)?);report["source_sha256"]=json!(file_sha(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/cua-driver/SOURCE.json"))?);
        Ok::<(),String>(())
    }).catch_unwind().await;
    let shutdown = app.shutdown_nomi_core_host().await;
    let fixture_closed = owned.close().await.unwrap_or(false);
    let status = api(&router, "GET", "/api/computer/status", Value::Null).await;
    let exit = read_json(&root.join("exit.json")).ok();
    let graceful = fixture_closed
        && owned.child.as_ref().is_none_or(|child| {
            exit.as_ref().is_some_and(|exit| {
                exit["pid"] == child.id()
                    && exit["graceful"] == true
                    && exit["control_file"] == true
            })
        });
    let audited = no_plaintext_secret(&root, secret.as_bytes());
    report["cleanup"] = json!({"normal_shutdown_succeeded":shutdown.is_ok(),"worker_running":status.as_ref().ok().map(|value|value["running"].clone()),"fixture_exact_child_exited":fixture_closed,"fixture_graceful_receipt":graceful,"plaintext_credential_absent":audited.is_ok()});
    let passed = matches!(&outcome, Ok(Ok(())))
        && shutdown.is_ok()
        && status.as_ref().is_ok_and(|value| value["running"] == false)
        && graceful
        && audited.is_ok();
    report["ok"] = json!(passed);
    if let Ok(Err(code)) = &outcome {
        report["failure"] = json!(code);
    } else if outcome.is_err() {
        report["failure"] = json!("live test assertion failed; output suppressed");
    }
    save(&root, "live-computer-report.json", &report).expect("sanitized report must be saved");
    assert!(
        passed,
        "real-model Computer acceptance failed; inspect sanitized live-computer-report.json"
    );
}

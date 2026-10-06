//! Real Step Plan + canonical Session + background IDMM acceptance demo.
//! The loopback gateway injects only explicit 503/silence faults; all successful
//! generations, including the decision sidecar, use the fixed official upstream.
//! No Session facts are seeded, no tools are granted, and no model reply is mocked.

use super::*;
use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use axum::{extract::State, response::Response, routing::post};

const CASE_DEADLINE: Duration = Duration::from_secs(180);
const OPTIONS: &str = "请选择演示输出格式：\n1. Markdown\n2. HTML（推荐）\n3. 取消";
const RULE_DONE: &str = "IDMM_RULE_DONE_HTML";
const TITLE: &str = "缓存观察实验";
const OPEN_QUESTION: &str = "请提供这个演示的最终标题。";
const SIDECAR_DONE: &str = TITLE;
const SENSITIVE: &str = "请提供 API key 才能继续。";
const RECOVERY_DONE: &str = "IDMM_RECOVERY_DONE";

#[derive(Default)]
struct GatewayCounters {
    failing: AtomicBool,
    stall_next: AtomicBool,
    injected_errors: AtomicUsize,
    injected_stalls: AtomicUsize,
    upstream_calls: AtomicUsize,
    sidecar_calls: AtomicUsize,
}

#[derive(Clone)]
struct GatewayState {
    client: reqwest::Client,
    counters: Arc<GatewayCounters>,
}

struct Gateway {
    base_url: String,
    counters: Arc<GatewayCounters>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Gateway {
    fn drop(&mut self) { self.task.abort(); }
}

fn gateway_error(status: StatusCode, code: &'static str) -> Response {
    Response::builder().status(status).header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"error":{"message":code,"type":"server_error","code":code}}).to_string()))
        .expect("fixed response")
}

async fn forward(State(state): State<GatewayState>, request: Request<Body>) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, BODY_LIMIT).await else {
        return gateway_error(StatusCode::PAYLOAD_TOO_LARGE, "IDMM_DEMO_BODY_LIMIT");
    };
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
        return gateway_error(StatusCode::BAD_REQUEST, "IDMM_DEMO_JSON_REQUIRED");
    };
    if payload.get("model").and_then(Value::as_str) != Some(STEPFUN_PLAN_MODEL) {
        return gateway_error(StatusCode::BAD_REQUEST, "IDMM_DEMO_MODEL_MISMATCH");
    }
    let Some(authorization) = parts.headers.get(header::AUTHORIZATION).cloned() else {
        return gateway_error(StatusCode::UNAUTHORIZED, "IDMM_DEMO_AUTH_REQUIRED");
    };
    if state.counters.failing.load(Ordering::SeqCst) {
        state.counters.injected_errors.fetch_add(1, Ordering::SeqCst);
        return gateway_error(StatusCode::SERVICE_UNAVAILABLE, "IDMM_DEMO_PROVIDER_UNAVAILABLE");
    }
    if state.counters.stall_next.swap(false, Ordering::SeqCst) {
        state.counters.injected_stalls.fetch_add(1, Ordering::SeqCst);
        // No upstream generation or external effect was started. The native
        // cancellation closes this request while a successor can proceed.
        tokio::time::sleep(Duration::from_secs(120)).await;
        return gateway_error(StatusCode::GATEWAY_TIMEOUT, "IDMM_DEMO_INJECTED_TIMEOUT");
    }
    let sidecar = payload.get("messages").and_then(Value::as_array).is_some_and(|messages|
        messages.iter().any(|message| message.get("role").and_then(Value::as_str) == Some("system")
            && message.get("content").and_then(Value::as_str).is_some_and(|text|
                text.contains("constrained decision sidecar"))));
    state.counters.upstream_calls.fetch_add(1, Ordering::SeqCst);
    if sidecar { state.counters.sidecar_calls.fetch_add(1, Ordering::SeqCst); }
    // Redirects are disabled, and the destination cannot be supplied by a model
    // or CLI argument. Only the official Step Plan receives the bearer token.
    let response = state.client.post(format!("{STEPFUN_PLAN_BASE_URL}/chat/completions"))
        .header(header::AUTHORIZATION, authorization)
        .header(header::CONTENT_TYPE, "application/json")
        .body(bytes).send().await;
    let Ok(response) = response else {
        return gateway_error(StatusCode::BAD_GATEWAY, "IDMM_DEMO_UPSTREAM_NETWORK_ERROR");
    };
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    let mut builder = Response::builder().status(status);
    if let Some(value) = content_type { builder = builder.header(header::CONTENT_TYPE, value); }
    builder.body(Body::from_stream(response.bytes_stream())).expect("fixed streaming response")
}

async fn gateway() -> Result<Gateway, SmokeFailure> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await
        .map_err(|_| failure("IDMM_GATEWAY_BIND_FAILED"))?;
    let address = listener.local_addr().map_err(|_| failure("IDMM_GATEWAY_ADDRESS_FAILED"))?;
    let counters = Arc::new(GatewayCounters::default());
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(30)).timeout(CASE_DEADLINE).build()
        .map_err(|_| failure("IDMM_GATEWAY_CLIENT_FAILED"))?;
    let router = Router::new().route("/chat/completions", post(forward))
        .with_state(GatewayState { client, counters: counters.clone() });
    let task = tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    Ok(Gateway { base_url: format!("http://{address}"), counters, task })
}

fn failure(code: &'static str) -> SmokeFailure { SmokeFailure::new("idmm.demo", code, 422) }

async fn state(router: &Router, session: &str) -> Result<Value, SmokeFailure> {
    let response = successful_json(router, "idmm.state", Method::GET,
        format!("/api/agent-sessions/{session}/idmm"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    envelope_data("idmm.state", response)
}

async fn config(router: &Router, session: &str, provider: &str, mode: &str, interval: u32) -> Result<(), SmokeFailure> {
    let response = successful_json(router, "idmm.config", Method::PUT,
        format!("/api/agent-sessions/{session}/idmm"), Some(json!({
            "mode":mode,"scan_interval_secs":interval,"idle_timeout_secs":30,
            "min_interval_secs":0,"max_retries":3,"max_interventions_per_hour":20,
            "bypass_model": if mode == "rule_plus_model" {
                json!({"provider_id":provider,"model":STEPFUN_PLAN_MODEL})
            } else { json!({}) }
        })), LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    envelope_data("idmm.config", response)?;
    Ok(())
}

async fn evaluate(router: &Router, session: &str) -> Result<Value, SmokeFailure> {
    let response = successful_json(router, "idmm.evaluate", Method::POST,
        format!("/api/agent-sessions/{session}/idmm/evaluate"), Some(json!({})),
        CASE_DEADLINE, &[StatusCode::OK]).await?;
    envelope_data("idmm.evaluate", response)
}

async fn checked(router: &Router, session: &str) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if state(router, session).await?.get("last_checked_at").is_some_and(|v| !v.is_null()) { return Ok(()); }
        if tokio::time::Instant::now() >= deadline { return Err(failure("IDMM_BACKGROUND_CHECK_MISSING")); }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_text(router: &Router, session: &str, text: &str) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + CASE_DEADLINE;
    loop {
        let (messages, _) = session_messages_after(router, "idmm.messages", session, 0).await?;
        let events = session_events(router, session).await?;
        let has_text = messages.iter().filter_map(assistant_text_projection)
            .any(|projection| projection.get("content").and_then(Value::as_str) == Some(text));
        let head = envelope_data("idmm.head", successful_json(router, "idmm.head", Method::GET,
            format!("/api/agent-sessions/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
        if has_text && head.pointer("/head/status").and_then(Value::as_str) == Some("ready") { return Ok(()); }
        // Provider/stall cases intentionally include an earlier failed/cancelled
        // Turn. Their successor must still produce an exact completed reply.
        if tokio::time::Instant::now() >= deadline {
            let failed = events.iter().any(|event| event.get("kind").and_then(Value::as_str) == Some("turn/failed"));
            return Err(failure(if failed { "IDMM_RESULT_FAILED_OR_RECOVERY_MISSING" } else { "IDMM_EXACT_REPLY_MISSING" }));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_intervention(router: &Router, session: &str, kind: &str, status: &str) -> Result<Value, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + CASE_DEADLINE;
    loop {
        let current = state(router, session).await?;
        if current.get("recent_interventions").and_then(Value::as_array).is_some_and(|items|
            items.iter().any(|item| item.get("kind").and_then(Value::as_str) == Some(kind)
                && item.get("status").and_then(Value::as_str) == Some(status))) { return Ok(current); }
        if current.get("recent_interventions").and_then(Value::as_array).is_some_and(|items|
            items.iter().filter(|item| item.get("status").and_then(Value::as_str) == Some("failed")).count() >= 3) {
            return Err(failure("IDMM_INTERVENTION_FAILED"));
        }
        if tokio::time::Instant::now() >= deadline { return Err(failure("IDMM_INTERVENTION_MISSING")); }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_terminal(router: &Router, session: &str, operation: &str, kind: &str) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + CASE_DEADLINE;
    loop {
        if session_events(router, session).await?.iter().any(|event|
            event.get("kind").and_then(Value::as_str) == Some(kind)
                && event.get("correlation_id").and_then(Value::as_str) == Some(operation)) { return Ok(()); }
        if tokio::time::Instant::now() >= deadline { return Err(failure("IDMM_EXPECTED_TERMINAL_MISSING")); }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn question_prompt(question: &str, completion: &str) -> String {
    if question == OPEN_QUESTION {
        // The observable deliverable is the confirmed title itself. A separate
        // synthetic final marker in this context would also be a plausible
        // minimal sidecar reply and would confound title quality acceptance.
        return format!("演示项目的最终标题已确定为「{TITLE}」。这是一个无工具的标题确认对话。请先向我确认标题：当前回复只能原样输出「{OPEN_QUESTION}」，然后等待下一条输入，禁止自己回答或调用工具。收到下一条输入后，只原样复述收到的标题，禁止加说明、引号或其他标记。");
    }
    format!("这是一个无工具的智能决策对话验收。第一轮仅原样输出以下提问并结束当前回复，必须等待下一条输入，禁止自己回答、补充说明或调用工具：\n{question}\n\n收到下一条真实输入后，仅输出 {completion}。如果是格式选择，只有收到 2 才输出该完成标记。演示最终标题的已确认偏好是「{TITLE}」。")
}

async fn new_session(router: &Router, preset: &str, provider: &str, model: &str) -> Result<String, SmokeFailure> {
    Ok(create_session(router, preset, provider, model, json!([])).await?.0)
}

async fn report(router: &Router, session: &str, name: &str, started: tokio::time::Instant,
    counters: &GatewayCounters, sidecar_before: usize, api_key: &str) -> Result<(), SmokeFailure> {
    let current = state(router, session).await?;
    let events = session_events(router, session).await?;
    let (messages, _) = session_messages_after(router, "idmm.evidence", session, 0).await?;
    let turns: Vec<_> = events.iter().filter(|event| event.get("kind").and_then(Value::as_str) == Some("turn/started"))
        .map(|event| {
            // Turn start references the accepted input; origin belongs to that
            // input event, not a copied field on the Turn start payload.
            let input_id = event.pointer("/payload/value/source_message_id");
            let origin = events.iter().find(|candidate| Some(&candidate["event_id"]) == input_id)
                .and_then(|input| input.pointer("/payload/value/origin"));
            let terminal = events.iter().find(|candidate| candidate["correlation_id"] == event["correlation_id"]
                && matches!(candidate["kind"].as_str(), Some("turn/completed" | "turn/failed" | "turn/cancelled")));
            json!({"operation_id":event["correlation_id"],"source_message_id":input_id,"origin":origin,
                "terminal_kind":terminal.map(|terminal| &terminal["kind"]),
                "terminal_event_id":terminal.map(|terminal| &terminal["event_id"])})
        }).collect();
    let idmm_turns = turns.iter().filter(|turn| turn.get("origin").and_then(Value::as_str) == Some("idmm")).count();
    let completed_turns = events.iter().filter(|event| event.get("kind").and_then(Value::as_str) == Some("turn/completed")).count();
    let expected = if matches!(name, "off" | "sensitive_halt" | "provider_pause") { 0 } else { 1 };
    let idmm_completed = turns.iter().filter(|turn| turn["origin"] == "idmm").all(|turn|
        events.iter().any(|event| event["kind"] == "turn/completed"
            && event["correlation_id"] == turn["operation_id"]));
    let mut check_error = None;
    if idmm_turns != expected || !idmm_completed {
        check_error = Some("IDMM_CANONICAL_TURN_EVIDENCE_INVALID");
    }
    if messages.iter().any(|message| message.get("presentation_intent").and_then(Value::as_str) == Some("tool")) {
        check_error = Some("IDMM_UNEXPECTED_TOOL_EXECUTION");
    }
    let sidecar_calls = counters.sidecar_calls.load(Ordering::SeqCst) - sidecar_before;
    if (name == "sidecar_question" && !(1..=3).contains(&sidecar_calls))
        || (name != "sidecar_question" && sidecar_calls != 0) { check_error = Some("IDMM_SIDECAR_CALL_COUNT_INVALID"); }
    let inputs: Vec<_> = messages.iter().filter(|message|
        message["presentation_intent"] == "message" && message.pointer("/projection/state").and_then(Value::as_str) == Some("accepted"))
        .filter_map(|message| message.pointer("/projection/content").and_then(Value::as_str)).collect();
    if expected == 1 && inputs.len() != 2 { check_error = Some("IDMM_INPUT_COUNT_INVALID"); }
    if matches!(name, "manual_rule" | "background_rule") && inputs.get(1) != Some(&"2") {
        check_error = Some("IDMM_RECOMMENDED_OPTION_NOT_DELIVERED");
    }
    if name == "sidecar_question" && !inputs.get(1).is_some_and(|text| text.contains(TITLE)) {
        check_error = Some("IDMM_SIDECAR_IGNORED_USER_PREFERENCE");
    }
    let native_resumes = events.iter().filter(|event| event["kind"] == "turn/resume-authorized").count();
    if name == "provider_pause" && (turns.len() != 1 || native_resumes != 1 || completed_turns != 1) {
        check_error = Some("IDMM_NATIVE_RESUME_EVIDENCE_INVALID");
    }
    let transcript: Vec<_> = messages.iter().filter(|message| message.get("presentation_intent").and_then(Value::as_str) == Some("message"))
        .filter_map(|message| {
            let projection = message.get("projection")?;
            let text = projection.get("content")?.as_str()?;
            Some(json!({"state":projection["state"],"content":text.chars().take(1500).collect::<String>()}))
        }).collect();
    let evidence = json!({"case":name,"status":if check_error.is_some(){"fail"}else{"pass"},
        "failure_code":check_error,"session_id":session,
        "elapsed_ms":started.elapsed().as_millis() as u64,"idmm_turns":idmm_turns,
        "completed_turns":completed_turns,"sidecar_calls":sidecar_calls,
        "native_resumes":native_resumes,
        "known_limitation":if name == "provider_pause" { Some("provider_pause_requires_explicit_native_resume") } else { None },
        "upstream_calls":counters.upstream_calls.load(Ordering::SeqCst),
        "injected_errors":counters.injected_errors.load(Ordering::SeqCst),
        "injected_stalls":counters.injected_stalls.load(Ordering::SeqCst),
        "turns":turns,"interventions":current["recent_interventions"],"transcript":transcript});
    if value_contains(&evidence, api_key) { return Err(failure("IDMM_EVIDENCE_CONTAINS_CREDENTIAL")); }
    eprintln!("NOMIFUN_IDMM_DEMO_CASE {evidence}");
    match check_error { Some(code) => Err(failure(code)), None => Ok(()) }
}

pub(super) async fn run(router: &Router, api_key: &str, model: &str) -> Result<(), SmokeFailure> {
    let gateway = gateway().await?;
    let provider = configure_stepfun(router, api_key, &gateway.base_url, model).await?;
    let (preset, _) = create_agent_preset(router, &provider, model, &[]).await?;

    for (name, mode, interval, question, completion) in [
        ("off", "off", 5, OPTIONS, RULE_DONE),
        ("manual_rule", "rule_only", 300, OPTIONS, RULE_DONE),
        ("background_rule", "rule_only", 5, OPTIONS, RULE_DONE),
        ("sidecar_question", "rule_plus_model", 5, OPEN_QUESTION, SIDECAR_DONE),
        ("sensitive_halt", "rule_plus_model", 5, SENSITIVE, RULE_DONE),
    ] {
        eprintln!("NOMIFUN_IDMM_DEMO_ACTIVE case={name}");
        gateway.counters.upstream_calls.store(0, Ordering::SeqCst);
        let started = tokio::time::Instant::now();
        let sidecar_before = gateway.counters.sidecar_calls.load(Ordering::SeqCst);
        let session = new_session(router, &preset, &provider, model).await?;
        config(router, &session, &provider, mode, interval).await?;
        if mode != "off" { checked(router, &session).await?; }
        start_session_turn(router, "idmm.question", &session, &format!("idmm-demo:{name}"), question_prompt(question, completion)).await?;
        if name == "off" {
            wait_text(router, &session, OPTIONS).await?;
            tokio::time::sleep(Duration::from_secs(6)).await;
            let current = evaluate(router, &session).await?;
            if current["recent_interventions"].as_array().is_none_or(|items| !items.is_empty()) {
                return Err(failure("IDMM_OFF_INTERVENED"));
            }
        } else if name == "sensitive_halt" {
            wait_text(router, &session, SENSITIVE).await?;
            wait_intervention(router, &session, "safety_halt", "halted").await?;
        } else {
            if name == "manual_rule" {
                wait_text(router, &session, OPTIONS).await?;
                if !state(router, &session).await?["recent_interventions"].as_array().is_some_and(Vec::is_empty) {
                    return Err(failure("IDMM_MANUAL_CASE_ALREADY_INTERVENED"));
                }
                evaluate(router, &session).await?;
            }
            if let Err(error) = wait_intervention(router, &session,
                if name == "sidecar_question" { "open_question" } else { "option_decision" }, "succeeded").await {
                let _ = report(router, &session, name, started, &gateway.counters, sidecar_before, api_key).await;
                return Err(error);
            }
            if let Err(error) = wait_text(router, &session, completion).await {
                let _ = report(router, &session, name, started, &gateway.counters, sidecar_before, api_key).await;
                return Err(error);
            }
            // Re-evaluation must not produce another canonical recovery Turn.
            evaluate(router, &session).await?;
            evaluate(router, &session).await?;
        }
        report(router, &session, name, started, &gateway.counters, sidecar_before, api_key).await?;
        config(router, &session, &provider, "off", 5).await?;
    }

    for name in ["provider_pause", "stalled_recovery"] {
        eprintln!("NOMIFUN_IDMM_DEMO_ACTIVE case={name}");
        gateway.counters.upstream_calls.store(0, Ordering::SeqCst);
        gateway.counters.injected_errors.store(0, Ordering::SeqCst);
        gateway.counters.injected_stalls.store(0, Ordering::SeqCst);
        let started = tokio::time::Instant::now();
        let sidecar_before = gateway.counters.sidecar_calls.load(Ordering::SeqCst);
        let session = new_session(router, &preset, &provider, model).await?;
        if name == "provider_pause" { gateway.counters.failing.store(true, Ordering::SeqCst); }
        else {
            gateway.counters.stall_next.store(true, Ordering::SeqCst);
            config(router, &session, &provider, "rule_only", 5).await?;
        }
        let operation = start_session_turn(router, "idmm.recovery", &session, &format!("idmm-demo:{name}"),
            format!("这是无工具的恢复验收。请仅输出 {RECOVERY_DONE}，不要调用工具。后续若收到继续或恢复指令，也只输出同一个标记。")).await?;
        if name == "provider_pause" {
            wait_terminal(router, &session, &operation, "turn/paused").await?;
            gateway.counters.failing.store(false, Ordering::SeqCst);
            let paused = envelope_data("idmm.execution", successful_json(router, "idmm.execution", Method::GET,
                format!("/api/agent-sessions/{session}/execution"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?)?;
            if paused["state"] != "paused" || paused["operation_id"] != operation
                || paused.pointer("/pause/reason").and_then(Value::as_str) != Some("EXECUTION_MODEL_PROVIDER_UNAVAILABLE")
                || paused.pointer("/pause/cleanup_proven").and_then(Value::as_bool) != Some(true)
                || paused["checkpoint_retained"] != true {
                return Err(failure("IDMM_PROVIDER_NATIVE_PAUSE_MISSING"));
            }
            // Isolate provider-fault supervision from the separate idle timer.
            // The production IDMM currently observes this retained Turn as
            // running, not failed; the demo must expose that integration gap.
            successful_json(router, "idmm.paused_config", Method::PUT,
                format!("/api/agent-sessions/{session}/idmm"), Some(json!({"mode":"rule_only",
                    "scan_interval_secs":5,"recover_stalled_turns":false,"min_interval_secs":0})),
                LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
            checked(router, &session).await?;
            tokio::time::sleep(Duration::from_secs(6)).await;
            if !state(router, &session).await?["recent_interventions"].as_array().is_some_and(Vec::is_empty) {
                return Err(failure("IDMM_PAUSE_UNEXPECTEDLY_REPLAYED"));
            }
            let resume = json!({"operation_id":operation,"idempotency_key":"idmm-demo-native-resume",
                "expected_pause_revision":paused["pause"]["revision"],
                "expected_checkpoint_revision":paused["checkpoint_revision"],
                "expected_checkpoint_digest":paused["checkpoint_digest"],"budget":{}});
            let mut stale = resume.clone();
            stale["expected_checkpoint_digest"] = json!("f".repeat(64));
            let (status, _) = dispatch_json(router, "idmm.stale_resume", Method::POST,
                format!("/api/agent-sessions/{session}/execution/resume"), Some(stale), LOCAL_API_DEADLINE).await?;
            if status != StatusCode::CONFLICT { return Err(failure("IDMM_STALE_CHECKPOINT_ACCEPTED")); }
            let first = envelope_data("idmm.native_resume", successful_json(router, "idmm.native_resume", Method::POST,
                format!("/api/agent-sessions/{session}/execution/resume"), Some(resume.clone()),
                CASE_DEADLINE, &[StatusCode::OK]).await?)?;
            let replay = envelope_data("idmm.native_resume", successful_json(router, "idmm.native_resume", Method::POST,
                format!("/api/agent-sessions/{session}/execution/resume"), Some(resume),
                CASE_DEADLINE, &[StatusCode::OK]).await?)?;
            if first["seq"] != replay["seq"] { return Err(failure("IDMM_NATIVE_RESUME_DUPLICATED")); }
        } else {
            wait_intervention(router, &session, "stalled_turn", "succeeded").await?;
            wait_terminal(router, &session, &operation, "turn/cancelled").await?;
        }
        wait_text(router, &session, RECOVERY_DONE).await?;
        report(router, &session, name, started, &gateway.counters, sidecar_before, api_key).await?;
        config(router, &session, &provider, "off", 5).await?;
    }
    Ok(())
}

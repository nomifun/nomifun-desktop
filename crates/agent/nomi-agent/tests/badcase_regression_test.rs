//! Full-stack regression cover for a QA round that found the engine shipping
//! confident, wrong work.
//!
//! These drive the real `AgentEngine` loop, the real tool registry, and the real
//! OpenAI-compatible provider over local HTTP. Nothing here is a unit-level
//! stub: the point is that unit tests could all pass while the assembled system
//! still misbehaved, which is exactly what happened — the serializer dropped
//! steering on the wire, and the model shipped a contract-violating
//! implementation while truthfully reporting green tests.
//!
//! Each test documents the observed failure it pins.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nomi_agent::engine::AgentEngine;
use nomi_config::compat::ProviderCompat;
use nomi_config::config::{Config, ProviderType, SessionConfig, ToolsConfig};
use nomi_config::hooks::HooksConfig;
use nomi_mcp::config::McpConfig;
use nomi_providers::create_provider;
use nomi_tools::registry::ToolRegistry;
use nomi_types::message::StopReason;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sse(chunks: &[Value]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str("data: ");
        body.push_str(&chunk.to_string());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn text_then_stop(text: &str) -> String {
    sse(&[
        json!({ "choices": [{ "delta": { "content": text }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] }),
    ])
}

fn tool_call(id: &str, name: &str, args: Value) -> String {
    sse(&[
        json!({ "choices": [{ "delta": { "tool_calls": [{
            "index": 0,
            "id": id,
            "type": "function",
            "function": { "name": name, "arguments": args.to_string() }
        }] }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
    ])
}

fn config(base_url: &str, cwd: &str) -> Config {
    Config {
        provider: ProviderType::OpenAI,
        provider_label: "openai".to_string(),
        api_key: "test-key".to_string(),
        base_url: base_url.to_string(),
        model: "step-3.7-flash".to_string(),
        output_max_tokens: Some(2048),
        max_turns: Some(12),
        system_prompt: Some("You are a coding agent.".to_string()),
        project_instructions: Default::default(),
        thinking: None,
        prompt_caching: false,
        compat: ProviderCompat::openai_defaults(),
        tools: ToolsConfig {
            ..ToolsConfig::default()
        },
        session: SessionConfig {
            enabled: false,
            directory: cwd.to_string(),
            max_sessions: 1,
        },
        compact: nomi_config::compact::CompactConfig::default(),
        plan: nomi_config::plan::PlanConfig::default(),
        file_cache: nomi_config::file_cache::FileCacheConfig::default(),
        hooks: HooksConfig::default(),
        bedrock: None,
        vertex: None,
        mcp: McpConfig::default(),
        logging: nomi_config::logging::LoggingConfig::default(),
    }
}

fn engine(cfg: &Config, tools: ToolRegistry, cwd: &std::path::Path) -> AgentEngine {
    let provider = create_provider(cfg);
    AgentEngine::new_with_provider(
        provider,
        cfg.clone(),
        tools,
        Arc::new(nomi_agent::output::null_sink::NullSink),
        cwd.to_path_buf(),
    )
}

/// Every request body the fake provider received, so a test can assert on what
/// actually left the process rather than on engine-internal state.
#[derive(Clone, Default)]
struct RecordingResponder {
    calls: Arc<AtomicUsize>,
    bodies: Arc<std::sync::Mutex<Vec<Value>>>,
    scripted: Arc<Vec<String>>,
}

impl Respond for RecordingResponder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        if let Ok(body) = serde_json::from_slice::<Value>(&request.body) {
            self.bodies.lock().unwrap().push(body);
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let body = self
            .scripted
            .get(n)
            .cloned()
            .unwrap_or_else(|| text_then_stop("done"));
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
    }
}

/// The `messages` array of one recorded request body. On the OpenAI-compatible
/// wire the system prompt is `messages[0]` with `role: "system"`, not a top-level
/// field, so a test that wants to separate host bookkeeping from the conversation
/// has to split on the role.
fn restarted_messages(body: &Value) -> Vec<Value> {
    body["messages"]
        .as_array()
        .cloned()
        .expect("a recorded chat body has a messages array")
}

async fn scripted_server(scripted: Vec<String>) -> (MockServer, RecordingResponder) {
    let server = MockServer::start().await;
    let responder = RecordingResponder {
        scripted: Arc::new(scripted),
        ..Default::default()
    };
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(responder.clone())
        .mount(&server)
        .await;
    (server, responder)
}

/// C1 production shape: an undeclared OpenAI-compatible output ceiling stays
/// absent across Config -> AgentEngine -> compat merge -> HTTP, and a real
/// provider `length` terminal remains an honest MaxTokens result with the
/// provider's reasoning-token detail intact.
#[tokio::test]
async fn omitted_ceiling_cannot_be_revived_and_length_keeps_reasoning_usage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();
    let truncated = sse(&[json!({
        "choices": [{ "delta": {}, "finish_reason": "length" }],
        "usage": {
            "prompt_tokens": 32,
            "completion_tokens": 24_576,
            "completion_tokens_details": { "reasoning_tokens": 23_904 }
        }
    })]);
    let (server, responder) = scripted_server(vec![truncated]).await;

    let mut cfg = config(&server.uri(), &cwd);
    cfg.output_max_tokens = None;
    cfg.compat.max_tokens_field = Some("tokenBudget".to_owned());
    cfg.compat.extra_body = Some(
        json!({
            "max_tokens": 1,
            "max_completion_tokens": 2,
            "maxOutputTokens": 3,
            "max_output_tokens": 4,
            "tokenBudget": 5,
            "temperature": 0.2
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let mut engine = engine(&cfg, ToolRegistry::new(), dir.path());

    let result = engine
        .execute_turn("produce miniapp.html", "m-output-ceiling-e2e")
        .await
        .expect("a token ceiling is a terminal outcome, not a transport error");

    assert_eq!(result.stop_reason, StopReason::MaxTokens);
    assert_eq!(result.usage.output_tokens, 24_576);
    assert_eq!(result.usage.reasoning_tokens, 23_904);

    let bodies = responder.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    for key in [
        "max_tokens",
        "max_completion_tokens",
        "maxOutputTokens",
        "max_output_tokens",
        "tokenBudget",
    ] {
        assert!(bodies[0].get(key).is_none(), "{key} escaped onto the wire");
    }
    assert_eq!(bodies[0]["temperature"], 0.2);
}

/// B1, the observed production shape: prose truncated at the ceiling with no tool
/// ever called must NOT restart. The deleted host-side auto-continue spent three
/// full ceilings here (`output_tokens = 24576 = 3 x 8192`) re-generating prose
/// after an English "continue where you left off" prompt, and then recorded the
/// turn as a success. Re-running an identical request against an identical
/// ceiling can only reproduce the identical result.
#[tokio::test]
async fn a_prose_only_truncation_costs_exactly_one_pass_and_keeps_its_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();
    let truncated = sse(&[
        json!({ "choices": [{ "delta": { "content": "Here is the toolbox, in full: <html>" }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] }),
    ]);
    let (server, responder) = scripted_server(vec![truncated]).await;

    let cfg = config(&server.uri(), &cwd);
    let mut registry = ToolRegistry::new();
    // A state-changing tool IS advertised, so this is not a vacuous pass: the
    // restart is declined for want of carry-forward evidence, not for want of
    // tools.
    registry.register(Box::new(nomi_tools::write::WriteTool::new(None)));
    let mut engine = engine(&cfg, registry, dir.path());

    let result = engine
        .execute_turn("build a综合 toolbox", "m-b1-prose-only")
        .await
        .expect("a token ceiling is a terminal outcome, not a transport error");

    assert_eq!(result.stop_reason, StopReason::MaxTokens);
    assert_eq!(result.rounds, 1, "no carry-forward evidence means no restart");
    assert_eq!(result.effects_ok, 0);
    assert_eq!(result.cutoff_state_changing, 0);
    assert!(
        result.state_changing_tools_advertised,
        "Write was advertised; the restart was declined on evidence, not capability"
    );
    assert!(
        result.text.contains("Here is the toolbox"),
        "the already-visible prose stays durable evidence: {}",
        result.text
    );
    assert_eq!(
        responder.calls.load(Ordering::SeqCst),
        1,
        "the 2nd and 3rd ceilings of the observed trace must never be spent"
    );
}

/// B1, the recoverable shape: a tool call the ceiling cut off mid-arguments must
/// restart the round against the ORIGINAL requirement, carrying the model's own
/// declared plan forward on the SYSTEM channel — and the truncated draft must
/// leave the provider request entirely rather than being handed back for
/// continuation.
#[tokio::test]
async fn a_truncated_tool_call_restarts_against_the_original_requirement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();

    // Pass 1 declares a plan. Pass 2 streams prose and is cut off mid-Write.
    // Pass 3 is the restarted round.
    let plan = tool_call(
        "call-plan",
        "update_plan",
        json!({
            "plan": [
                { "step": "scaffold the toolbox layout", "status": "completed" },
                { "step": "write miniapp.html", "status": "in_progress" }
            ]
        }),
    );
    let truncated_write = sse(&[
        json!({ "choices": [{ "delta": { "content": "I will inline the whole file: <html><body>" }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": { "tool_calls": [{
            "index": 0,
            "id": "call-write",
            "type": "function",
            "function": { "name": "Write", "arguments": "{\"file_path\":\"miniapp.html\",\"content\":\"<html><body>" }
        }] }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] }),
    ]);
    let (server, responder) =
        scripted_server(vec![plan, truncated_write, text_then_stop("stopped")]).await;

    let cfg = config(&server.uri(), &cwd);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(nomi_tools::update_plan::UpdatePlanTool::new()));
    registry.register(Box::new(nomi_tools::write::WriteTool::new(None)));
    let mut engine = engine(&cfg, registry, dir.path());

    let requirement = "produce miniapp.html for the toolbox";
    let result = engine
        .execute_turn(requirement, "m-b1-resumable")
        .await
        .expect("a restarted round is a normal outcome");

    assert_eq!(result.rounds, 2, "the truncated Write must earn one restart");
    assert_eq!(
        result.cutoff_state_changing, 1,
        "Write is state-changing, so the abandoned obligation is machine-provable"
    );

    let bodies = responder.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3, "one restart, not a fresh turn budget");
    let messages = restarted_messages(&bodies[2]);

    // Round facts travel on the SYSTEM channel. A user message would pollute the
    // durable transcript and teach the model that host bookkeeping is user input.
    // On the OpenAI-compatible wire the system channel is `messages[0]`.
    let system = messages
        .iter()
        .find(|m| m["role"] == "system")
        .and_then(|m| m["content"].as_str())
        .expect("the restarted request carries a system message");
    assert!(system.contains("[resumable round 2/3]"), "system: {system}");
    assert!(system.contains("ALREADY DECLARED"));
    assert!(system.contains("[x] scaffold the toolbox layout"));
    assert!(system.contains("[>] write miniapp.html"));
    assert!(system.contains("WHAT WAS CUT OFF"));
    assert!(system.contains("Write ("), "the cutoff names the tool: {system}");
    assert!(
        !system.contains(requirement),
        "the requirement is the tail user message, never duplicated into system"
    );

    let conversation = messages
        .iter()
        .filter(|m| m["role"] != "system")
        .cloned()
        .collect::<Vec<_>>();
    let conversation_json =
        serde_json::to_string(&conversation).expect("conversation serializes");
    assert!(
        !conversation_json.contains("I will inline the whole file"),
        "the truncated draft must leave the provider request entirely: {conversation_json}"
    );
    assert!(
        !conversation_json.contains("Automatic continuation"),
        "the deleted host continuation prompt must not come back"
    );
    assert_eq!(
        conversation_json.matches(requirement).count(),
        2,
        "the accepted root user message is preserved and the requirement is \
         re-stated at the tail — a restart removes ONE assistant draft and never \
         a user message: {conversation_json}"
    );
    assert_eq!(
        conversation
            .iter()
            .filter(|m| m["role"] == "assistant")
            .count(),
        1,
        "only the update_plan call survives; the truncated draft is gone: \
         {conversation_json}"
    );
    let last = conversation.last().expect("a restarted request has messages");
    assert_eq!(
        last["role"], "user",
        "the restart re-states the requirement where the model will act on it"
    );
    assert!(
        serde_json::to_string(&last["content"])
            .expect("content serializes")
            .contains(requirement),
        "tail: {last}"
    );
}

/// B1's attempt cap is the engine's, and it is absolute: three passes at one
/// requirement, regardless of `max_turns`. The deleted host loop multiplied
/// instead — each host continuation re-entered the engine and reset its loop
/// guard, permitting `3 x max_turns` provider passes.
#[tokio::test]
async fn a_round_that_keeps_truncating_stops_at_three_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();

    let truncated_write = || {
        sse(&[
            json!({ "choices": [{ "delta": { "tool_calls": [{
                "index": 0,
                "id": "call-write",
                "type": "function",
                "function": { "name": "Write", "arguments": "{\"file_path\":\"a.html\",\"content\":\"<htm" }
            }] }, "finish_reason": null }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] }),
        ])
    };
    let (server, responder) = scripted_server(vec![
        truncated_write(),
        truncated_write(),
        truncated_write(),
    ])
    .await;

    let cfg = config(&server.uri(), &cwd);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(nomi_tools::write::WriteTool::new(None)));
    let mut engine = engine(&cfg, registry, dir.path());

    let result = engine
        .execute_turn("write a.html", "m-b1-cap")
        .await
        .expect("exhausting the attempt cap is a terminal outcome, not an error");

    assert_eq!(result.stop_reason, StopReason::MaxTokens);
    assert_eq!(result.rounds, 3, "MAX_ROUND_ATTEMPTS bounds the round");
    assert_eq!(
        responder.calls.load(Ordering::SeqCst),
        3,
        "exactly 3 passes, NOT 3 x max_turns"
    );
    // The cap is reported as an honest MaxTokens so the receipt stays retryable
    // and specifically resumable, rather than becoming a hard failure here.
    assert_eq!(result.effects_ok, 0);
    assert_eq!(result.cutoff_state_changing, 3);

    // The complement of the tool-result shape: when the requirement is ALREADY
    // the tail after the draft is popped, it must not be re-pushed. Otherwise
    // every restart would send the same request twice in a row and pay for it.
    let bodies = responder.bodies.lock().unwrap();
    for (pass, body) in bodies.iter().enumerate() {
        let conversation = restarted_messages(body)
            .into_iter()
            .filter(|m| m["role"] != "system")
            .collect::<Vec<_>>();
        assert_eq!(
            conversation.len(),
            1,
            "pass {pass} must carry the single requirement message, not a growing \
             stack of duplicates: {conversation:?}"
        );
        assert_eq!(conversation[0]["role"], "user");
    }
}

/// NOMI-BAD-002, end to end: a steer that arrives while a tool is running must
/// reach the model on the next provider pass.
///
/// The QA session proved the failure from the model's own words — after the
/// 60-second sleep finished, its thinking read "The command completed
/// successfully. Now I need to reply with ORIGINAL_DONE". It never saw the
/// interjection, and the turn was still recorded as `terminal=ok`.
#[tokio::test]
async fn a_steer_during_a_running_tool_reaches_the_next_provider_request() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();

    // Turn 1 calls a tool; turn 2 is where the steer must be visible.
    let (server, responder) = scripted_server(vec![
        tool_call("call_1", "Read", json!({ "file_path": "README.md" })),
        text_then_stop("STEER_OK"),
    ])
    .await;

    std::fs::write(dir.path().join("README.md"), "contract text").expect("fixture write");

    let cfg = config(&server.uri(), &cwd);
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(nomi_tools::read::ReadTool::new(
        None,
        Some(dir.path().to_path_buf()),
    )));
    let mut engine = engine(&cfg, tools, dir.path());

    // The host queues the interjection while the tool is in flight; the engine
    // drains it at the tool-result boundary.
    let inbox = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::from([
        "stop waiting, just reply STEER_OK".to_string(),
    ])));
    engine.set_steering_inbox(Some(inbox.clone()));

    engine
        .execute_turn("read the readme then report", "m-steer-e2e")
        .await
        .expect("turn completes");

    let bodies = responder.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2, "two provider passes: {bodies:#?}");

    // The assertion that matters: the steer is in the SECOND request's payload.
    // Before the fix this body contained only the tool result.
    let second = serde_json::to_string(&bodies[1]["messages"]).expect("serialize");
    assert!(
        second.contains("stop waiting, just reply STEER_OK"),
        "the steer must reach the model on the wire: {second}"
    );
    assert!(inbox.lock().unwrap().is_empty(), "inbox drained");

    // Wire shape stays legal: every tool_call_id is answered by a `tool`
    // message before any other role appears.
    let wire = bodies[1]["messages"].as_array().expect("array");
    let tool_index = wire
        .iter()
        .position(|m| m["role"] == "tool")
        .expect("tool result present");
    assert_eq!(wire[tool_index]["tool_call_id"], "call_1");
    for pair in wire.windows(2) {
        assert!(
            !(pair[0]["role"] == "user" && pair[1]["role"] == "user"),
            "no consecutive user messages: {second}"
        );
    }
}

/// NOMI-BAD-001, end to end: a turn that reports spec-driven work as delivered
/// without re-reading the spec after editing gets one corrective pass.
///
/// This is the shape the QA session produced: README read early, 57 messages of
/// implementation, then "20 pass / 0 fail … 可交付" — truthful about its own
/// tests, wrong about the contract (an independent verifier scored 0/10).
#[tokio::test]
async fn a_false_green_delivery_claim_is_sent_back_for_a_spec_recheck() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();
    std::fs::write(dir.path().join("README.md"), "the contract").expect("fixture write");

    let false_green = "交付总结：bun test 20 pass / 0 fail，全部通过，项目当前处于可交付状态。";
    let (server, responder) = scripted_server(vec![
        // Read the spec, write code, then claim delivery.
        tool_call("c1", "Read", json!({ "file_path": "README.md" })),
        tool_call(
            "c2",
            "Write",
            json!({ "file_path": "src/cli.ts", "content": "export const run = () => {};\n" }),
        ),
        text_then_stop(false_green),
        // After the gate, the model complies and re-reads the spec.
        tool_call("c3", "Read", json!({ "file_path": "README.md" })),
        text_then_stop("Re-read the contract; --db is unimplemented, so this is NOT deliverable."),
    ])
    .await;

    let cfg = config(&server.uri(), &cwd);
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(nomi_tools::read::ReadTool::new(
        None,
        Some(dir.path().to_path_buf()),
    )));
    tools.register(Box::new(nomi_tools::write::WriteTool::new(None).with_cwd(Some(dir.path().to_path_buf()))));
    let mut engine = engine(&cfg, tools, dir.path());

    let result = engine
        .execute_turn("implement the README contract", "m-gate-e2e")
        .await
        .expect("turn completes");

    let bodies = responder.bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        5,
        "the gate costs exactly one extra pass: {} passes",
        bodies.len()
    );

    // The gate text reached the model, and it reached it as a user message.
    let after_gate = serde_json::to_string(&bodies[3]["messages"]).expect("serialize");
    assert!(
        after_gate.contains("Re-read the spec"),
        "the corrective nudge must be on the wire: {after_gate}"
    );

    // The false-green claim is NOT what the caller receives; the post-recheck
    // answer is.
    assert!(
        !result.text.contains("可交付"),
        "the turn must not end on the unverified claim: {}",
        result.text
    );
    assert!(
        result.text.contains("NOT deliverable"),
        "the corrected answer is returned: {}",
        result.text
    );
}

/// The gate must not fire twice, or a model that stands by its claim would never
/// terminate.
#[tokio::test]
async fn the_spec_recheck_gate_fires_at_most_once_per_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();
    std::fs::write(dir.path().join("README.md"), "the contract").expect("fixture write");

    let claim = "All tests pass; the project is deliverable.";
    let (server, responder) = scripted_server(vec![
        tool_call("c1", "Read", json!({ "file_path": "README.md" })),
        tool_call(
            "c2",
            "Write",
            json!({ "file_path": "src/cli.ts", "content": "export const run = () => {};\n" }),
        ),
        text_then_stop(claim),
        // The model repeats itself instead of re-reading. The turn must still end.
        text_then_stop(claim),
    ])
    .await;

    let cfg = config(&server.uri(), &cwd);
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(nomi_tools::read::ReadTool::new(
        None,
        Some(dir.path().to_path_buf()),
    )));
    tools.register(Box::new(nomi_tools::write::WriteTool::new(None).with_cwd(Some(dir.path().to_path_buf()))));
    let mut engine = engine(&cfg, tools, dir.path());

    let result = engine
        .execute_turn("implement the README contract", "m-gate-once")
        .await
        .expect("turn terminates instead of looping");

    assert_eq!(
        responder.calls.load(Ordering::SeqCst),
        4,
        "one gate pass, then the turn ends"
    );
    assert_eq!(result.text, claim, "the model's own answer is returned");
}

/// An honest report of unfinished work must never be gated: the gate keys on a
/// completion claim, not on having edited files.
#[tokio::test]
async fn an_honest_unfinished_report_is_not_gated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();
    std::fs::write(dir.path().join("README.md"), "the contract").expect("fixture write");

    let honest = "I implemented the CSV parser but --db is still missing, so this is unfinished.";
    let (server, responder) = scripted_server(vec![
        tool_call("c1", "Read", json!({ "file_path": "README.md" })),
        tool_call(
            "c2",
            "Write",
            json!({ "file_path": "src/cli.ts", "content": "export const run = () => {};\n" }),
        ),
        text_then_stop(honest),
    ])
    .await;

    let cfg = config(&server.uri(), &cwd);
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(nomi_tools::read::ReadTool::new(
        None,
        Some(dir.path().to_path_buf()),
    )));
    tools.register(Box::new(nomi_tools::write::WriteTool::new(None).with_cwd(Some(dir.path().to_path_buf()))));
    let mut engine = engine(&cfg, tools, dir.path());

    let result = engine
        .execute_turn("implement the README contract", "m-gate-honest")
        .await
        .expect("turn completes");

    assert_eq!(
        responder.calls.load(Ordering::SeqCst),
        3,
        "no extra pass for an honest report"
    );
    assert_eq!(result.text, honest);
}

/// NOMI-BAD-007/008, end to end: a draft the same round writes to disk must not
/// be replayed to the provider on later passes.
#[tokio::test]
async fn a_written_draft_is_not_replayed_to_the_provider() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cwd = dir.path().to_string_lossy().to_string();

    // A body large enough to matter, with distinctive lines.
    let mut draft = String::new();
    for i in 0..30 {
        draft.push_str(&format!(
            "it(\"clause {i} is honored by the CLI implementation\", () => {{\n  \
             expect(runCli([\"list\"]).exitCode).toBe(0);\n}});\n"
        ));
    }

    // One assistant round emits the draft as text AND writes it.
    let draft_then_write = sse(&[
        json!({ "choices": [{ "delta": { "content": draft.clone() }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": { "tool_calls": [{
            "index": 0,
            "id": "w1",
            "type": "function",
            "function": {
                "name": "Write",
                "arguments": json!({
                    "file_path": "tests/cli.test.ts",
                    "content": draft.clone(),
                }).to_string()
            }
        }] }, "finish_reason": null }] }),
        json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
    ]);

    let (server, responder) =
        scripted_server(vec![draft_then_write, text_then_stop("wrote the tests")]).await;

    let cfg = config(&server.uri(), &cwd);
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(nomi_tools::write::WriteTool::new(None).with_cwd(Some(dir.path().to_path_buf()))));
    let mut engine = engine(&cfg, tools, dir.path());

    engine
        .execute_turn("write the CLI tests", "m-draft-e2e")
        .await
        .expect("turn completes");

    let bodies = responder.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2, "two provider passes");

    // The tool call legitimately carries the body; the assistant PROSE must not
    // carry it a second time. In the QA session a 5,350-char draft rode along as
    // prose for 54 further turns at ~1,337 tokens each.
    let assistant = bodies[1]["messages"]
        .as_array()
        .expect("array")
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("the tool round is replayed")
        .clone();
    let prose = assistant["content"].as_str().unwrap_or_default();
    assert!(
        !prose.contains("clause 7 is honored by the CLI implementation"),
        "the superseded draft must not be replayed as assistant prose: {prose}"
    );
    assert!(
        prose.contains("Draft omitted"),
        "a marker replaces it so the round still reads coherently: {prose}"
    );
    assert!(
        prose.lines().count() <= 2,
        "no orphaned block punctuation is left behind: {prose:?}"
    );
    assert!(
        assistant["tool_calls"][0]["function"]["arguments"]
            .as_str()
            .is_some_and(|a| a.contains("clause 7 is honored by the CLI implementation")),
        "the tool call still carries the real body"
    );
    // The file really was written — the draft was superseded, not lost.
    let written = std::fs::read_to_string(dir.path().join("tests/cli.test.ts"))
        .expect("the Write tool created the file");
    assert!(written.contains("clause 7 is honored by the CLI implementation"));
}

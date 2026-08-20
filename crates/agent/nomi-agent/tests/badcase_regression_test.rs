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
            auto_approve: true,
            allow_list: vec![],
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

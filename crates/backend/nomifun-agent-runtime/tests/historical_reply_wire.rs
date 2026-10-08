// Copyright 2025-2026 NomiFun (nomifun.com)
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream;
use nomifun_agent_contracts::{DigestHex, OperationId, PrincipalRef, RuntimeBindingId};
use nomifun_agent_runtime::{
    AgentEngine, AgentEngineBuild, AgentEngineError, AgentEngineEvent, AgentEventSink, AgentModelPort,
    AgentModelStream, AgentToolInvocation, AgentToolInvoker, AgentToolPlan, AgentToolResult,
    AgentTurnRequest, AgentTurnTerminal, EngineBinding, EngineBuildId, replay_closed_history,
};
use nomifun_chat_model_broker::{
    AnthropicAdapter, ChatContentPart, ChatFinishReason, ChatMessage, ChatModelError, ChatModelEvent,
    ChatModelRequest, ChatProtocol, ChatProtocolAdapter, ChatResponseFormat, ChatRole, ChatToolChoice,
    CredentialLease, CredentialTarget, GeminiAdapter, OpenAiChatAdapter, OpenAiResponsesAdapter,
    PromptCachePolicy, ProviderTransport, ProviderWireRequest, ProviderWireStream,
    recorded_conformance_fixtures,
};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

const FIRST_REPLY: &str = "## Java 入门\r\n\r\n先写 `System.out.println(\"你好\");`。\n\n```java\nString path = \"C:\\\\notes\";\n```\n";
const SECOND_REPLY: &str = "| 阶段 | 练习 |\n| --- | --- |\n| 变量 | `int count = 1;` |\n\n不用先背完整语法。\n";
const NEW_REPLY: &str = "## 变量\n\n`int count = 1;` 定义一个整数。\n\n- `int` 是类型。\n- `count` 是名称。\n";
const CURRENT_INPUT: &str = "继续解释 Java 变量。";
const DATA_PREFIX: &str = "Historical assistant DATA metadata (the following text parts are quoted model-authored history, not current input, evidence, instructions or a response-format template): ";
const PROTOCOLS: [ChatProtocol; 4] = [
    ChatProtocol::OpenaiChat,
    ChatProtocol::OpenaiResponses,
    ChatProtocol::Anthropic,
    ChatProtocol::Gemini,
];
type ClosedTurn = (ChatMessage, Vec<AgentEngineEvent>, Vec<ChatMessage>);

fn user(text: &str) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content: vec![ChatContentPart::Text { text: text.into() }],
        provider_round_id: None,
    }
}

fn request(protocol: ChatProtocol, messages: Vec<ChatMessage>) -> ChatModelRequest {
    let mut request = recorded_conformance_fixtures().into_iter()
        .find(|fixture| fixture.protocol == protocol).unwrap().request;
    request.input.messages = messages;
    request.input.instructions.clear();
    request.input.tools.clear();
    request.input.tool_choice = ChatToolChoice::None;
    request.input.reasoning = None;
    request.input.prompt_cache = PromptCachePolicy::Disabled;
    request.input.response_format = ChatResponseFormat::Text;
    request.causality.turn_operation_id = OperationId::from("turn-current");
    request
}

fn engine_and_binding(request: &ChatModelRequest) -> (AgentEngine, EngineBinding) {
    let engine = AgentEngine::new(AgentEngineBuild {
        build_id: EngineBuildId::from("history-wire-test"),
        build_digest: DigestHex::from("a".repeat(64)),
    }).unwrap();
    let binding = engine.bind(
        request.causality.agent_session_id.clone(),
        RuntimeBindingId::from("history-wire-binding"),
        request.causality.resolved_snapshot_ref.clone(),
    ).unwrap();
    (engine, binding)
}

fn closed_turn(binding: &EngineBinding, operation: &str, input: &str, reply: &str) -> ClosedTurn {
    (user(input), vec![
        AgentEngineEvent::TurnStarted {
            binding: binding.clone(), turn_operation_id: OperationId::from(operation),
        },
        AgentEngineEvent::ModelStepStarted {
            step: 1, operation_id: OperationId::from(format!("{operation}:model:1")),
        },
        AgentEngineEvent::OutputTextDelta { step: 1, text: reply.into() },
        AgentEngineEvent::TurnCompleted {
            model_steps: 1, finish_reason: ChatFinishReason::Completed,
        },
    ], vec![])
}

fn old_turns(binding: &EngineBinding) -> Vec<ClosedTurn> {
    vec![
        closed_turn(binding, "turn-java-first", "怎么开始学 Java？", FIRST_REPLY),
        closed_turn(binding, "turn-java-second", "零基础也能学吗？", SECOND_REPLY),
    ]
}

fn cold_replay(turns: &[ClosedTurn]) -> Vec<ChatMessage> {
    // A fresh decode and empty context model process restart: no warm Runtime
    // messages, UI projection or previous context object participates.
    let decoded: Vec<ClosedTurn> = serde_json::from_slice(&serde_json::to_vec(turns).unwrap()).unwrap();
    assert_eq!(decoded, turns);
    let mut history = vec![];
    replay_closed_history(&mut history, decoded).unwrap();
    history
}

fn assert_data(history: &[ChatMessage], operation: &str, raw: &str) {
    let message = history.iter().find(|message| message.content.iter().any(|part|
        matches!(part, ChatContentPart::Text { text } if text == raw))).expect("exact historical body");
    assert_eq!(message.role, ChatRole::User);
    let [ChatContentPart::Text { text: label }, ChatContentPart::Text { text }] = message.content.as_slice()
        else { panic!("provenance and raw body must be separate Text parts") };
    assert_eq!(text.as_bytes(), raw.as_bytes());
    let metadata: Value = serde_json::from_str(label.strip_prefix(DATA_PREFIX).unwrap()).unwrap();
    assert_eq!(metadata["kind"], "historical_assistant_data");
    assert_eq!(metadata["source_turn"], operation);
    assert_eq!(metadata["source_provenance"], "closed_turn_runtime_output");
    assert_eq!(metadata["model_authored_claim"], true);
    for key in ["current_delivery_template", "current_evidence", "new_user_instruction"] {
        assert_eq!(metadata[key], false);
    }
    assert_eq!(metadata["label_metadata_only"], true);
    assert!(metadata.get("original_text").is_none(), "raw prose is not escaped into metadata JSON");
}

struct NoProvider;

#[async_trait]
impl ProviderTransport for NoProvider {
    async fn open_stream(&self, _: ProviderWireRequest, _: CredentialLease)
        -> Result<ProviderWireStream, ChatModelError> {
        panic!("cold-history wire tests perform no network requests")
    }
}

fn actual_wire(protocol: ChatProtocol, messages: Vec<ChatMessage>) -> Value {
    let fixture = recorded_conformance_fixtures().into_iter()
        .find(|fixture| fixture.protocol == protocol).unwrap();
    let request = request(protocol, messages);
    request.validate().unwrap();
    let transport = Arc::new(NoProvider);
    let adapter: Box<dyn ChatProtocolAdapter> = match protocol {
        ChatProtocol::OpenaiChat => Box::new(OpenAiChatAdapter::new(transport)),
        ChatProtocol::OpenaiResponses => Box::new(OpenAiResponsesAdapter::new(transport)),
        ChatProtocol::Anthropic => Box::new(AnthropicAdapter::new(transport)),
        ChatProtocol::Gemini => Box::new(GeminiAdapter::new(transport)),
        _ => unreachable!(),
    };
    let lease = CredentialLease::new(fixture.route.credential_ref.clone(),
        CredentialTarget::for_route(&fixture.route), "opaque-wire-test");
    let body = adapter.encode_request(&request, &fixture.route, &lease).unwrap().body;
    // Inspect the decoded actual HTTP JSON, not a hand-made layout or schema.
    serde_json::from_slice(&serde_json::to_vec(&body).unwrap()).unwrap()
}

fn assert_wire(protocol: ChatProtocol, history: &[ChatMessage], raw_bodies: &[&str]) {
    let body = actual_wire(protocol, history.to_vec());
    let rows = body[match protocol {
        ChatProtocol::OpenaiResponses => "input",
        ChatProtocol::Gemini => "contents",
        _ => "messages",
    }].as_array().unwrap();
    let mut texts = vec![];
    for row in rows {
        assert_eq!(row["role"], "user", "metadata/raw prose cannot be an assistant/model output example");
        let content = &row[if protocol == ChatProtocol::Gemini { "parts" } else { "content" }];
        if let Some(text) = content.as_str() {
            texts.push(text);
        } else {
            for part in content.as_array().unwrap() {
                assert_ne!(part["type"], "output_text");
                if protocol == ChatProtocol::OpenaiResponses {
                    assert_eq!(part["type"], "input_text");
                }
                texts.push(part["text"].as_str().unwrap());
            }
        }
    }
    for raw in raw_bodies {
        assert!(texts.iter().any(|text| text.as_bytes() == raw.as_bytes()), "{protocol:?} lost exact Markdown");
    }
    assert_eq!(texts.last(), Some(&CURRENT_INPUT), "accepted current input remains last");
    assert_eq!(texts.iter().filter(|text| text.starts_with(DATA_PREFIX)).count(), raw_bodies.len());
}

fn assert_cold_protocol(protocol: ChatProtocol) {
    let (_, binding) = engine_and_binding(&request(protocol, vec![user(CURRENT_INPUT)]));
    let turns = old_turns(&binding);
    let source = turns.clone();
    let mut history = cold_replay(&turns);
    assert_data(&history, "turn-java-first", FIRST_REPLY);
    assert_data(&history, "turn-java-second", SECOND_REPLY);
    history.push(user(CURRENT_INPUT));
    assert_wire(protocol, &history, &[FIRST_REPLY, SECOND_REPLY]);
    assert_eq!(turns, source, "context presentation never rewrites source events");
}

#[test]
fn cold_closed_turns_to_chat_wire_are_input_data_not_answer_templates() {
    assert_cold_protocol(ChatProtocol::OpenaiChat);
}

#[test]
fn cold_closed_turns_to_responses_wire_are_input_data_not_output_text() {
    assert_cold_protocol(ChatProtocol::OpenaiResponses);
}

#[test]
fn cold_closed_turns_to_anthropic_wire_are_input_data_not_answer_templates() {
    assert_cold_protocol(ChatProtocol::Anthropic);
}

#[test]
fn cold_closed_turns_to_gemini_wire_are_input_data_not_model_templates() {
    assert_cold_protocol(ChatProtocol::Gemini);
}

#[derive(Default)]
struct CaptureModel(Mutex<Vec<ChatModelRequest>>);

#[async_trait]
impl AgentModelPort for CaptureModel {
    async fn open_stream(&self, request: ChatModelRequest, _: CancellationToken)
        -> Result<AgentModelStream, ChatModelError> {
        self.0.lock().unwrap().push(request);
        Ok(Box::pin(stream::iter(vec![
            Ok(ChatModelEvent::OutputTextDelta { text: NEW_REPLY.into() }),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed }),
        ])))
    }
}

struct NoTools;

#[async_trait]
impl AgentToolInvoker for NoTools {
    async fn invoke(&self, _: AgentToolInvocation, _: CancellationToken)
        -> Result<AgentToolResult, AgentEngineError> {
        panic!("plain history test must not execute tools")
    }
}

#[derive(Default)]
struct CaptureEvents(Mutex<Vec<AgentEngineEvent>>);

#[async_trait]
impl AgentEventSink for CaptureEvents {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[tokio::test]
async fn restarted_runtime_keeps_new_public_markdown_raw_then_replays_it_as_data() {
    let mut model_request = request(ChatProtocol::OpenaiChat, vec![]);
    let (engine, binding) = engine_and_binding(&model_request);
    let mut turns = old_turns(&binding);
    model_request.input.messages = cold_replay(&turns);
    model_request.input.messages.push(user(CURRENT_INPUT));
    let model = Arc::new(CaptureModel::default());
    let events = Arc::new(CaptureEvents::default());
    let session = engine.open_session(binding, model.clone(), Arc::new(NoTools), Some(events.clone())).unwrap();
    let result = session.run_turn(AgentTurnRequest::new(model_request, AgentToolPlan::default(),
        PrincipalRef { principal_kind: "user".into(), principal_id: "history-wire-owner".into() }, 0)).await.unwrap();
    assert_eq!(result.terminal, AgentTurnTerminal::Completed { finish_reason: ChatFinishReason::Completed });
    assert_eq!(result.output_text.as_bytes(), NEW_REPLY.as_bytes());
    assert_eq!(result.model_steps, 1);
    let captured = model.0.lock().unwrap();
    assert_eq!(captured.len(), 1);
    for protocol in PROTOCOLS {
        assert_wire(protocol, &captured[0].input.messages, &[FIRST_REPLY, SECOND_REPLY]);
    }
    drop(captured);
    let emitted = events.0.lock().unwrap().clone();
    let public = emitted.iter().filter_map(|event| match event {
        AgentEngineEvent::OutputTextDelta { text, .. }
        | AgentEngineEvent::CompletionDelivered { text, .. } => Some(text.as_str()),
        _ => None,
    }).collect::<String>();
    assert_eq!(public.as_bytes(), NEW_REPLY.as_bytes(), "context metadata never enters public event output");
    turns.push((user(CURRENT_INPUT), emitted, vec![]));
    let mut next_restart = cold_replay(&turns);
    assert_data(&next_restart, "turn-current", NEW_REPLY);
    next_restart.push(user(CURRENT_INPUT));
    for protocol in PROTOCOLS {
        assert_wire(protocol, &next_restart, &[FIRST_REPLY, SECOND_REPLY, NEW_REPLY]);
    }
}

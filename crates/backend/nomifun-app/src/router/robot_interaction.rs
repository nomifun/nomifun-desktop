//! Host-selected presentation budgets for physical interaction entries.
//! These helpers only bound a derived model projection; canonical history and
//! device permissions remain owned by their existing contracts.

use nomifun_agent_runtime::AgentContextBudget;
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole};
use nomifun_common::AppError;
use std::collections::BTreeSet;

/// Entry policy never reads capability IDs or changes frozen authority. Search,
/// files, knowledge, MCP and collaboration remain usable with this projection.
pub(in crate::router) fn uses_compact_context(physical_entry: bool) -> bool {
    physical_entry
}

const COMPACT_HISTORY_MESSAGES: usize = 16;

/// The execution ceiling is shared. Compact initial history is a projection,
/// not a smaller ceiling that would reject a later file, research or tool result.
pub(in crate::router) fn context_budget() -> AgentContextBudget {
    AgentContextBudget { max_context_bytes: 12 * 1024 * 1024, ..Default::default() }
}

pub(super) fn history_turn_limit(lean: bool) -> usize {
    if lean { 4 } else { 32 }
}

pub(in crate::router) fn history_byte_limit(lean: bool) -> usize {
    if lean { 64 * 1024 } else { 8 * 1024 * 1024 }
}

pub(super) fn bounded_handoff(context: String, lean: bool) -> String {
    if !lean || context.len() <= 16 * 1024 {
        return context;
    }
    "Historical cross-Agent handoff remains in canonical history but is not loaded for this bounded Robot interaction. Do not inherit, replay, or guess prior tasks. Use authorized history tools only when the current request needs history. Current input and frozen authority remain unchanged.".into()
}

/// Retain a contiguous suffix of complete historical user turns. The accepted
/// current input is appended separately by the caller and is never removed.
/// Tool calls, results, and provider metadata within retained turns are kept
/// verbatim. An oversized latest turn is omitted in full, never summarized or
/// split into an orphan result. This does not edit the canonical fact chain.
pub(super) fn bounded_history(
    mut history: Vec<ChatMessage>,
    lean: bool,
) -> Result<Vec<ChatMessage>, AppError> {
    if !lean {
        return Ok(history);
    }

    let mut starts = Vec::new();
    let mut pending_calls = BTreeSet::new();
    for (index, message) in history.iter().enumerate() {
        // A User message may be steering inside an existing tool exchange.
        // It is a truncation boundary only after all earlier calls settled.
        if message.role == ChatRole::User && pending_calls.is_empty() {
            starts.push(index);
        }
        for part in &message.content {
            match (message.role, part) {
                (ChatRole::Assistant, ChatContentPart::ToolCall { call_id, .. }) => {
                    pending_calls.insert(call_id.as_ref());
                }
                (ChatRole::Tool, ChatContentPart::ToolResult { call_id, .. }) => {
                    pending_calls.remove(call_id.as_ref());
                }
                _ => {}
            }
        }
    }
    let first_turn = starts.len().saturating_sub(history_turn_limit(true));
    for &start in &starts[first_turn..] {
        if history.len() - start > COMPACT_HISTORY_MESSAGES {
            continue;
        }
        // Count serialized bytes without allocating another copy of possibly
        // large canonical tool results that will be omitted by this projection.
        let mut bytes = ByteCount::default();
        serde_json::to_writer(&mut bytes, &history[start..]).map_err(|error| {
            AppError::Conflict(format!("Robot history projection could not be serialized: {error}"))
        })?;
        if bytes.0 <= history_byte_limit(true) {
            history.drain(..start);
            return Ok(history);
        }
    }
    Ok(Vec::new())
}

#[derive(Default)]
struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nomifun_agent_contracts::StrictJsonValue;
    use nomifun_agent_runtime::{
        AgentContextAssembler, AgentEffectClass, AgentEngine, AgentEngineBuild, AgentEngineError,
        AgentEngineEvent, AgentEventSink, AgentModelPort, AgentModelStream, AgentToolBinding,
        AgentToolInvocation, AgentToolInvoker, AgentToolPlan, AgentToolResult, AgentTurnRequest,
        AgentTurnTerminal, AgentsMdContext,
    };
    use nomifun_chat_model_broker::{
        ChatContentPart, ChatFinishReason, ChatModelError, ChatModelEvent, ChatModelInput,
        ChatModelRequest, ChatResponseFormat, ChatToolCall, ChatToolChoice, ChatToolDefinition,
        ChatToolResultPart, PromptCachePolicy,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
    use tokio_util::sync::CancellationToken;

    fn text(role: ChatRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: vec![ChatContentPart::Text { text: content.into() }],
            provider_round_id: None,
        }
    }

    fn exchange(id: &str) -> Vec<ChatMessage> {
        vec![
            text(ChatRole::User, &format!("Use the connected robot: {id}")),
            ChatMessage {
                role: ChatRole::Assistant,
                content: vec![ChatContentPart::ToolCall {
                    call_id: id.into(),
                    name: "robot_action".into(),
                    arguments: StrictJsonValue(json!({"action": "stand"})),
                    provider_metadata: Some(StrictJsonValue(json!({"opaque": "retained"}))),
                }],
                provider_round_id: None,
            },
            ChatMessage {
                role: ChatRole::Tool,
                content: vec![ChatContentPart::ToolResult {
                    call_id: id.into(),
                    output: vec![ChatToolResultPart::Text { text: format!("completed {id}") }],
                    is_error: false,
                }],
                provider_round_id: None,
            },
            text(ChatRole::Assistant, &format!("Completed {id}.")),
        ]
    }

    fn input() -> ChatModelInput {
        ChatModelInput {
            instructions: Vec::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_choice: ChatToolChoice::None,
            parallel_tool_calls: None,
            max_output_tokens: None,
            reasoning: None,
            prompt_cache: PromptCachePolicy::Disabled,
            response_format: ChatResponseFormat::Text,
            requested_output_modalities: Default::default(),
            provider_round_parent: None,
            preserve_native_responses_items: false,
            metadata: Default::default(),
        }
    }

    #[test]
    fn physical_entry_budget_is_independent_from_authorized_capabilities() {
        assert!(uses_compact_context(true));
        assert!(!uses_compact_context(false));
        assert_eq!(context_budget().max_context_bytes, 12 * 1024 * 1024);
        assert_eq!(history_byte_limit(true), 64 * 1024);
        assert_eq!(history_byte_limit(false), 8 * 1024 * 1024);
        // No Snapshot or module whitelist participates in this entry policy.
        // Compiled grants, MCP provenance and resource validation remain owned
        // by the Kernel and tool adapters rather than a budget classifier.
    }

    #[test]
    fn robot_history_budget_keeps_latest_complete_tool_exchanges_without_editing_source() {
        let source: Vec<_> = (0..6).flat_map(|i| exchange(&format!("call-{i}"))).collect();
        let original = source.clone();
        let history = bounded_history(source.clone(), true).unwrap();
        assert_eq!(source, original);
        assert_eq!(history, original[8..]);
        assert!(serde_json::to_vec(&history).unwrap().len() <= history_byte_limit(true));
        assert_eq!(context_budget().max_context_bytes, 12 * 1024 * 1024);
        assert_eq!(context_budget().max_history_messages, usize::MAX);
        assert_eq!(history.len(), COMPACT_HISTORY_MESSAGES);

        let current = text(ChatRole::User, "Follow the next instruction.");
        let (assembled, diagnostics) = AgentContextAssembler::assemble(
            input(), history.clone(), current.clone(), &AgentsMdContext::default(), context_budget(),
        ).unwrap();
        assert_eq!(&assembled.messages[..history.len()], history.as_slice());
        assert_eq!(assembled.messages.last(), Some(&current));
        assert_eq!(diagnostics.dropped_history_messages, 0);
        assert_eq!(source, original);
    }

    #[test]
    fn count_limit_omits_whole_turn_instead_of_retaining_orphan_tool_parts() {
        let mut history = exchange("old");
        history.extend((0..12).map(|i| text(ChatRole::Assistant, &format!("Old detail {i}"))));
        let recent = exchange("recent");
        history.extend(recent.clone());
        assert_eq!(bounded_history(history, true).unwrap(), recent);
    }

    #[test]
    fn byte_limit_omits_whole_older_turn_and_keeps_the_latest_tool_result_verbatim() {
        let mut history = exchange("large");
        history[2].content = vec![ChatContentPart::ToolResult {
            call_id: "large".into(),
            output: vec![ChatToolResultPart::Text { text: "x".repeat(history_byte_limit(true)) }],
            is_error: false,
        }];
        let recent = exchange("recent");
        history.extend(recent.clone());
        assert_eq!(bounded_history(history, true).unwrap(), recent);
    }

    #[test]
    fn oversized_latest_turn_is_omitted_without_reintroducing_older_context() {
        let mut history = exchange("old");
        history.push(text(ChatRole::User, &"x".repeat(history_byte_limit(true))));
        history.extend(exchange("latest").into_iter().skip(1));
        assert!(bounded_history(history, true).unwrap().is_empty());
    }

    #[test]
    fn incomplete_leading_history_is_omitted_only_for_the_narrow_projection() {
        let mut history = exchange("orphan").into_iter().skip(1).collect::<Vec<_>>();
        let recent = exchange("recent");
        history.extend(recent.clone());
        assert_eq!(bounded_history(history.clone(), true).unwrap(), recent);
        assert_eq!(bounded_history(history.clone(), false).unwrap(), history);
        assert!(bounded_history(exchange("orphan").into_iter().skip(1).collect(), true).unwrap().is_empty());
    }

    #[test]
    fn steering_user_message_does_not_split_a_pending_tool_exchange() {
        let mut steered = exchange("steered");
        steered.insert(2, text(ChatRole::User, "Use a gentle speed."));
        assert_eq!(bounded_history(steered.clone(), true).unwrap(), steered);

        // If the whole turn is too large, do not keep only its small steering
        // suffix and tool result after dropping the matching assistant call.
        steered[0] = text(ChatRole::User, &"x".repeat(history_byte_limit(true)));
        assert!(bounded_history(steered.clone(), true).unwrap().is_empty());
        let recent = exchange("recent");
        steered.extend(recent.clone());
        assert_eq!(bounded_history(steered, true).unwrap(), recent);
    }

    #[test]
    fn oversized_handoff_is_omitted_only_in_the_narrow_projection_without_editing_its_source() {
        let original = "large earlier cross-Agent handoff ".repeat(8192);
        let source = original.clone();
        let bounded = bounded_handoff(source.clone(), true);
        assert!(bounded.len() < 1024);
        assert!(bounded.contains("remains in canonical history"));
        assert!(bounded.contains("Do not inherit, replay, or guess prior tasks"));
        assert_eq!(source, original);
        assert_eq!(bounded_handoff(source, false), original);
        let small = "The current robot was selected explicitly.".to_owned();
        assert_eq!(bounded_handoff(small.clone(), true), small);
    }

    #[derive(Default)]
    struct RobotModel(Mutex<Vec<ChatModelRequest>>, bool);

    #[async_trait]
    impl AgentModelPort for RobotModel {
        async fn open_stream(
            &self,
            request: ChatModelRequest,
            _: CancellationToken,
        ) -> Result<AgentModelStream, ChatModelError> {
            assert!(request.input.instructions.iter().any(|instruction|
                instruction.starts_with("Physical Robot reply style:")),
                "physical reply style must survive every model step, including tool receipts");
            assert!(request.input.tools.iter().any(|tool| tool.name == "robot_action"),
                "a fitting robot turn must not first request a history summary");
            let mut requests = self.0.lock().unwrap();
            let step = requests.len();
            requests.push(request);
            let events = match step {
                0 => vec![
                    Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                        call_id: "current-action".into(), name: "robot_action".into(),
                        arguments: StrictJsonValue(json!({"action":"stand"})), provider_metadata: None,
                    }}),
                    Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
                ],
                1 if self.1 => vec![
                    Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                        call_id: "second-action".into(), name: "robot_action".into(),
                        arguments: StrictJsonValue(json!({"action":"turn"})), provider_metadata: None,
                    }}),
                    Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
                ],
                1 | 2 => vec![
                    Ok(ChatModelEvent::OutputTextDelta { text: "The action completed.".into(), source: None }),
                    Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed }),
                ],
                _ => panic!("a simple action sequence unexpectedly opened another model request"),
            };
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    #[derive(Default)]
    struct RobotTool(AtomicUsize);

    #[async_trait]
    impl AgentToolInvoker for RobotTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            assert_eq!(invocation.call.name, "robot_action");
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(AgentToolResult::text(invocation.call.call_id,
                json!({"accepted":true,"completed":true}).to_string(), false))
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<AgentEngineEvent>>);

    #[async_trait]
    impl AgentEventSink for Events {
        async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn robot_full_history_runs_action_and_reply_without_a_compaction_model_request() {
        run_robot_action_scenario(false).await;
    }

    #[tokio::test]
    async fn robot_reply_style_short_acknowledgment_still_waits_for_every_requested_movement() {
        run_robot_action_scenario(true).await;
    }

    async fn run_robot_action_scenario(sequence: bool) {
        let engine = AgentEngine::new(AgentEngineBuild {
            build_id: "robot-budget-regression".into(), build_digest: "a".repeat(64).into(),
        }).unwrap().with_context_budget(context_budget()).unwrap();
        let mut request = nomifun_chat_model_broker::recorded_conformance_fixtures().remove(0).request;
        let binding = engine.bind(request.causality.agent_session_id.clone(), "robot-binding".into(),
            request.causality.resolved_snapshot_ref.clone()).unwrap();
        let original: Vec<_> = (0..4).flat_map(|i| exchange(&format!("history-{i}"))).collect();
        request.input = input();
        super::super::append_robot_reply_style(&mut request.input.instructions, true);
        request.input.messages = bounded_history(original.clone(), true).unwrap();
        assert_eq!(request.input.messages.len(), 16);
        let handoff = "large earlier cross-Agent handoff ".repeat(8192);
        assert!(handoff.len() > 16 * 1024);
        request.input.messages.push(text(ChatRole::Assistant, &bounded_handoff(handoff, true)));
        request.input.messages.push(text(ChatRole::User, if sequence { "Stand once, then turn." } else { "Stand once." }));
        let definition = ChatToolDefinition {
            name: "robot_action".into(), description: "Control the selected robot.".into(),
            input_schema: StrictJsonValue(json!({"type":"object","properties":{"action":{"type":"string"}},"required":["action"]})),
            deferred: false,
        };
        let plan = AgentToolPlan::new([AgentToolBinding {
            model_name: definition.name.clone(),
            schema_digest: nomifun_agent_contracts::digest_payload(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: "schema://robot/motion/input".into(),
            capability_contract_digest: "c".repeat(64).into(), definition,
            capability_id: "robot".into(), action_id: "robot/motion".into(),
            resource_binding_ids: Default::default(),
            effect_class: AgentEffectClass::ExternalUncertainEffect, parallel_safe: false,
        }]).unwrap();
        let model = Arc::new(RobotModel(Mutex::new(Vec::new()), sequence));
        let tool = Arc::new(RobotTool::default());
        let events = Arc::new(Events::default());
        let session = engine.open_session(binding, model.clone(), tool.clone(), Some(events.clone())).unwrap();
        // This in-memory observation sink intentionally cannot acknowledge a
        // durable checkpoint. These tests cover exact action/reply rounds;
        // production Host tests exercise the persisted segmented policy.
        let result = session.run_turn(AgentTurnRequest::new(request, plan,
            nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(), principal_id:"owner".into() }, 0)).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        let requests = model.0.lock().unwrap();
        assert_eq!(requests.len(), if sequence { 3 } else { 2 });
        assert_eq!(&requests[0].input.messages[..16], original.as_slice());
        assert_eq!(tool.0.load(Ordering::SeqCst), if sequence { 2 } else { 1 });
        for (step, call) in [(1, "current-action"), (2, "second-action")].into_iter().take(if sequence { 2 } else { 1 }) {
            assert!(requests[step].input.messages.iter().any(|message| message.content.iter().any(|part|
                matches!(part, ChatContentPart::ToolResult {call_id, is_error:false, ..} if call_id.as_ref()==call))),
                "each next step must observe the preceding real movement receipt");
        }
        assert!(!events.0.lock().unwrap().iter().any(|event|
            matches!(event, AgentEngineEvent::CompactionStarted { .. } | AgentEngineEvent::ContextCompacted { .. })));
    }
}

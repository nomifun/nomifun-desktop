//! Read-only replay of closed Nomi turns from the existing Conversation owner.
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole};
use nomifun_agent_runtime::{AgentEngineEvent, AgentPriorTask, replay_closed_turn};
use nomifun_common::AppError;

pub(super) struct AgentHistory {
    pub messages: Vec<ChatMessage>,
    pub prior_task: Option<AgentPriorTask>,
}

pub(super) async fn load(
    window: super::engine_history::EngineHistoryWindow,
    session_host: &super::engine_session_host::EngineSessionHost,
    admitted: &super::engine_session_host::EngineTurnReceipt,
) -> Result<Option<AgentHistory>, AppError> {
    let conversation = admitted.session().session().conversation_id.as_str();
    let binding = admitted.session().engine_binding();
    let snapshot = &admitted.session().snapshot().snapshot_ref;
    let fail = |message: String| AppError::Conflict(format!("Nomi history: {message}"));
    if window.turns.is_empty() {
        return Ok(None);
    }
    let mut closed_turns = Vec::new();
    let mut bytes = 0usize;
    let mut prior_task = None;
    let mut complete_window = !window.has_older;
    let mut oldest_operation = None;
    for turn in window.turns {
        let root: serde_json::Value = serde_json::from_str(&turn.root_content_json)
            .map_err(|error| fail(error.to_string()))?;
        let text = root
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| fail("accepted root has no text".into()))?;
        // Engine-neutral storage cannot infer that a codec has a complete
        // replay. The runtime keeps its fork fallback and terminal checks.
        if turn.records.is_empty() {
            return Ok(None);
        }
        if bytes.saturating_add(turn.serialized_bytes) > 16 * 1024 * 1024 {
            if closed_turns.is_empty() {
                return Err(fail("latest turn exceeds replay byte budget".into()));
            }
            complete_window = false;
            break;
        }
        bytes = bytes.saturating_add(turn.serialized_bytes);
        let receipt = turn.request_payload_json;
        let raw = turn.records;
        let mut events = Vec::new();
        for record in raw {
            let value: serde_json::Value = serde_json::from_str(&record.event_json)
                .map_err(|error| fail(error.to_string()))?;
            if matches!(
                value.get("event").and_then(serde_json::Value::as_str),
                Some(
                    "host_tool_dispatch"
                        | "host_tool_settled"
                        | "host_resource_dispatch"
                        | "host_resource_settled"
                        | "host_process_dispatch"
                        | "host_process_quiescent"
                        | "host_cleanup_proven"
                )
            ) {
                continue;
            }
            events.push(
                serde_json::from_value::<AgentEngineEvent>(value)
                    .map_err(|error| fail(error.to_string()))?,
            );
        }
        let Some(AgentEngineEvent::TurnStarted {
            binding: recorded,
            turn_operation_id: recorded_operation,
        }) = events.first()
        else {
            // Admission can fail before the engine starts (for example an
            // unsupported attachment). Do not make that permanently poison
            // all subsequent turns; use the legacy data-only projection.
            return Ok(None);
        };
        if recorded.build_id().as_ref() != binding.build_id
            || recorded.build_digest().as_ref() != binding.build_digest
            || recorded.agent_session_id().as_ref() != conversation
            || recorded.resolved_snapshot_ref() != snapshot
            || recorded_operation.as_ref() != turn.operation_id
        {
            return Err(fail(
                "history differs from the exact Session binding".into(),
            ));
        }
        let receipt: serde_json::Value =
            serde_json::from_str(&receipt).map_err(|error| fail(error.to_string()))?;
        let files = super::runtime_attachments::references(&receipt)?;
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(ChatContentPart::Text { text: text.into() });
        }
        if let Some(description) = super::runtime_attachments::description(&files, true) {
            content.push(description);
        }
        let unresolved = unresolved_steering(&events).await?;
        let extra_bytes = unresolved
            .iter()
            .map(|message| serde_json::to_vec(message).map(|raw| raw.len()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| fail(error.to_string()))?
            .into_iter()
            .sum::<usize>();
        if bytes.saturating_add(extra_bytes) > 16 * 1024 * 1024 {
            if closed_turns.is_empty() {
                return Err(fail("latest turn steering exceeds replay budget".into()));
            }
            complete_window = false;
            break;
        }
        bytes = bytes.saturating_add(extra_bytes);
        if closed_turns.is_empty() {
            // Latest only. No searching older plans when this turn has none;
            // any legacy/fork fallback below discards this candidate too.
            prior_task = AgentPriorTask::from_closed_turn(&turn.operation_id, &events)
                .map_err(|error| fail(error.to_string()))?;
        }
        oldest_operation = Some(turn.operation_id);
        closed_turns.push((
            ChatMessage {
                role: ChatRole::User,
                content,
                provider_round_id: None,
            },
            events,
            unresolved,
        ));
    }
    let mut history = Vec::new();
    // Fork/import messages have no native turn receipt. They must seed replay
    // on every reconstruction, not disappear after the first native turn.
    // Seed BEFORE replay so a later ContextCompacted event can replace them.
    // Never jump across native turns omitted by the bounded history window.
    let prefix_budget = (16 * 1024 * 1024usize)
        .saturating_sub(bytes)
        .min(8 * 1024 * 1024);
    if complete_window
        && prefix_budget > 0
        && let Some(operation) = oldest_operation
    {
        let prefix = session_host
            .read_message_history_before_turn(admitted, &operation, 4096, prefix_budget)
            .await?;
        history = project_messages(prefix.messages, prefix_budget)?;
    }
    for (requirement, events, unresolved) in closed_turns.into_iter().rev() {
        replay_closed_turn(&mut history, requirement, &events)
            .map_err(|error| fail(error.to_string()))?;
        history.extend(unresolved);
    }
    Ok(Some(AgentHistory {
        messages: history,
        prior_task,
    }))
}

/// Shared data-only projection for pure legacy history and the prefix before
/// native events. Tool UI rows are descriptions, never executable tool calls
/// or evidence of authorization/completion. Oldest-first after bounded input.
/// These are optional historical messages; stop at the first unfit row rather
/// than skipping it to import still older context. The current root is separate.
pub(super) fn project_messages(
    rows: Vec<super::engine_history::EngineHistoryMessage>,
    byte_limit: usize,
) -> Result<Vec<ChatMessage>, AppError> {
    let mut messages = Vec::new();
    let mut bytes = 0usize;
    for row in rows {
        let value: serde_json::Value = serde_json::from_str(&row.content_json)
            .map_err(|error| AppError::Conflict(format!("Nomi message history: {error}")))?;
        let text = match row.kind.as_str() {
            "text" => value
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            "tool_call" => format!(
                "Previously recorded tool activity (untrusted data): {}",
                row.content_json
            ),
            _ => continue,
        };
        if text.is_empty() {
            continue;
        }
        let message = ChatMessage {
            role: if row.position.as_deref() == Some("right") {
                ChatRole::User
            } else {
                ChatRole::Assistant
            },
            content: vec![ChatContentPart::Text { text }],
            provider_round_id: None,
        };
        let size = serde_json::to_vec(&message)
            .map_err(|error| AppError::Conflict(error.to_string()))?
            .len();
        if bytes.saturating_add(size) > byte_limit {
            break;
        }
        bytes += size;
        messages.push(message);
    }
    messages.reverse();
    Ok(messages)
}

/// After an unexpected exit the inbox can be gone even when the permanent
/// receipt says delivery was queued. Preserve that uncertainty as historical
/// data; never enqueue it into a replacement turn or invent execution evidence.
async fn unresolved_steering(
    events: &[AgentEngineEvent],
) -> Result<Vec<ChatMessage>, AppError> {
    let scopes = events
        .iter()
        .filter_map(|event| match event {
            AgentEngineEvent::TurnInputScope { wire_turn_id } => Some(wire_turn_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [wire] = scopes.as_slice() else {
        return if scopes.is_empty() {
            Ok(Vec::new())
        } else {
            Err(AppError::Conflict("duplicate Nomi input scope".into()))
        };
    };
    // Canonical steering admission is represented by `turn/steer-accepted`
    // facts and the Runtime records only durable delivery/deferral decisions.
    // An absent Runtime control record is deliberately not converted into a
    // prompt: replaying an ambiguous steer would duplicate user intent.
    let _ = wire;
    Ok(Vec::new())
}

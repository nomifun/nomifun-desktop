//! Rebuild model history from durable engine events, never execute their tools.
//! This is normal closed-turn replay, NOT proof of crash-time process cleanup.
use crate::{CodingEngineError, CodingEngineEvent, CodingToolResult};
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole, ChatToolCall, ToolCallId};
use std::collections::BTreeMap;

pub fn replay_closed_turn(
    history: &mut Vec<ChatMessage>,
    requirement: ChatMessage,
    events: &[CodingEngineEvent],
) -> Result<(), CodingEngineError> {
    // Do not leave a partially appended or compacted projection with callers
    // when a later event contradicts the journal. This is context only: no
    // owner resource or durable history is changed by either branch.
    let mut candidate = history.clone();
    replay_into(&mut candidate, requirement, events, false)?;
    *history = candidate;
    Ok(())
}

fn replay_into(
    history: &mut Vec<ChatMessage>,
    requirement: ChatMessage,
    events: &[CodingEngineEvent],
    isolated_archive: bool,
) -> Result<(), CodingEngineError> {
    let (terminal_steps, interrupted) = match events.last() {
        Some(CodingEngineEvent::TurnCompleted { model_steps, .. }) => (*model_steps, false),
        Some(
            CodingEngineEvent::TurnCancelled { model_steps }
            | CodingEngineEvent::TurnFailed { model_steps, .. },
        ) => (*model_steps, true),
        _ => {
            return Err(invalid(
                "turn has no durable terminal; replay cannot prove recovery",
            ));
        }
    };
    if !matches!(events.first(), Some(CodingEngineEvent::TurnStarted { .. }))
        || events
            .iter()
            .skip(1)
            .any(|event| matches!(event, CodingEngineEvent::TurnStarted { .. }))
        || events[..events.len() - 1].iter().any(|event| {
            matches!(
                event,
                CodingEngineEvent::TurnCompleted { .. }
                    | CodingEngineEvent::TurnCancelled { .. }
                    | CodingEngineEvent::TurnFailed { .. }
            )
        })
    {
        return Err(CodingEngineError::ReplayContract(
            "replay requires exactly one turn start and one final terminal".into(),
        ));
    }
    history.push(requirement.clone());
    let mut retained_inputs = vec![requirement.clone()];
    let mut steering_receipts = std::collections::BTreeSet::new();
    let mut seen_call_ids = std::collections::BTreeSet::new();
    let mut model_steps = 0u16;
    let mut batch = ReplayBatch::default();
    for event in events {
        batch.validate_event_step(event)?;
        match event {
            CodingEngineEvent::SteeringInputs { inputs }
            | CodingEngineEvent::SteeringDeferred { inputs, .. } => {
                for input in inputs {
                    input.validate()?;
                    if steering_receipts.len() >= 16
                        || !steering_receipts.insert(input.receipt_operation_id.clone())
                    {
                        return Err(CodingEngineError::ReplayContract(
                            "duplicate or excessive steering receipts in history".into(),
                        ));
                    }
                    let message = input.message();
                    retained_inputs.push(message.clone());
                    batch.notices.push(message);
                }
                if let CodingEngineEvent::SteeringDeferred { reason, .. } = event {
                    batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                        format!("Delivery observation (not a new instruction): the preceding queued inputs did not reach a model boundary before this turn ended: {reason}")));
                }
            }
            CodingEngineEvent::ModelStepStarted { step, .. } => {
                if model_steps.checked_add(1) != Some(*step) {
                    return Err(invalid("non-contiguous model steps in history"));
                }
                batch.flush(history, false)?;
                model_steps = *step;
                batch.step = Some(*step);
            }
            CodingEngineEvent::ToolStarted {
                step,
                call_id,
                action_id,
                ..
            } if *step > 0 => {
                if batch.discarded {
                    return Err(CodingEngineError::ReplayContract(
                        "tool dispatch after output-limit discard".into(),
                    ));
                }
                if !batch.calls.contains_key(call_id) || !batch.started.insert(call_id.clone()) {
                    return Err(invalid(
                        "tool admission has no complete call or repeats an admission",
                    ));
                }
                if let Some(kind) =
                    crate::tool_context::ToolContextKind::for_action(action_id.as_ref())
                {
                    batch.context_kinds.insert(call_id.clone(), kind);
                }
            }
            CodingEngineEvent::ToolCallDelta {
                step,
                call_id,
                name,
                ..
            } if *step > 0 => {
                if batch.discarded {
                    return Err(CodingEngineError::ReplayContract(
                        "tool delta after output-limit discard".into(),
                    ));
                }
                crate::stream_limits::identity(call_id, name)
                    .map_err(|error| invalid(&error.to_string()))?;
                if !batch.proposed.contains(call_id) && !seen_call_ids.insert(call_id.clone()) {
                    return Err(invalid(
                        "model reused a prior tool-call identity in history",
                    ));
                }
                if batch.proposed.insert(call_id.clone()) {
                    batch.proposal_order.push(call_id.clone());
                }
                if batch.proposed.len() > 64 {
                    return Err(CodingEngineError::ReplayContract(
                        "excessive tool proposals in history".into(),
                    ));
                }
            }
            CodingEngineEvent::ModelOutputTruncated {
                step,
                discarded_tool_call_ids,
                continuation,
            } => {
                crate::output_limit::validate_discarded(*step, discarded_tool_call_ids)?;
                if batch.step != Some(*step)
                    || !batch.started.is_empty()
                    || !batch.results.is_empty()
                    || batch.proposed
                        != discarded_tool_call_ids
                            .iter()
                            .cloned()
                            .collect::<std::collections::BTreeSet<_>>()
                    || batch.discarded
                {
                    return Err(CodingEngineError::ReplayContract(
                        "output-limit discard contradicts tool history".into(),
                    ));
                }
                batch.discarded = true;
                batch
                    .content
                    .retain(|part| matches!(part, ChatContentPart::Text { .. }));
                batch.calls.clear();
                batch.order.clear();
                batch
                    .notices
                    .push(crate::output_limit::notice(*continuation));
            }
            CodingEngineEvent::OutputTextDelta { text, .. } => {
                if batch.discarded {
                    return Err(CodingEngineError::ReplayContract(
                        "model text after output-limit discard".into(),
                    ));
                }
                if let Some(ChatContentPart::Text { text: previous }) = batch.content.last_mut() {
                    previous.push_str(text);
                } else {
                    batch
                        .content
                        .push(ChatContentPart::Text { text: text.clone() });
                }
            }
            CodingEngineEvent::ToolCallCompleted { step, call } if *step > 0 => {
                if batch.discarded {
                    return Err(CodingEngineError::ReplayContract(
                        "tool call after output-limit discard".into(),
                    ));
                }
                crate::stream_limits::completed(call)
                    .map_err(|error| invalid(&error.to_string()))?;
                if !batch.proposed.contains(&call.call_id)
                    && !seen_call_ids.insert(call.call_id.clone())
                {
                    return Err(invalid(
                        "model reused a prior tool-call identity in history",
                    ));
                }
                if batch.proposed.insert(call.call_id.clone()) {
                    batch.proposal_order.push(call.call_id.clone());
                }
                if batch.proposed.len() > 64 {
                    return Err(invalid("excessive completed tool proposals in history"));
                }
                if batch
                    .calls
                    .insert(call.call_id.clone(), call.clone())
                    .is_some()
                {
                    return Err(CodingEngineError::ReplayContract(
                        "duplicate persisted tool call".into(),
                    ));
                }
                batch.order.push(call.call_id.clone());
                batch.content.push(ChatContentPart::ToolCall {
                    call_id: call.call_id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                    provider_metadata: call.provider_metadata.clone(),
                });
            }
            CodingEngineEvent::ToolCompleted { step, result } if *step > 0 => {
                result.validate_for(&result.call_id)?;
                if !batch.calls.contains_key(&result.call_id) {
                    return Err(CodingEngineError::ReplayContract(
                        "persisted tool result has no call".into(),
                    ));
                }
                if batch
                    .results
                    .insert(result.call_id.clone(), result.clone())
                    .is_some()
                {
                    return Err(invalid("duplicate persisted tool result"));
                }
                batch.result_order.push(result.call_id.clone());
            }
            CodingEngineEvent::ToolResultsOrdered { step, call_ids } => {
                if *step == 0
                    || batch.step != Some(*step)
                    || batch.discarded
                    || batch.model_order.is_some()
                    || call_ids.is_empty()
                    || call_ids.len() > 64
                    || call_ids != &batch.proposal_order
                    || call_ids.len() != batch.calls.len()
                    || call_ids.len() != batch.results.len()
                    || call_ids.iter().any(|id| !batch.results.contains_key(id))
                {
                    return Err(invalid(
                        "model result ordering does not match one complete proposal batch",
                    ));
                }
                batch.model_order = Some(call_ids.clone());
            }
            CodingEngineEvent::ContextCompacted {
                summary,
                retained_tool_call_ids,
                retained_context,
                ..
            } => {
                batch.flush(history, false)?;
                // Only a self-contained replacement may bridge a missing
                // prefix of the bounded production history window.
                {
                    let unique = retained_tool_call_ids
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>();
                    if unique.len() != retained_tool_call_ids.len()
                        || unique.len() > 64
                        || unique.iter().any(|id| {
                            id.as_ref().is_empty()
                                || id.as_ref().len() > 256
                                || id.as_ref().chars().any(char::is_control)
                        })
                    {
                        return Err(invalid("invalid compaction references"));
                    }
                }
                let mut local_ids = retained_tool_call_ids.as_slice();
                if isolated_archive {
                    // A multi-batch suffix can span a previous turn even after
                    // this turn has started. Unknown IDs may only be a prefix;
                    // they are not imported into this turn's archive/evidence.
                    let absent = local_ids
                        .iter()
                        .take_while(|id| !seen_call_ids.contains(*id))
                        .count();
                    local_ids = &local_ids[absent..];
                    if local_ids.iter().any(|id| !seen_call_ids.contains(id)) {
                        return Err(invalid(
                            "absent compaction IDs follow current-turn references",
                        ));
                    }
                    if absent > 0 && history.iter().flat_map(|message| &message.content).any(|part| {
                        matches!(part, ChatContentPart::ToolCall { call_id, .. } if !local_ids.contains(call_id))
                    }) {
                        return Err(invalid("absent compaction prefix would skip an available local batch"));
                    }
                }
                let retained = if let Some(items) =
                    retained_context.as_ref().filter(|_| !isolated_archive)
                {
                    crate::compacted_history::restore(
                        items,
                        &retained_inputs,
                        retained_tool_call_ids,
                    )?
                } else if local_ids.is_empty() {
                    retained_inputs.clone()
                } else {
                    let exchange = crate::context_tail::selected(history, local_ids)
                        .map_err(|error| CodingEngineError::ReplayContract(error.to_string()))?
                        .ok_or_else(|| {
                            CodingEngineError::ReplayContract(
                                "Compaction references do not match a bounded contiguous tool suffix".into(),
                            )
                        })?;
                    exchange
                        .with_required_inputs(&retained_inputs)
                        .map_err(|error| CodingEngineError::ReplayContract(error.to_string()))?
                };
                // The summary covers the whole model view, including previous
                // turns. Do not prepend that same old history a second time.
                history.clear();
                history.push(crate::context_lifecycle::summary_message(summary));
                // Journal projections may contain bounded excerpts or image
                // descriptors. Retention never rehydrates original payloads
                // or turns historical observations into new execution proof.
                history.extend(retained);
            }
            CodingEngineEvent::CompletionReview { status } => {
                batch.flush(history, false)?;
                history.push(status.completion_review_message()?);
            }
            CodingEngineEvent::PlanUpdated { plan } => {
                batch.notices.push(crate::context_lifecycle::text_message(
                    ChatRole::User,
                    plan.context()?,
                ));
            }
            CodingEngineEvent::CompletionReported { report } => {
                batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                    format!("Historical completion account (model assessment with observation references, not fresh proof for this turn): {}",
                        serde_json::to_string(report).map_err(|error| CodingEngineError::ReplayContract(error.to_string()))?)));
            }
            CodingEngineEvent::InstructionsUpdated { context } => {
                batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                    format!("Previously observed repository instructions (historical data; current rules must be re-read):\n{context}")));
            }
            _ => {}
        }
    }
    // The shared lifecycle can interrupt/drop a driver or catch a panic and
    // emit Cancelled/Failed with zero steps (unknown to that layer). Do not
    // mistake that sentinel for proof that no model step ran. Successful
    // outcomes and nonzero interrupted counts must match the journal exactly.
    if model_steps != terminal_steps && !(interrupted && terminal_steps == 0) {
        return Err(invalid(
            "terminal model-step count differs from the recorded steps",
        ));
    }
    // Only the final batch of an interrupted turn may lack results. Earlier
    // batches had to settle before a model step, review or compaction began.
    batch.flush(history, interrupted)?;
    Ok(())
}

/// Validate this turn's batches without pretending to reconstruct its absent
/// older model prefix. Normal replay continues to require that prefix.
pub(crate) fn validate_archive_turn(
    requirement: ChatMessage,
    events: &[CodingEngineEvent],
) -> Result<(), CodingEngineError> {
    replay_into(&mut Vec::new(), requirement, events, true)
}

#[derive(Default)]
struct ReplayBatch {
    proposed: std::collections::BTreeSet<ToolCallId>,
    proposal_order: Vec<ToolCallId>,
    model_order: Option<Vec<ToolCallId>>,
    step: Option<u16>,
    started: std::collections::BTreeSet<ToolCallId>,
    discarded: bool,
    notices: Vec<ChatMessage>,
    content: Vec<ChatContentPart>,
    calls: BTreeMap<ToolCallId, ChatToolCall>,
    order: Vec<ToolCallId>,
    result_order: Vec<ToolCallId>,
    results: BTreeMap<ToolCallId, CodingToolResult>,
    context_kinds: BTreeMap<ToolCallId, crate::tool_context::ToolContextKind>,
}

impl ReplayBatch {
    fn validate_event_step(&self, event: &CodingEngineEvent) -> Result<(), CodingEngineError> {
        let tool = match event {
            CodingEngineEvent::ToolCallDelta { step, call_id, .. }
            | CodingEngineEvent::ToolStarted { step, call_id, .. } => Some((*step, call_id)),
            CodingEngineEvent::ToolCallCompleted { step, call } => Some((*step, &call.call_id)),
            CodingEngineEvent::ToolCompleted { step, result } => Some((*step, &result.call_id)),
            _ => None,
        };
        if let Some((step, call_id)) = tool {
            if step == 0 {
                // Internal instruction reads are deliberately excluded from
                // model history; arbitrary model calls cannot use this lane.
                return if call_id.as_ref().starts_with("coding-instructions:") {
                    Ok(())
                } else {
                    Err(invalid(
                        "step-zero tool history is not an internal instruction read",
                    ))
                };
            }
            if self.step != Some(step) {
                return Err(invalid(
                    "tool event differs from the active replay model step",
                ));
            }
            if self.model_order.is_some() {
                return Err(invalid(
                    "tool event follows finalized model result ordering",
                ));
            }
        }
        if let CodingEngineEvent::OutputTextDelta { step, .. }
        | CodingEngineEvent::ReasoningDelta { step, .. }
        | CodingEngineEvent::Usage { step, .. } = event
            && (*step == 0 || self.step != Some(*step))
        {
            return Err(invalid(
                "model event differs from the active replay model step",
            ));
        }
        if matches!(
            event,
            CodingEngineEvent::ToolCallDelta { .. }
                | CodingEngineEvent::ToolCallCompleted { .. }
                | CodingEngineEvent::OutputTextDelta { .. }
                | CodingEngineEvent::ReasoningDelta { .. }
                | CodingEngineEvent::Usage { .. }
        ) && (!self.started.is_empty() || !self.results.is_empty())
        {
            return Err(invalid(
                "model stream event follows tool admission or results in the same step",
            ));
        }
        Ok(())
    }

    fn flush(
        &mut self,
        history: &mut Vec<ChatMessage>,
        allow_incomplete: bool,
    ) -> Result<(), CodingEngineError> {
        if !allow_incomplete
            && !self.discarded
            && (self.proposed.iter().any(|id| !self.calls.contains_key(id))
                || self.results.len() != self.calls.len())
        {
            return Err(invalid(
                "unfinished tool batch before a continuation boundary or successful terminal",
            ));
        }
        if !self.content.is_empty() {
            history.push(ChatMessage {
                role: ChatRole::Assistant,
                content: std::mem::take(&mut self.content),
                provider_round_id: None,
            });
        }
        // Completed parallel batches explicitly preserve the model projection
        // order independently of result arrival. Legacy/unfinalized batches
        // retain their recorded result order, not declaration-completion order.
        // Neither constitutes an effect-order or model-delivery proof.
        let mut result_order = self
            .model_order
            .take()
            .unwrap_or_else(|| std::mem::take(&mut self.result_order));
        self.result_order.clear();
        for call_id in self.order.drain(..) {
            if !self.results.contains_key(&call_id) {
                result_order.push(call_id);
            }
        }
        for call_id in result_order {
            // An interrupted model turn can close before delivering a tool
            // result. Never infer rollback or retry the effect during replay.
            let result = self.results.remove(&call_id).unwrap_or_else(|| CodingToolResult::text(
                call_id.clone(), "No model-visible result was recorded before this turn ended. The effect may have occurred; inspect actual state before retrying.", true,
            ));
            // The durable observation can already be bounded by the host.
            // Apply the same model policy only to available admitted output;
            // never reconstruct omitted text or change original event data.
            let result = match self.context_kinds.get(&call_id) {
                Some(kind) => crate::tool_context::project(*kind, &result),
                None => result,
            };
            history.push(ChatMessage {
                role: ChatRole::Tool,
                provider_round_id: None,
                content: vec![ChatContentPart::ToolResult {
                    call_id,
                    output: result.output,
                    is_error: result.is_error,
                }],
            });
        }
        self.calls.clear();
        self.proposed.clear();
        self.proposal_order.clear();
        self.results.clear();
        self.context_kinds.clear();
        history.append(&mut self.notices);
        self.step = None;
        self.started.clear();
        self.discarded = false;
        Ok(())
    }
}

fn invalid(message: &str) -> CodingEngineError {
    CodingEngineError::ReplayContract(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodingEngine, CodingEngineBuild, EngineBuildId, EngineBinding};
    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, DigestHex, OperationId, ResolvedSnapshotId,
        ResolvedSnapshotRef, RuntimeBindingId, StrictJsonValue,
    };
    use nomifun_chat_model_broker::{ChatContentPart, ChatRole, ChatToolCall};

    fn binding() -> EngineBinding {
        CodingEngine::new(CodingEngineBuild {
            build_id: EngineBuildId::from("build"),
            build_digest: DigestHex::from("a".repeat(64)),
        })
        .unwrap()
        .bind(
            AgentSessionId::from("session"),
            RuntimeBindingId::from("binding"),
            ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("snapshot"),
                snapshot_digest: DigestHex::from("b".repeat(64)),
            },
        )
        .unwrap()
    }

    fn requirement() -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text {
                text: "change the file".into(),
            }],
            provider_round_id: None,
        }
    }

    #[test]
    fn restart_history_without_a_durable_terminal_is_never_replayed() {
        let mut history = Vec::new();
        let error = replay_closed_turn(
            &mut history,
            requirement(),
            &[CodingEngineEvent::TurnStarted {
                binding: binding(),
                turn_operation_id: OperationId::from("turn"),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, CodingEngineError::ReplayContract(message)
            if message.contains("no durable terminal")));
        assert!(history.is_empty());
    }

    #[test]
    fn interrupted_effect_without_a_result_becomes_unknown_and_is_not_replayed() {
        let call_id = ToolCallId::from("effect-1");
        let events = vec![
            CodingEngineEvent::TurnStarted {
                binding: binding(),
                turn_operation_id: OperationId::from("turn"),
            },
            CodingEngineEvent::ModelStepStarted {
                step: 1,
                operation_id: OperationId::from("turn:model:1"),
            },
            CodingEngineEvent::ToolCallCompleted {
                step: 1,
                call: ChatToolCall {
                    call_id: call_id.clone(),
                    name: "write_file".into(),
                    arguments: StrictJsonValue(serde_json::json!({"path":"a","content":"b"})),
                    provider_metadata: None,
                },
            },
            CodingEngineEvent::ToolStarted {
                step: 1,
                call_id: call_id.clone(),
                capability_id: CapabilityId::from("workspace.files"),
                action_id: ActionId::from("workspace.files/write"),
            },
            CodingEngineEvent::TurnFailed {
                model_steps: 1,
                message: "application restarted".into(),
            },
        ];
        let mut history = Vec::new();
        replay_closed_turn(&mut history, requirement(), &events).unwrap();
        let result = history
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|part| match part {
                ChatContentPart::ToolResult {
                    call_id: observed,
                    output,
                    is_error,
                } if observed == &call_id => Some((output, is_error)),
                _ => None,
            })
            .expect("unknown effect observation");
        assert!(*result.1);
        assert!(result.0.iter().any(|part| matches!(part,
            nomifun_chat_model_broker::ChatToolResultPart::Text { text }
                if text.contains("may have occurred") && text.contains("before retrying"))));
        assert!(!events.iter().any(|event| matches!(event, CodingEngineEvent::ToolCompleted { .. })));
    }
}

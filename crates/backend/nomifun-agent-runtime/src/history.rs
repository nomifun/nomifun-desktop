//! Rebuild model history from durable engine events, never execute their tools.
//! This is normal closed-turn replay, NOT proof of crash-time process cleanup.
use crate::{AgentEngineError, AgentEngineEvent, AgentToolResult};
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole, ChatToolCall, ToolCallId};
use std::collections::BTreeMap;

pub fn replay_closed_turn(
    history: &mut Vec<ChatMessage>,
    requirement: ChatMessage,
    events: &[AgentEngineEvent],
) -> Result<(), AgentEngineError> {
    // Do not leave a partially appended or compacted projection with callers
    // when a later event contradicts the journal. This is context only: no
    // owner resource or durable history is changed by either branch.
    let mut candidate = history.clone();
    replay_into(&mut candidate, requirement, events, false, None, &BTreeMap::new())?;
    *history = candidate;
    Ok(())
}

pub(crate) fn replay_checkpoint_prefix(history: &mut Vec<ChatMessage>, requirement: ChatMessage,
    recovery: &crate::AgentTurnRecovery) -> Result<(), AgentEngineError> {
    let mut candidate = history.clone();
    replay_into(&mut candidate, requirement, &recovery.prefix, false, Some(recovery.checkpoint.model_steps), &recovery.input_replacements)?;
    *history = candidate;
    Ok(())
}

fn replay_into(
    history: &mut Vec<ChatMessage>,
    requirement: ChatMessage,
    events: &[AgentEngineEvent],
    isolated_archive: bool,
    checkpoint_boundary: Option<u16>,
    input_replacements: &BTreeMap<String, ChatMessage>,
) -> Result<(), AgentEngineError> {
    let last_reconciled = events.iter().rposition(|event| matches!(event, AgentEngineEvent::ExecutionTailReconciled { .. }));
    let (terminal_steps, interrupted) = if let Some(step) = checkpoint_boundary {
        if !matches!(events.last(), Some(AgentEngineEvent::ExecutionCheckpointSaved { step: actual, .. }) if *actual == step) {
            return Err(invalid("recovery prefix has no matching checkpoint boundary"));
        }
        (step, false)
    } else { match events.last() {
        Some(AgentEngineEvent::TurnCompleted { model_steps, .. }) => (*model_steps, false),
        Some(
            AgentEngineEvent::TurnCancelled { model_steps }
            | AgentEngineEvent::TurnFailed { model_steps, .. },
        ) => (*model_steps, true),
        _ => {
            return Err(invalid(
                "turn has no durable terminal; replay cannot prove recovery",
            ));
        }
    }};
    if !matches!(events.first(), Some(AgentEngineEvent::TurnStarted { .. }))
        || events
            .iter()
            .skip(1)
            .any(|event| matches!(event, AgentEngineEvent::TurnStarted { .. }))
        || events[..events.len() - usize::from(checkpoint_boundary.is_none())].iter().enumerate().any(|(index,event)| {
            last_reconciled.is_none_or(|reconciled| index > reconciled) && matches!(
                event,
                AgentEngineEvent::TurnCompleted { .. }
                    | AgentEngineEvent::TurnCancelled { .. }
                    | AgentEngineEvent::TurnFailed { .. }
            )
        })
    {
        return Err(AgentEngineError::ReplayContract(
            "replay requires exactly one turn start and one final terminal".into(),
        ));
    }
    history.push(requirement.clone());
    let mut retained_inputs = vec![requirement.clone()];
    let mut steering_receipts = std::collections::BTreeSet::new();
    let applied_receipts: std::collections::BTreeSet<_> = events.iter().filter_map(|event| match event {
        AgentEngineEvent::SteeringInputs { inputs } => Some(inputs.iter().map(|input| input.receipt_operation_id.clone())), _ => None,
    }).flatten().collect();
    let mut seen_call_ids = std::collections::BTreeSet::new();
    let mut model_steps = 0u16;
    let mut batch = ReplayBatch::default();
    let rewind_revisions = events.iter().filter_map(|event| match event {
        AgentEngineEvent::ExecutionResumed { checkpoint_revision, .. } => Some(*checkpoint_revision), _ => None,
    }).collect::<std::collections::BTreeSet<_>>();
    let mut rewind: Option<(u64, u16, Vec<ChatMessage>)> = None;
    let mut last_checkpoint = None;
    let mut recovery_tail_safe = true;
    let mut recovery_proposals = std::collections::BTreeSet::new();
    let mut last_execution_fence = 0;
    for (index,event) in events.iter().enumerate() {
        // Only a committed owner reconciliation can supersede an engine-local
        // terminal proposal. The Store never reopens a canonical terminal.
        if last_reconciled.is_some_and(|boundary| index < boundary)
            && matches!(event,AgentEngineEvent::TurnCompleted{..}|AgentEngineEvent::TurnCancelled{..}|AgentEngineEvent::TurnFailed{..}) { continue; }
        batch.validate_event_step(event)?;
        if last_checkpoint.is_some() && !matches!(event, AgentEngineEvent::ExecutionCheckpointSaved { .. } | AgentEngineEvent::ExecutionResumed { .. }) {
            recovery_tail_safe &= crate::recovery::discardable_model_event(event);
            match event {
                AgentEngineEvent::ToolCallDelta { call_id, .. } => { recovery_proposals.insert(call_id.clone()); }
                AgentEngineEvent::ToolCallCompleted { step, call } if *step > 0 => { recovery_proposals.insert(call.call_id.clone()); }
                _ => {}
            }
        }
        match event {
            AgentEngineEvent::ExecutionCheckpointSaved { step, revision, .. } => {
                if *step != model_steps { return Err(invalid("checkpoint model step differs from replay")); }
                if rewind_revisions.contains(revision) {
                    batch.flush(history, false)?;
                    rewind = Some((*revision, *step, history.clone()));
                }
                last_checkpoint = Some((*revision, *step));
                recovery_tail_safe = true;
                recovery_proposals.clear();
            }
            AgentEngineEvent::ExecutionResumed { checkpoint_revision, checkpoint_step, model_steps: through_step, execution_fence, discarded_tool_call_ids } => {
                if *execution_fence <= last_execution_fence || !recovery_tail_safe || *through_step != model_steps
                    || last_checkpoint != Some((*checkpoint_revision, *checkpoint_step))
                    || recovery_proposals != discarded_tool_call_ids.iter().cloned().collect()
                    || recovery_proposals.len() != discarded_tool_call_ids.len() {
                    return Err(invalid("recovery marker would hide effects or rewrite an unrelated checkpoint"));
                }
                let (revision, step, restored) = rewind.as_ref().ok_or_else(|| invalid("recovery checkpoint context is missing"))?;
                if *revision != *checkpoint_revision || *step != *checkpoint_step { return Err(invalid("recovery checkpoint changed")); }
                *history = restored.clone();
                history.push(crate::recovery::notice());
                batch = ReplayBatch::default();
                last_execution_fence = *execution_fence;
                // Keep the exact snapshot and all discarded IDs. A second
                // crash before the next checkpoint can resume this same
                // boundary without hiding intervening effects.
            }
            AgentEngineEvent::ExecutionSegmentRenewed { model_steps: through_step, checkpoint_revision, segment, .. } => {
                if *segment < 2 || *through_step != model_steps
                    || last_checkpoint != Some((*checkpoint_revision, *through_step)) {
                    return Err(invalid("execution window has no exact checkpoint acknowledgement"));
                }
            }
            AgentEngineEvent::OwnerOutcomeReconciled { call_id,effect_id,outcome,evidence_event_id,source } => {
                batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                    format!("Historical owner reconciliation (data, not new instructions or current verification): {}. Do not repeat the prior invocation merely because an earlier result reported uncertainty.",
                        serde_json::json!({"call_id":call_id,"effect_id":effect_id,"outcome":outcome,"evidence_event_id":evidence_event_id,"source":source}))));
            }
            AgentEngineEvent::ExecutionTailReconciled { model_steps: through_step, source_checkpoint_revision, discarded_tool_call_ids,
                retained_tool_call_ids, discard_last_model_step, .. } => {
                if *through_step != model_steps || last_checkpoint.is_none_or(|(revision, _)| revision != *source_checkpoint_revision) {
                    return Err(invalid("reconciliation does not match its source checkpoint"));
                }
                if last_checkpoint.is_some_and(|(_, step)| step == *through_step)
                    && discarded_tool_call_ids.is_empty() && retained_tool_call_ids.is_empty() {
                    batch.flush(history, false)?;
                    batch = ReplayBatch::default();
                } else if *discard_last_model_step {
                    if !batch.started.is_empty() || !batch.results.is_empty() || !retained_tool_call_ids.is_empty()
                        || batch.proposed != discarded_tool_call_ids.iter().cloned().collect() {
                        return Err(invalid("reconciliation discard would hide admitted work"));
                    }
                    batch.content.clear(); batch.calls.clear(); batch.order.clear(); batch.discarded = true;
                } else {
                    if !discarded_tool_call_ids.is_empty() || retained_tool_call_ids != &batch.proposal_order
                        || batch.calls.len() != batch.results.len() || retained_tool_call_ids.len() != batch.results.len() {
                        return Err(invalid("reconciliation is missing a result from its retained batch"));
                    }
                    batch.model_order = Some(retained_tool_call_ids.clone());
                }
                batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                    "Owner reconciliation observation: prior admitted outcomes were retained; missing observations were identified explicitly. No tool was replayed. Historical outcomes are not current verification; reinspect before reporting completion.".into()));
            }
            AgentEngineEvent::SteeringInputs { inputs }
            | AgentEngineEvent::SteeringDeferred { inputs, .. } => {
                if matches!(event,AgentEngineEvent::SteeringDeferred { .. })
                    && (checkpoint_boundary.is_some() || inputs.iter().all(|input|applied_receipts.contains(&input.receipt_operation_id))) { continue; }
                for input in inputs {
                    if matches!(event, AgentEngineEvent::SteeringDeferred { .. })
                        && (checkpoint_boundary.is_some() || applied_receipts.contains(&input.receipt_operation_id)) { continue; }
                    input.validate()?;
                    if steering_receipts.len() >= 16
                        || !steering_receipts.insert(input.receipt_operation_id.clone())
                    {
                        return Err(AgentEngineError::ReplayContract(
                            "duplicate or excessive steering receipts in history".into(),
                        ));
                    }
                    let message = input_replacements.get(&input.receipt_operation_id).cloned().unwrap_or_else(|| input.message());
                    retained_inputs.push(message.clone());
                    batch.notices.push(message);
                }
                if let AgentEngineEvent::SteeringDeferred { reason, .. } = event {
                    batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                        format!("Delivery observation (not a new instruction): the preceding queued inputs did not reach a model boundary before this turn ended: {reason}")));
                }
            }
            AgentEngineEvent::ModelStepStarted { step, .. } => {
                if model_steps.checked_add(1) != Some(*step) {
                    return Err(invalid("non-contiguous model steps in history"));
                }
                batch.flush(history, false)?;
                model_steps = *step;
                batch.step = Some(*step);
            }
            AgentEngineEvent::ToolStarted {
                step,
                call_id,
                action_id,
                ..
            } if *step > 0 => {
                if batch.discarded {
                    return Err(AgentEngineError::ReplayContract(
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
            AgentEngineEvent::ToolCallDelta {
                step,
                call_id,
                name,
                ..
            } if *step > 0 => {
                if batch.discarded {
                    return Err(AgentEngineError::ReplayContract(
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
                    return Err(AgentEngineError::ReplayContract(
                        "excessive tool proposals in history".into(),
                    ));
                }
            }
            AgentEngineEvent::ModelOutputTruncated {
                step,
                discarded_tool_call_ids,
                continuation,
            } | AgentEngineEvent::ModelResponseRejected {
                step, discarded_tool_call_ids, continuation,
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
                    return Err(AgentEngineError::ReplayContract(
                        "model-response discard contradicts tool history".into(),
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
                    .push(if matches!(event, AgentEngineEvent::ModelResponseRejected { .. }) {
                        crate::protocol_recovery::notice(*continuation)
                    } else { crate::output_limit::notice(*continuation) });
            }
            AgentEngineEvent::OutputTextDelta { text, .. } => {
                if batch.discarded {
                    return Err(AgentEngineError::ReplayContract(
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
            AgentEngineEvent::ToolCallCompleted { step, call } if *step > 0 => {
                if batch.discarded {
                    return Err(AgentEngineError::ReplayContract(
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
                    return Err(AgentEngineError::ReplayContract(
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
            AgentEngineEvent::ToolCompleted { step, result } | AgentEngineEvent::ToolOutcomeReconciled { step, result, .. } if *step > 0 => {
                result.validate_for(&result.call_id)?;
                if !batch.calls.contains_key(&result.call_id) {
                    return Err(AgentEngineError::ReplayContract(
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
            AgentEngineEvent::ToolResultsOrdered { step, call_ids } => {
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
            AgentEngineEvent::ContextCompacted {
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
                        .map_err(|error| AgentEngineError::ReplayContract(error.to_string()))?
                        .ok_or_else(|| {
                            AgentEngineError::ReplayContract(
                                "Compaction references do not match a bounded contiguous tool suffix".into(),
                            )
                        })?;
                    exchange
                        .with_required_inputs(&retained_inputs)
                        .map_err(|error| AgentEngineError::ReplayContract(error.to_string()))?
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
            AgentEngineEvent::CompletionReview { status } => {
                batch.flush(history, false)?;
                history.push(status.completion_review_message()?);
            }
            AgentEngineEvent::PlanUpdated { plan } => {
                batch.notices.push(crate::context_lifecycle::text_message(
                    ChatRole::User,
                    plan.context()?,
                ));
            }
            AgentEngineEvent::CompletionReported { report } => {
                batch.notices.push(crate::context_lifecycle::text_message(ChatRole::User,
                    format!("Historical completion account (model assessment with observation references, not fresh proof for this turn): {}",
                        serde_json::to_string(report).map_err(|error| AgentEngineError::ReplayContract(error.to_string()))?)));
            }
            AgentEngineEvent::InstructionsUpdated { context } => {
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
    events: &[AgentEngineEvent],
) -> Result<(), AgentEngineError> {
    replay_into(&mut Vec::new(), requirement, events, true, None, &BTreeMap::new())
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
    results: BTreeMap<ToolCallId, AgentToolResult>,
    context_kinds: BTreeMap<ToolCallId, crate::tool_context::ToolContextKind>,
}

impl ReplayBatch {
    fn validate_event_step(&self, event: &AgentEngineEvent) -> Result<(), AgentEngineError> {
        let tool = match event {
            AgentEngineEvent::ToolCallDelta { step, call_id, .. }
            | AgentEngineEvent::ToolStarted { step, call_id, .. } => Some((*step, call_id)),
            AgentEngineEvent::ToolCallCompleted { step, call } => Some((*step, &call.call_id)),
            AgentEngineEvent::ToolCompleted { step, result } | AgentEngineEvent::ToolOutcomeReconciled { step, result, .. } => Some((*step, &result.call_id)),
            _ => None,
        };
        if let Some((step, call_id)) = tool {
            if step == 0 {
                // Internal instruction reads are deliberately excluded from
                // model history; arbitrary model calls cannot use this lane.
                return if call_id.as_ref().starts_with("agent-instructions:") {
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
        if let AgentEngineEvent::OutputTextDelta { step, .. }
        | AgentEngineEvent::ReasoningDelta { step, .. }
        | AgentEngineEvent::Usage { step, .. } = event
            && (*step == 0 || self.step != Some(*step))
        {
            return Err(invalid(
                "model event differs from the active replay model step",
            ));
        }
        if matches!(
            event,
            AgentEngineEvent::ToolCallDelta { .. }
                | AgentEngineEvent::ToolCallCompleted { .. }
                | AgentEngineEvent::OutputTextDelta { .. }
                | AgentEngineEvent::ReasoningDelta { .. }
                | AgentEngineEvent::Usage { .. }
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
    ) -> Result<(), AgentEngineError> {
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
            let result = self.results.remove(&call_id).unwrap_or_else(|| AgentToolResult::text(
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

fn invalid(message: &str) -> AgentEngineError {
    AgentEngineError::ReplayContract(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEngine, AgentEngineBuild, EngineBuildId, EngineBinding};
    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, CapabilityId, DigestHex, OperationId, ResolvedSnapshotId,
        ResolvedSnapshotRef, RuntimeBindingId, StrictJsonValue,
    };
    use nomifun_chat_model_broker::{ChatContentPart, ChatRole, ChatToolCall};

    fn binding() -> EngineBinding {
        AgentEngine::new(AgentEngineBuild {
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
            &[AgentEngineEvent::TurnStarted {
                binding: binding(),
                turn_operation_id: OperationId::from("turn"),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, AgentEngineError::ReplayContract(message)
            if message.contains("no durable terminal")));
        assert!(history.is_empty());
    }

    #[test]
    fn interrupted_effect_without_a_result_becomes_unknown_and_is_not_replayed() {
        let call_id = ToolCallId::from("effect-1");
        let events = vec![
            AgentEngineEvent::TurnStarted {
                binding: binding(),
                turn_operation_id: OperationId::from("turn"),
            },
            AgentEngineEvent::ModelStepStarted {
                step: 1,
                operation_id: OperationId::from("turn:model:1"),
            },
            AgentEngineEvent::ToolCallCompleted {
                step: 1,
                call: ChatToolCall {
                    call_id: call_id.clone(),
                    name: "write_file".into(),
                    arguments: StrictJsonValue(serde_json::json!({"path":"a","content":"b"})),
                    provider_metadata: None,
                },
            },
            AgentEngineEvent::ToolStarted {
                step: 1,
                call_id: call_id.clone(),
                capability_id: CapabilityId::from("workspace.files"),
                action_id: ActionId::from("workspace.files/write"),
            },
            AgentEngineEvent::TurnFailed {
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
        assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::ToolCompleted { .. })));
    }
}

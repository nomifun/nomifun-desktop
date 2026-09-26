//! Recovery of a quiescent checkpoint. The host must first acquire the
//! canonical execution lease and verify the complete owner journal tail.
//! This type carries data only and cannot authorize or replay any tool.
use std::collections::{BTreeMap, BTreeSet};
use nomifun_chat_model_broker::{ChatMessage, ChatRole, ToolCallId};
use crate::{AgentEngineError, AgentEngineEvent, AgentExecutionCheckpoint, AgentSteeringInput, EngineBinding};

#[derive(Clone, Debug)]
pub struct AgentTurnRecovery {
    pub(crate) checkpoint: AgentExecutionCheckpoint,
    pub(crate) checkpoint_revision: u64,
    pub(crate) execution_fence: u64,
    pub(crate) prefix: Vec<AgentEngineEvent>,
    pub(crate) tail: Vec<AgentEngineEvent>,
    pub(crate) next_instruction_sequence: u32,
    pub(crate) last_model_step: u16,
    pub(crate) reserved_call_ids: BTreeSet<ToolCallId>,
    pub(crate) discarded_call_ids: Vec<ToolCallId>,
    pub(crate) input_replacements: BTreeMap<String, ChatMessage>,
    pub(crate) applied_inputs: Vec<ChatMessage>,
    prepared_inputs: Vec<AgentSteeringInput>,
}

pub(crate) fn discardable_model_event(event: &AgentEngineEvent) -> bool {
    matches!(event, AgentEngineEvent::ModelStepStarted { .. } | AgentEngineEvent::OutputTextDelta { .. }
        | AgentEngineEvent::ReasoningDelta { .. } | AgentEngineEvent::ToolCallDelta { .. }
        | AgentEngineEvent::ToolCallCompleted { .. } | AgentEngineEvent::Usage { .. }
        | AgentEngineEvent::CompactionStarted { .. } | AgentEngineEvent::CompactionUsage { .. }
        | AgentEngineEvent::ContextCompacted { .. } | AgentEngineEvent::ContextLimitRecoveryStarted { .. }
        | AgentEngineEvent::ModelOutputTruncated { .. } | AgentEngineEvent::ModelResponseRejected { .. }
        | AgentEngineEvent::ExecutionResumed { .. } | AgentEngineEvent::ExecutionBudgetPrepared { .. }
        | AgentEngineEvent::ExecutionSegmentRenewed { .. } | AgentEngineEvent::ContextPrepared { .. }
        | AgentEngineEvent::RuntimeModulesActivated { .. })
}

impl AgentTurnRecovery {
    pub fn checkpoint(&self) -> &AgentExecutionCheckpoint { &self.checkpoint }
    pub fn checkpoint_revision(&self) -> u64 { self.checkpoint_revision }
    pub fn last_model_step(&self) -> u16 { self.last_model_step }
    pub fn execution_fence(&self) -> u64 { self.execution_fence }
    pub fn prepared_inputs(&self) -> &[AgentSteeringInput] { &self.prepared_inputs }

    pub fn new(checkpoint: AgentExecutionCheckpoint, checkpoint_revision: u64, execution_fence: u64,
        prefix: Vec<AgentEngineEvent>, tail: Vec<AgentEngineEvent>, prepared_inputs: Vec<AgentSteeringInput>) -> Result<Self, AgentEngineError> {
        checkpoint.validate()?;
        let fail = || AgentEngineError::ReplayContract("recovery journal differs from its exact checkpoint boundary".into());
        if execution_fence == 0 || checkpoint_revision == 0 || !matches!(prefix.last(),
            Some(AgentEngineEvent::ExecutionCheckpointSaved { step, revision, .. })
                if *step == checkpoint.model_steps && *revision == checkpoint_revision)
            || !matches!(prefix.first(), Some(AgentEngineEvent::TurnStarted { binding, turn_operation_id })
                if binding == &checkpoint.binding && turn_operation_id == &checkpoint.turn_operation_id)
            || tail.iter().any(|event| !discardable_model_event(event)) {
            return Err(fail());
        }
        crate::stream_limits::serialized_size(&prefix, nomifun_agent_contracts::MAX_NATIVE_APPROVED_JOURNAL_BYTES as usize)?;
        crate::stream_limits::serialized_size(&tail, 8 * 1024 * 1024)?;
        let mut reserved = BTreeSet::new();
        let mut recorded_inputs = Vec::new();
        for event in &prefix {
            match event {
                AgentEngineEvent::ToolCallDelta { call_id, .. } => { reserved.insert(call_id.clone()); }
                AgentEngineEvent::ToolCallCompleted { call, .. } => { reserved.insert(call.call_id.clone()); }
                AgentEngineEvent::SteeringInputs { inputs } => recorded_inputs.extend(inputs.iter().cloned()),
                _ => {}
            }
        }
        if recorded_inputs.iter().map(|input| input.receipt_operation_id.as_str()).collect::<Vec<_>>()
            != checkpoint.applied_steering_receipts.iter().map(|id| id.as_ref()).collect::<Vec<_>>() {
            return Err(fail());
        }
        let mut replacements = BTreeMap::new();
        let mut applied_inputs = Vec::new();
        if prepared_inputs.len() != recorded_inputs.len() { return Err(fail()); }
        for (recorded, prepared) in recorded_inputs.iter().zip(&prepared_inputs) {
            prepared.validate()?;
            if !prepared.same_delivery(recorded) || prepared.image_count != prepared.prepared_images.len() {
                return Err(fail());
            }
            let message = prepared.project_message(true);
            replacements.insert(prepared.receipt_operation_id.clone(), message.clone());
            applied_inputs.push(message);
        }
        let mut discarded = BTreeSet::new();
        let mut last_model_step = checkpoint.model_steps;
        let mut last_recovery_fence = 0;
        for event in &tail {
            let event_step = match event {
                AgentEngineEvent::ToolCallCompleted { step: 0, call } if call.name == "read_file" && call.call_id.as_ref().starts_with("agent-instructions:") => None,
                AgentEngineEvent::OutputTextDelta { step, .. } | AgentEngineEvent::ReasoningDelta { step, .. }
                | AgentEngineEvent::ToolCallDelta { step, .. } | AgentEngineEvent::ToolCallCompleted { step, .. }
                | AgentEngineEvent::Usage { step, .. } | AgentEngineEvent::ModelOutputTruncated { step, .. }
                | AgentEngineEvent::ModelResponseRejected { step, .. } => Some(*step), _ => None,
            };
            if event_step.is_some_and(|step| step != last_model_step || step <= checkpoint.model_steps) { return Err(fail()); }
            match event {
                AgentEngineEvent::ExecutionResumed { checkpoint_revision: revision, checkpoint_step, model_steps, execution_fence: fence, discarded_tool_call_ids } => {
                    if *revision != checkpoint_revision || *checkpoint_step != checkpoint.model_steps
                        || *model_steps != last_model_step || *fence <= last_recovery_fence || *fence >= execution_fence
                        || discarded_tool_call_ids.iter().cloned().collect::<BTreeSet<_>>() != discarded
                        || discarded_tool_call_ids.len() != discarded.len() {
                        return Err(fail());
                    }
                    last_recovery_fence = *fence;
                }
                AgentEngineEvent::ExecutionSegmentRenewed { segment, model_steps, checkpoint_revision: revision, .. } => {
                    if *revision != checkpoint_revision || *model_steps != checkpoint.model_steps
                        || checkpoint.segments.as_ref().is_none_or(|state| state.segment != *segment) {
                        return Err(fail());
                    }
                }
                AgentEngineEvent::ModelStepStarted { step, .. } => {
                    if last_model_step.checked_add(1) != Some(*step) { return Err(fail()); }
                    last_model_step = *step;
                }
                AgentEngineEvent::ToolCallDelta { call_id, .. } => { discarded.insert(call_id.clone()); }
                AgentEngineEvent::ToolCallCompleted { step, call } if *step > 0 => { discarded.insert(call.call_id.clone()); }
                _ => {}
            }
        }
        if discarded.iter().any(|id| reserved.contains(id)) { return Err(fail()); }
        reserved.extend(discarded.iter().cloned());
        let next_instruction_sequence = prefix.iter().chain(&tail).filter_map(|event| match event {
            AgentEngineEvent::ToolCallCompleted { step: 0, call } => call.call_id.as_ref().strip_prefix("agent-instructions:")?.parse::<u32>().ok(),
            _ => None,
        }).max().map_or(Some(100), |sequence| sequence.checked_add(1)).ok_or_else(fail)?;
        Ok(Self { checkpoint, checkpoint_revision, execution_fence, prefix, tail, next_instruction_sequence, last_model_step,
            reserved_call_ids: reserved, discarded_call_ids: discarded.into_iter().collect(),
            input_replacements: replacements, applied_inputs, prepared_inputs })
    }

    pub(crate) fn validate_for(&self, binding: &EngineBinding, operation: &nomifun_agent_contracts::OperationId, generation: u64) -> Result<(), AgentEngineError> {
        if &self.checkpoint.binding != binding || &self.checkpoint.turn_operation_id != operation
            || self.checkpoint.active_set_generation != generation {
            return Err(AgentEngineError::InvalidContract("recovery cannot change Session, build, Snapshot, Turn or capability generation".into()));
        }
        Ok(())
    }
}

pub(crate) fn notice() -> ChatMessage {
    crate::context_lifecycle::text_message(ChatRole::User,
        "Engine recovery observation, not a new user request: execution resumed from its last quiescent checkpoint. Unexecuted model output after that checkpoint was discarded; no prior tool was replayed. Preserve the complete original task and accepted corrections. Reinspect current workspace/instructions and pending outcomes before new effects. Historical checks are not current verification. Old process handles and tool-archive IDs are not reusable; obtain fresh authorized observations. Do not redo already completed edits just because the process restarted.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint() -> AgentExecutionCheckpoint {
        AgentExecutionCheckpoint {
            version: 1,
            binding: EngineBinding::new("session".into(), "binding".into(), "build".into(), "a".repeat(64).into(),
                nomifun_agent_contracts::ResolvedSnapshotRef { snapshot_id: "snapshot".into(), snapshot_digest: "b".repeat(64).into() }).unwrap(),
            turn_operation_id: "turn".into(), active_set_generation: 0, model_steps: 0, tool_call_count: 0,
            accepted_input_count: 1, applied_steering_receipts: vec![], plan: Default::default(), work: Default::default(),
            patch_recovery: Default::default(), segments: None, control_rejections: Default::default(),
        }
    }

    #[test]
    fn repeated_recovery_can_reuse_one_checkpoint_without_resurrecting_abandoned_text() {
        let checkpoint = checkpoint();
        let prefix = vec![AgentEngineEvent::TurnStarted { binding: checkpoint.binding.clone(), turn_operation_id: "turn".into() },
            AgentEngineEvent::ExecutionCheckpointSaved { step: 0, revision: 1, digest: "c".repeat(64).into() }];
        let tail = vec![
            AgentEngineEvent::ModelStepStarted { step: 1, operation_id: "turn:model:1".into() },
            AgentEngineEvent::OutputTextDelta { step: 1, text: "abandoned-one".into() },
            AgentEngineEvent::ExecutionResumed { checkpoint_revision: 1, checkpoint_step: 0, model_steps: 1, execution_fence: 1, discarded_tool_call_ids: vec![] },
            AgentEngineEvent::ExecutionBudgetPrepared { context_window_tokens: 32768, max_output_tokens: 4096, max_model_steps: 32 },
            AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "turn:model:2".into() },
            AgentEngineEvent::OutputTextDelta { step: 2, text: "abandoned-two".into() },
        ];
        let recovery = AgentTurnRecovery::new(checkpoint.clone(), 1, 2, prefix.clone(), tail.clone(), vec![]).unwrap();
        assert_eq!(recovery.last_model_step, 2);
        let mut history_events = prefix.clone(); history_events.extend(tail.clone());
        history_events.extend([
            AgentEngineEvent::ExecutionResumed { checkpoint_revision: 1, checkpoint_step: 0, model_steps: 2, execution_fence: 2, discarded_tool_call_ids: vec![] },
            AgentEngineEvent::ModelStepStarted { step: 3, operation_id: "turn:model:3".into() },
            AgentEngineEvent::OutputTextDelta { step: 3, text: "new-result".into() },
            AgentEngineEvent::TurnCompleted { model_steps: 3, finish_reason: nomifun_chat_model_broker::ChatFinishReason::Completed },
        ]);
        let mut history = Vec::new();
        crate::replay_closed_turn(&mut history, crate::context_lifecycle::text_message(ChatRole::User, "original task".into()), &history_events).unwrap();
        let encoded = serde_json::to_string(&history).unwrap();
        assert!(encoded.contains("original task") && encoded.contains("new-result"));
        assert!(!encoded.contains("abandoned-one") && !encoded.contains("abandoned-two"));
        let mut unsafe_tail = tail;
        unsafe_tail.push(AgentEngineEvent::ToolStarted { step: 2, call_id: "effect".into(), capability_id: "workspace.files".into(), action_id: "workspace.files/write".into() });
        assert!(AgentTurnRecovery::new(checkpoint, 1, 2, prefix, unsafe_tail, vec![]).is_err());
    }
}

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
        | AgentEngineEvent::CompactionSummaryRejected { .. }
        | AgentEngineEvent::ContextCompacted { .. } | AgentEngineEvent::ContextLimitRecoveryStarted { .. }
        | AgentEngineEvent::ModelOutputTruncated { .. } | AgentEngineEvent::ModelResponseRejected { .. }
        | AgentEngineEvent::ModelResponseValidated { .. }
        | AgentEngineEvent::VoiceModelStepSuperseded { .. }
        | AgentEngineEvent::ExecutionResumed { .. } | AgentEngineEvent::ExecutionBudgetPrepared { .. }
        | AgentEngineEvent::ExecutionSegmentRenewed { .. } | AgentEngineEvent::ContextPrepared { .. }
        | AgentEngineEvent::RuntimeModulesActivated { .. })
        || matches!(event, AgentEngineEvent::AwaitingChildren { .. })
}

/// Validate a parked child boundary with the existing typed progress reducer.
/// Its explicit-resume proposal is not applied: automatic attachment retains
/// the original checkpoint, budget, counters, plan and result coverage.
fn validate_child_wait_tail(checkpoint:&AgentExecutionCheckpoint,revision:u64,tail:&[AgentEngineEvent])
    ->Result<(),AgentEngineError> {
    let Some(wait)=checkpoint.awaiting_children.as_ref() else {return Ok(());};
    let invalid=||AgentEngineError::ReplayContract("waiting checkpoint crossed an execution or different child-wait boundary".into());
    if !checkpoint.work.running_processes.is_empty() || tail.iter().any(|event|match event {
        AgentEngineEvent::AwaitingChildren {model_steps,request}=>*model_steps!=checkpoint.model_steps||request!=wait,
        AgentEngineEvent::ExecutionResumed {..}|AgentEngineEvent::ExecutionBudgetPrepared {..}
            |AgentEngineEvent::ExecutionSegmentRenewed {..}|AgentEngineEvent::ContextPrepared {..}
            |AgentEngineEvent::RuntimeModulesActivated {..}=>false,
        _=>true,
    }) {return Err(invalid());}
    let reduced=crate::reconcile_execution_tail_with_contracts(checkpoint,revision,tail,&BTreeSet::new(),&BTreeMap::new(),
        &nomifun_agent_contracts::NativeBudgetIncrease::default(),&crate::AgentReconciliationContracts::new())?;
    if reduced.checkpoint.model_steps!=checkpoint.model_steps||reduced.checkpoint.tool_call_count!=checkpoint.tool_call_count
        ||reduced.checkpoint.accepted_input_count!=checkpoint.accepted_input_count
        ||reduced.checkpoint.applied_steering_receipts!=checkpoint.applied_steering_receipts
        ||reduced.checkpoint.awaiting_children!=checkpoint.awaiting_children||reduced.checkpoint.collaboration!=checkpoint.collaboration
        ||reduced.checkpoint.control_rejections!=checkpoint.control_rejections {return Err(invalid());}
    Ok(())
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
        validate_child_wait_tail(&checkpoint,checkpoint_revision,&tail)?;
        crate::stream_limits::serialized_size(&prefix, nomifun_agent_contracts::MAX_NATIVE_APPROVED_JOURNAL_BYTES as usize)?;
        crate::stream_limits::serialized_size(&tail, 8 * 1024 * 1024)?;
        let (collaboration, awaiting_children) = crate::collaboration::recorded_progress(&prefix)?;
        if collaboration != checkpoint.collaboration
            || (awaiting_children != checkpoint.awaiting_children
                && collaboration.wait().as_ref() != checkpoint.awaiting_children.as_ref())
            || crate::user_input_request::recorded_pending(&prefix)? != checkpoint.pending_user_input
            || (checkpoint.pending_user_input.is_some() && tail.iter().any(|event|
                matches!(event, AgentEngineEvent::ModelStepStarted { .. }))) {
            return Err(fail());
        }
        if tail.iter().any(|event| matches!(event, AgentEngineEvent::AwaitingChildren { model_steps, request }
            if *model_steps != checkpoint.model_steps || checkpoint.awaiting_children.as_ref() != Some(request))) {
            return Err(fail());
        }
        if let Some(request) = &checkpoint.pending_user_input {
            if !prefix.iter().any(|event| matches!(event,
                AgentEngineEvent::ToolCompleted { result, .. } | AgentEngineEvent::ToolOutcomeReconciled { result, .. }
                    if result.call_id == request.request_id && !result.is_error)) { return Err(fail()); }
        }
        let mut protocol_recovery = crate::protocol_recovery::ProtocolRecovery::default();
        protocol_recovery.restore(prefix.iter().chain(&tail))?;
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
        if prepared_inputs.iter().flat_map(|input| &input.prepared_skill_instructions).map(String::len).sum::<usize>() > 24 * 1024 {
            return Err(fail());
        }
        for (recorded, prepared) in recorded_inputs.iter().zip(&prepared_inputs) {
            prepared.validate_prepared_live()?;
            if !prepared.same_delivery(recorded) {
                return Err(fail());
            }
            let message = prepared.project_message(true);
            replacements.insert(prepared.receipt_operation_id.clone(), message.clone());
            applied_inputs.push(message);
        }
        let mut discarded = BTreeSet::new();
        let mut batch_proposals = BTreeSet::new();
        let mut batch_completed = BTreeSet::new();
        let mut batch_discarded = false;
        let mut batch_validated = false;
        let mut last_model_step = checkpoint.model_steps;
        let mut last_recovery_fence = 0;
        for event in &tail {
            let event_step = match event {
                AgentEngineEvent::ToolCallCompleted { step: 0, call } if call.name == "read_file" && call.call_id.as_ref().starts_with("agent-instructions:") => None,
                AgentEngineEvent::OutputTextDelta { step, .. } | AgentEngineEvent::ReasoningDelta { step, .. }
                | AgentEngineEvent::ToolCallDelta { step, .. } | AgentEngineEvent::ToolCallCompleted { step, .. }
                | AgentEngineEvent::Usage { step, .. } | AgentEngineEvent::ModelOutputTruncated { step, .. }
                | AgentEngineEvent::ModelResponseRejected { step, .. } | AgentEngineEvent::ModelResponseValidated { step }
                | AgentEngineEvent::VoiceModelStepSuperseded {step,..} => Some(*step), _ => None,
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
                    batch_proposals.clear(); batch_completed.clear(); batch_discarded = false; batch_validated = false;
                }
                AgentEngineEvent::ToolCallDelta { call_id, .. } => {
                    if batch_discarded || batch_validated { return Err(fail()); }
                    batch_proposals.insert(call_id.clone()); discarded.insert(call_id.clone());
                }
                AgentEngineEvent::ToolCallCompleted { step, call } if *step > 0 => {
                    if batch_discarded || batch_validated || !batch_completed.insert(call.call_id.clone()) { return Err(fail()); }
                    batch_proposals.insert(call.call_id.clone()); discarded.insert(call.call_id.clone());
                }
                AgentEngineEvent::ModelResponseValidated { .. } => {
                    if batch_discarded || batch_proposals != batch_completed { return Err(fail()); }
                    batch_validated = true;
                }
                AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, .. }
                | AgentEngineEvent::ModelOutputTruncated { discarded_tool_call_ids, .. }
                | AgentEngineEvent::VoiceModelStepSuperseded { discarded_tool_call_ids, .. } => {
                    if batch_discarded || batch_proposals != discarded_tool_call_ids.iter().cloned().collect() {
                        return Err(fail());
                    }
                    batch_discarded = true;
                }
                AgentEngineEvent::OutputTextDelta { .. } | AgentEngineEvent::ReasoningDelta { .. }
                | AgentEngineEvent::Usage { .. } if batch_validated || batch_discarded => return Err(fail()),
                _ => {}
            }
        }
        if discarded.iter().any(|id| reserved.contains(id)) { return Err(fail()); }
        if checkpoint.segments.as_ref().is_some_and(|state| last_model_step > state.total_model_limit()) {
            return Err(fail());
        }
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
            patch_recovery: Default::default(), segments: None, control_rejections: Default::default(), completion_review: Default::default(), pending_user_input: None, awaiting_children: None, collaboration: Default::default(),
        }
    }

    fn child_wait_checkpoint() -> (AgentExecutionCheckpoint,Vec<AgentEngineEvent>) {
        let mut checkpoint=checkpoint();
        checkpoint.model_steps=1;checkpoint.tool_call_count=1;
        let wait=crate::AgentChildrenWait {execution_ids:vec!["original-execution".into()],after_cursor:None,
            mode:crate::AgentChildrenWaitMode::All};
        let cursor=crate::AgentCollaborationSnapshot::observation_cursor(BTreeMap::from([("original-execution".into(),
            crate::AgentCollaborationObservation {state:"4".repeat(64),ready:"5".repeat(64)})])).unwrap();
        let mut prefix=vec![AgentEngineEvent::TurnStarted {binding:checkpoint.binding.clone(),turn_operation_id:checkpoint.turn_operation_id.clone()},
            AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None},
            AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {call_id:"original-wait".into(),
                name:crate::collaboration::WAIT.into(),arguments:nomifun_agent_contracts::StrictJsonValue(serde_json::to_value(&wait).unwrap()),provider_metadata:None}},
            AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,result:crate::AgentToolResult::text("original-wait".into(),
                serde_json::json!({"state":"awaiting_children","cursor":cursor}).to_string(),false)}];
        (checkpoint.collaboration,checkpoint.awaiting_children)=crate::collaboration::recorded_progress(&prefix).unwrap();
        assert_eq!(checkpoint.awaiting_children.as_ref(),Some(&wait));
        let digest=nomifun_agent_contracts::digest_payload(&checkpoint).unwrap();
        prefix.push(AgentEngineEvent::ExecutionCheckpointSaved {step:1,revision:1,digest});
        (checkpoint,prefix)
    }

    #[test]
    fn automatic_recovery_validates_the_same_pure_child_wait_without_spending_or_replaying_work() {
        let (checkpoint,prefix)=child_wait_checkpoint();
        let wait=AgentEngineEvent::AwaitingChildren {model_steps:checkpoint.model_steps,request:checkpoint.awaiting_children.clone().unwrap()};
        let tail=vec![wait.clone(),AgentEngineEvent::ExecutionResumed {checkpoint_revision:1,checkpoint_step:1,
            model_steps:1,execution_fence:1,discarded_tool_call_ids:vec![]},
            AgentEngineEvent::ExecutionBudgetPrepared {context_window_tokens:32768,max_output_tokens:4096,max_model_steps:32},wait];
        let recovered=AgentTurnRecovery::new(checkpoint.clone(),1,2,prefix,tail,vec![]).unwrap();
        assert_eq!(recovered.checkpoint(),&checkpoint,"validation cannot apply the explicit-resume plan, work or budget rewrite");
        assert_eq!(recovered.last_model_step(),1);
        assert!(recovered.discarded_call_ids.is_empty());
        assert_eq!(recovered.reserved_call_ids,BTreeSet::from(["original-wait".into()]));
    }

    #[test]
    fn automatic_child_wait_recovery_rejects_changed_requests_and_any_execution_tail() {
        let (checkpoint,prefix)=child_wait_checkpoint();
        let wait=checkpoint.awaiting_children.clone().unwrap();
        let mut changed_scope=wait.clone();changed_scope.execution_ids=vec!["another-execution".into()];
        let mut changed_mode=wait.clone();changed_mode.mode=crate::AgentChildrenWaitMode::Any;
        let mut changed_cursor=wait.clone();changed_cursor.after_cursor=Some(crate::AgentCollaborationSnapshot::observation_cursor(
            BTreeMap::from([("original-execution".into(),crate::AgentCollaborationObservation {state:"6".repeat(64),ready:"7".repeat(64)})])).unwrap());
        let rejected=vec![
            AgentEngineEvent::AwaitingChildren {model_steps:1,request:changed_scope},
            AgentEngineEvent::AwaitingChildren {model_steps:1,request:changed_mode},
            AgentEngineEvent::AwaitingChildren {model_steps:1,request:changed_cursor},
            AgentEngineEvent::AwaitingChildren {model_steps:2,request:wait},
            AgentEngineEvent::ModelStepStarted {step:2,operation_id:"turn:model:2".into(),correction_source:None},
            AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {call_id:"new-tool".into(),name:"read_file".into(),
                arguments:nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"path":"source.txt"})),provider_metadata:None}},
            AgentEngineEvent::ChildrenWaitResolved {model_steps:1,cursor:crate::AgentCollaborationSnapshot::observation_cursor(BTreeMap::from([
                ("original-execution".into(),crate::AgentCollaborationObservation {state:"8".repeat(64),ready:"9".repeat(64)})])).unwrap()},
            AgentEngineEvent::WorkStatus {status:Default::default()},
        ];
        for event in rejected {
            assert!(AgentTurnRecovery::new(checkpoint.clone(),1,1,prefix.clone(),vec![event.clone()],vec![]).is_err(),"unsafe tail: {event:?}");
        }
        let mut live=checkpoint.clone();live.work.running_processes.insert("unreaped-original-owner".into());
        assert!(AgentTurnRecovery::new(live,1,1,prefix.clone(),vec![],vec![]).is_err());
        let mut wrong_prefix=prefix;wrong_prefix[0]=AgentEngineEvent::TurnStarted {binding:checkpoint.binding.clone(),turn_operation_id:"another-turn".into()};
        assert!(AgentTurnRecovery::new(checkpoint,1,1,wrong_prefix,vec![],vec![]).is_err());
    }

    #[test]
    fn repeated_recovery_can_reuse_one_checkpoint_without_resurrecting_abandoned_text() {
        let checkpoint = checkpoint();
        let prefix = vec![AgentEngineEvent::TurnStarted { binding: checkpoint.binding.clone(), turn_operation_id: "turn".into() },
            AgentEngineEvent::ExecutionCheckpointSaved { step: 0, revision: 1, digest: "c".repeat(64).into() }];
        let tail = vec![
            AgentEngineEvent::ModelStepStarted { step: 1, operation_id: "turn:model:1".into(), correction_source: None },
            AgentEngineEvent::OutputTextDelta { step: 1, text: "abandoned-one".into(), source: None },
            AgentEngineEvent::ExecutionResumed { checkpoint_revision: 1, checkpoint_step: 0, model_steps: 1, execution_fence: 1, discarded_tool_call_ids: vec![] },
            AgentEngineEvent::ExecutionBudgetPrepared { context_window_tokens: 32768, max_output_tokens: 4096, max_model_steps: 32 },
            AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "turn:model:2".into(), correction_source: None },
            AgentEngineEvent::OutputTextDelta { step: 2, text: "abandoned-two".into(), source: None },
        ];
        let recovery = AgentTurnRecovery::new(checkpoint.clone(), 1, 2, prefix.clone(), tail.clone(), vec![]).unwrap();
        assert_eq!(recovery.last_model_step, 2);
        let mut history_events = prefix.clone(); history_events.extend(tail.clone());
        history_events.extend([
            AgentEngineEvent::ExecutionResumed { checkpoint_revision: 1, checkpoint_step: 0, model_steps: 2, execution_fence: 2, discarded_tool_call_ids: vec![] },
            AgentEngineEvent::ModelStepStarted { step: 3, operation_id: "turn:model:3".into(), correction_source: None },
            AgentEngineEvent::OutputTextDelta { step: 3, text: "new-result".into(), source: None },
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

    #[test]
    fn applied_steering_recovery_requires_rehydrated_skill_body_without_journaling_it() {
        let mut checkpoint = checkpoint();
        checkpoint.accepted_input_count = 2;
        checkpoint.applied_steering_receipts = vec!["skill-receipt".into()];
        let recorded = AgentSteeringInput {
            receipt_operation_id: "skill-receipt".into(), message_id: "skill-message".into(),
            text: "Use the selected Skill".into(), files: vec![],
            inject_skills: vec!["selected-skill".into()], image_count: 0,
            prepared_images: vec![], prepared_skill_instructions: vec![],
        };
        let mut prepared = recorded.clone();
        prepared.prepared_skill_instructions = vec!["EXACT_FROZEN_SKILL_BODY".into()];
        let prefix = vec![
            AgentEngineEvent::TurnStarted { binding: checkpoint.binding.clone(), turn_operation_id: "turn".into() },
            AgentEngineEvent::SteeringInputs { inputs: vec![recorded] },
            AgentEngineEvent::ExecutionCheckpointSaved { step: 0, revision: 1, digest: "c".repeat(64).into() },
        ];
        let recovery = AgentTurnRecovery::new(checkpoint.clone(), 1, 2, prefix.clone(), vec![], vec![prepared.clone()]).unwrap();
        assert_eq!(recovery.prepared_inputs()[0].prepared_skill_instructions,
            ["EXACT_FROZEN_SKILL_BODY"]);
        assert!(!serde_json::to_string(&prefix).unwrap().contains("EXACT_FROZEN_SKILL_BODY"));
        assert!(!serde_json::to_string(&recovery.applied_inputs).unwrap().contains("EXACT_FROZEN_SKILL_BODY"));
        prepared.prepared_skill_instructions.clear();
        assert!(AgentTurnRecovery::new(checkpoint, 1, 2, prefix, vec![], vec![prepared]).is_err(),
            "an old ID-only journal must be rehydrated from the exact frozen Skill before resume");
    }
}

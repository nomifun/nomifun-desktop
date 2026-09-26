//! Pure reconstruction from canonical facts. This module cannot invoke a
//! model/tool, invent an effect outcome, or authorize its own continuation.
use std::collections::{BTreeMap, BTreeSet};
use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};
use crate::{AgentEngineError, AgentEngineEvent, AgentExecutionCheckpoint, AgentToolResult};

#[cfg(test)]
#[path = "reconciliation_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentReconciliationSource {
    OwnerReceipt,
    OwnerAttestation,
    ReadOutcomeUnavailable,
    NotDispatched,
}

#[derive(Clone, Debug)]
pub struct AgentReconciledOutcome {
    pub result: AgentToolResult,
    pub source: AgentReconciliationSource,
    pub evidence_event_id: Option<String>,
    pub owner_operation_id: Option<nomifun_agent_contracts::OperationId>,
}

pub struct AgentReconciledResume {
    pub checkpoint: AgentExecutionCheckpoint,
    pub observations: Vec<AgentEngineEvent>,
}

pub fn reconcile_execution_tail(
    checkpoint: &AgentExecutionCheckpoint,
    checkpoint_revision: u64,
    tail: &[AgentEngineEvent],
    externally_dispatched: &BTreeSet<ToolCallId>,
    outcomes: &BTreeMap<ToolCallId, AgentReconciledOutcome>,
    grant: &nomifun_agent_contracts::NativeBudgetIncrease,
) -> Result<AgentReconciledResume, AgentEngineError> {
    checkpoint.validate()?;
    let invalid = |message: &str| AgentEngineError::ReplayContract(format!("resume reconciliation: {message}"));
    let mut next = checkpoint.clone();
    let mut step = checkpoint.model_steps;
    let mut proposed = BTreeSet::new();
    let mut order = Vec::new();
    let mut calls = BTreeMap::<ToolCallId, (u16, ChatToolCall)>::new();
    let mut completed = BTreeSet::new();
    let mut admitted = externally_dispatched.clone();
    let mut current_results = BTreeSet::new();
    let mut discarded = false;
    let mut new_results = 0u32;
    let mut inputs: BTreeSet<_> = checkpoint.applied_steering_receipts.iter().cloned().collect();
    let plan_changed = tail.iter().any(|event| matches!(event, AgentEngineEvent::PlanUpdated { plan } if plan.revision != checkpoint.plan.revision));
    for event in tail {
        match event {
            AgentEngineEvent::ModelStepStarted { step: current, .. } => {
                if *current != step.checked_add(1).ok_or_else(|| invalid("model counter exhausted"))?
                    || (!discarded && !proposed.is_empty() && proposed != current_results) {
                    return Err(invalid("a model step crossed an unresolved previous batch"));
                }
                step = *current; proposed.clear(); order.clear(); current_results.clear(); discarded = false;
            }
            AgentEngineEvent::ToolCallDelta { step: current, call_id, .. } if *current > 0 => {
                if *current != step || discarded { return Err(invalid("proposal is outside its model batch")); }
                if proposed.insert(call_id.clone()) { order.push(call_id.clone()); }
            }
            AgentEngineEvent::ToolCallCompleted { step: current, call } => {
                if *current > 0 && (*current != step || discarded) { return Err(invalid("completed proposal has no model batch")); }
                if calls.insert(call.call_id.clone(), (*current, call.clone())).is_some() { return Err(invalid("duplicate completed call identity")); }
                if *current > 0 && proposed.insert(call.call_id.clone()) { order.push(call.call_id.clone()); }
            }
            AgentEngineEvent::ToolStarted { step: current, call_id, .. } => {
                if calls.get(call_id).is_none_or(|(recorded, _)| recorded != current) { return Err(invalid("tool admission has no exact completed proposal")); }
                admitted.insert(call_id.clone());
            }
            AgentEngineEvent::ToolCompleted { step: current, result }
            | AgentEngineEvent::ToolOutcomeReconciled { step: current, result, .. } => {
                let (recorded_step, call) = calls.get(&result.call_id).ok_or_else(|| invalid("result has no proposal"))?;
                if recorded_step != current || !completed.insert(result.call_id.clone()) { return Err(invalid("duplicate or mismatched result")); }
                result.validate_for(&result.call_id)?;
                if *current > 0 {
                    current_results.insert(result.call_id.clone()); new_results = new_results.saturating_add(1);
                    if admitted.contains(&result.call_id) { if let Some(segments) = next.segments.as_mut() { segments.observe(call,result)?; } }
                    let changed = if call.name == crate::planning::TOOL_NAME { plan_changed } else { true };
                    next.control_rejections.observe(&call.name, result, changed);
                }
            }
            AgentEngineEvent::ModelOutputTruncated { step: current, discarded_tool_call_ids, .. }
            | AgentEngineEvent::ModelResponseRejected { step: current, discarded_tool_call_ids, .. } => {
                if *current != step || !current_results.is_empty() || proposed.iter().any(|id| admitted.contains(id))
                    || proposed != discarded_tool_call_ids.iter().cloned().collect() { return Err(invalid("discard would hide an admitted effect")); }
                discarded = true;
            }
            AgentEngineEvent::SteeringInputs { inputs: applied } => for input in applied {
                input.validate()?;
                let id = nomifun_agent_contracts::OperationId::from(input.receipt_operation_id.clone());
                if !inputs.insert(id.clone()) { return Err(invalid("accepted input was applied twice")); }
                next.applied_steering_receipts.push(id);
            },
            AgentEngineEvent::PlanUpdated { plan } => next.plan = plan.clone(),
            AgentEngineEvent::WorkStatus { status } => next.work = status.clone(),
            AgentEngineEvent::PatchRecoveryUpdated { state } => next.patch_recovery = state.clone(),
            // The host separately proves the canonical Turn is nonterminal.
            // A private terminal proposal can precede a failed terminal commit.
            AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnCancelled { .. } | AgentEngineEvent::TurnFailed { .. } => {},
            AgentEngineEvent::TurnStarted { .. } | AgentEngineEvent::ExecutionCheckpointSaved { .. } => return Err(invalid("a different root or checkpoint cannot be resumed")),
            _ => {}
        }
    }
    let keep_batch = !current_results.is_empty() || proposed.iter().any(|id| admitted.contains(id));
    if keep_batch && proposed.iter().any(|id| !calls.contains_key(id)) { return Err(invalid("admitted batch contains incomplete arguments")); }
    let mut observations = Vec::new();
    for (id, (call_step, call)) in &calls {
        if completed.contains(id) || (*call_step > 0 && (!keep_batch || *call_step != step)) { continue; }
        let outcome = if let Some(outcome) = outcomes.get(id) {
            outcome.clone()
        } else if !admitted.contains(id) {
            AgentReconciledOutcome { result: AgentToolResult::text(id.clone(), "Not dispatched: the paused execution never admitted this proposal. Reconsider under the current task before any new call.", true),
                source: AgentReconciliationSource::NotDispatched, evidence_event_id: None, owner_operation_id: None }
        } else { return Err(invalid("admitted invocation has no owner-backed reconciliation outcome")); };
        outcome.result.validate_for(id)?;
        if admitted.contains(id) && matches!(outcome.source, AgentReconciliationSource::NotDispatched)
            && outcome.evidence_event_id.is_none() { return Err(invalid("missing dispatch needs a fenced journal witness")); }
        if *call_step > 0 { new_results = new_results.saturating_add(1); }
        if *call_step > 0 && admitted.contains(id) { if let Some(segments) = next.segments.as_mut() { segments.observe(call,&outcome.result)?; } }
        observations.extend([AgentEngineEvent::ToolOutcomeReconciled { step: *call_step, result: outcome.result,
            source: outcome.source, evidence_event_id: outcome.evidence_event_id, owner_operation_id: outcome.owner_operation_id }]);
    }
    let discard_last = !keep_batch;
    observations.push(AgentEngineEvent::ExecutionTailReconciled {
        model_steps: step, source_checkpoint_revision: checkpoint_revision,
        discarded_tool_call_ids: if discard_last { proposed.iter().cloned().collect() } else { vec![] },
        retained_tool_call_ids: if keep_batch { order } else { vec![] }, discard_last_model_step: discard_last,
        retry_stall_guards: grant.retry_stall_guards,
    });
    next.model_steps = step;
    next.tool_call_count = next.tool_call_count.checked_add(new_results).ok_or_else(|| invalid("tool count exhausted"))?;
    next.accepted_input_count = 1 + next.applied_steering_receipts.len();
    next.plan.needs_replan = true;
    // The caller must establish actual owner cleanup before committing this
    // proposal. No command/verification provenance survives the resume.
    next.work.running_processes.clear(); next.work.observed_processes.clear(); next.work.recent_commands.clear();
    next.work.command_observed_after_latest_mutation = false;
    if grant.retry_stall_guards { next.control_rejections.reset(); }
    if let Some(segments) = next.segments.as_mut() { segments.authorize_resume(grant, step)?; }
    else if grant.additional_segments != 0 { return Err(invalid("no segmented model budget exists at this checkpoint")); }
    next.validate()?;
    Ok(AgentReconciledResume { checkpoint: next, observations })
}

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
    NativeControlReceipt,
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

/// Read-only effect classifications from the exact validated frozen Snapshot.
/// These contracts interpret observations; they never authorize execution.
pub type AgentReconciliationContracts = BTreeMap<
    (nomifun_agent_contracts::CapabilityId, nomifun_agent_contracts::ActionId), crate::AgentEffectClass>;

pub fn reconcile_execution_tail_with_contracts(
    checkpoint: &AgentExecutionCheckpoint, checkpoint_revision: u64, tail: &[AgentEngineEvent],
    externally_dispatched: &BTreeSet<ToolCallId>, outcomes: &BTreeMap<ToolCallId, AgentReconciledOutcome>,
    grant: &nomifun_agent_contracts::NativeBudgetIncrease, contracts: &AgentReconciliationContracts,
) -> Result<AgentReconciledResume, AgentEngineError> {
    reconcile_tail(checkpoint, checkpoint_revision, tail, externally_dispatched, outcomes, grant, Some(contracts))
}

#[cfg(test)]
pub fn reconcile_execution_tail(
    checkpoint: &AgentExecutionCheckpoint,
    checkpoint_revision: u64,
    tail: &[AgentEngineEvent],
    externally_dispatched: &BTreeSet<ToolCallId>,
    outcomes: &BTreeMap<ToolCallId, AgentReconciledOutcome>,
    grant: &nomifun_agent_contracts::NativeBudgetIncrease,
) -> Result<AgentReconciledResume, AgentEngineError> {
    reconcile_tail(checkpoint, checkpoint_revision, tail, externally_dispatched, outcomes, grant, None)
}

fn reconcile_tail(
    checkpoint: &AgentExecutionCheckpoint, checkpoint_revision: u64, tail: &[AgentEngineEvent],
    externally_dispatched: &BTreeSet<ToolCallId>, outcomes: &BTreeMap<ToolCallId, AgentReconciledOutcome>,
    grant: &nomifun_agent_contracts::NativeBudgetIncrease, contracts: Option<&AgentReconciliationContracts>,
) -> Result<AgentReconciledResume, AgentEngineError> {
    checkpoint.validate()?;
    let invalid = |message: &str| AgentEngineError::ReplayContract(format!("resume reconciliation: {message}"));
    let mut next = checkpoint.clone();
    let mut step = checkpoint.model_steps;
    let mut proposed = BTreeSet::new();
    let mut order = Vec::new();
    let mut journal_call_order = Vec::new();
    let mut calls = BTreeMap::<ToolCallId, (u16, ChatToolCall)>::new();
    let mut completed = BTreeSet::new();
    let mut admitted = externally_dispatched.clone();
    let mut current_results = BTreeSet::new();
    let mut discarded = false;
    let mut new_results = 0u32;
    let mut inputs: BTreeSet<_> = checkpoint.applied_steering_receipts.iter().cloned().collect();
    let mut plan_revision_at_step = checkpoint.plan.revision;
    let mut protocol_verdicts = BTreeSet::new();
    let mut correction_sources = BTreeSet::new();
    let mut input_requests = BTreeMap::new();
    let mut live_wait_unresolved = false;
    let mut phase_correction_steps = BTreeSet::new();
    let mut delegation_calls = BTreeSet::new();
    let mut identities = BTreeMap::new();
    let mut commands = crate::workflow::CommandTracker::default();
    let mut last_plan = checkpoint.plan.clone();
    let mut last_work = checkpoint.work.clone();
    for event in tail {
        match event {
            AgentEngineEvent::CollaborationInherited { .. } =>
                return Err(invalid("collaboration inheritance must precede the original Turn entry checkpoint")),
            AgentEngineEvent::ModelStepStarted { step: current, correction_source, .. } => {
                if next.pending_user_input.is_some() { return Err(invalid("pending input handoff crossed a model boundary without accepted input")); }
                if next.awaiting_children.is_some() { return Err(invalid("pending child wait crossed a model boundary without its exact owner wake")); }
                if *current != step.checked_add(1).ok_or_else(|| invalid("model counter exhausted"))?
                    || (!discarded && !proposed.is_empty() && proposed != current_results) {
                    return Err(invalid("a model step crossed an unresolved previous batch"));
                }
                // References before this checkpoint require its canonical
                // prefix, validated by AgentTurnRecovery rather than guessed.
                if correction_source.is_some_and(|source| source >= *current
                    || (source > checkpoint.model_steps && !correction_sources.contains(&source))) {
                    return Err(invalid("correction source has no committed continuing rejection"));
                }
                step = *current; proposed.clear(); order.clear(); current_results.clear(); discarded = false;
                plan_revision_at_step = next.plan.revision;
            }
            AgentEngineEvent::ModelResponseValidated { step: current } => {
                if *current != step || discarded || !current_results.is_empty()
                    || proposed.iter().any(|id| admitted.contains(id) || !calls.contains_key(id))
                    || !protocol_verdicts.insert(*current) {
                    return Err(invalid("response validation contradicts its unadmitted proposals"));
                }
                correction_sources.clear();
            }
            AgentEngineEvent::ToolCallDelta { step: current, call_id, .. } if *current > 0 => {
                if *current != step || discarded { return Err(invalid("proposal is outside its model batch")); }
                if proposed.insert(call_id.clone()) { order.push(call_id.clone()); journal_call_order.push(call_id.clone()); }
            }
            AgentEngineEvent::ToolCallCompleted { step: current, call } => {
                if *current > 0 && (*current != step || discarded) { return Err(invalid("completed proposal has no model batch")); }
                if calls.insert(call.call_id.clone(), (*current, call.clone())).is_some() { return Err(invalid("duplicate completed call identity")); }
                if *current == 0 { journal_call_order.push(call.call_id.clone()); }
                else if proposed.insert(call.call_id.clone()) { order.push(call.call_id.clone()); journal_call_order.push(call.call_id.clone()); }
            }
            AgentEngineEvent::ToolStarted { step: current, call_id, capability_id, action_id } => {
                if calls.get(call_id).is_none_or(|(recorded, _)| recorded != current) { return Err(invalid("tool admission has no exact completed proposal")); }
                admitted.insert(call_id.clone());
                let identity = (capability_id.clone(), action_id.clone());
                if contracts.is_some_and(|contracts| !contracts.contains_key(&identity)) {
                    return Err(invalid("admitted tool has no exact frozen observation contract"));
                }
                identities.insert(call_id.clone(), identity);
                if capability_id.as_ref() == "agent.collaboration" && matches!(action_id.as_ref(), "agent/delegate"|"agent/fork") {
                    delegation_calls.insert(call_id.clone());
                }
            }
            AgentEngineEvent::ToolCompleted {step:current,result,runtime_hold:Some(crate::RuntimeToolHoldReason::PhaseRestricted)} => {
                let (recorded_step,_) = calls.get(&result.call_id).ok_or_else(|| invalid("phase correction has no proposal"))?;
                if recorded_step != current || !result.is_error || admitted.contains(&result.call_id)
                    || !completed.insert(result.call_id.clone()) {
                    return Err(invalid("phase correction differs from its unadmitted feedback"));
                }
                result.validate_for(&result.call_id)?;
                if *current>0 {
                    current_results.insert(result.call_id.clone());new_results=new_results.saturating_add(1);
                    // Runtime spends one correction per rejected model batch,
                    // independent of its call count. Feedback is not work.
                    if phase_correction_steps.insert(*current) { next.control_rejections.observe_phase_correction(); }
                }
            }
            AgentEngineEvent::ToolCompleted { step: current, result, .. }
            | AgentEngineEvent::ToolOutcomeReconciled { step: current, result, .. } => {
                let (recorded_step, call) = calls.get(&result.call_id).ok_or_else(|| invalid("result has no proposal"))?;
                if recorded_step != current || !completed.insert(result.call_id.clone()) { return Err(invalid("duplicate or mismatched result")); }
                result.validate_for(&result.call_id)?;
                if call.name == crate::collaboration::WAIT && !result.is_error {
                    next.awaiting_children = Some(next.collaboration.normalize_wait(&crate::collaboration::from_wait_call(call)?)?);
                }
                if next.collaboration.observe_tool_result(call, result, delegation_calls.contains(&result.call_id))? {
                    next.work.observe_child_workspace_change();
                }
                if *current > 0 && let Some(contracts) = contracts {
                    let attempted = admitted.contains(&result.call_id) && !matches!(event,
                        AgentEngineEvent::ToolOutcomeReconciled { source: AgentReconciliationSource::NotDispatched, .. });
                    if matches!(event, AgentEngineEvent::ToolOutcomeReconciled {source:AgentReconciliationSource::OwnerAttestation,..}) {
                        next.work.observe_child_workspace_change();
                        if result.is_error { next.work.observe_deferred(); }
                    } else {
                        observe_work(&mut next.work, &mut commands, call, result, attempted, &identities, contracts)?;
                    }
                }
                if input_requests.contains_key(&result.call_id) && result.is_error {
                    return Err(invalid("accepted native input request has a contradicting failed result"));
                }
                next.plan.apply_exact_outcome(call,result,!admitted.contains(&result.call_id)
                    || crate::exact_actions::proven_nonstart(call,result));
                if *current > 0 {
                    current_results.insert(result.call_id.clone()); new_results = new_results.saturating_add(1);
                    if admitted.contains(&result.call_id) {
                        if let Some(segments) = next.segments.as_mut() { segments.observe(call,result)?; }
                        if next.completion_review.phase == crate::AgentExecutionPhase::EndIntentReview {
                            next.completion_review.phase = crate::AgentExecutionPhase::Execution;
                        }
                    }
                    let changed = if call.name == crate::planning::TOOL_NAME { next.plan.revision != plan_revision_at_step } else { true };
                    next.control_rejections.observe(&call.name, result, changed);
                }
            }
            AgentEngineEvent::ModelOutputTruncated { step: current, discarded_tool_call_ids, .. }
            | AgentEngineEvent::ModelResponseRejected { step: current, discarded_tool_call_ids, .. }
            | AgentEngineEvent::VoiceModelStepSuperseded {step:current,discarded_tool_call_ids,..} => {
                if *current != step || !current_results.is_empty() || proposed.iter().any(|id| admitted.contains(id))
                    || proposed != discarded_tool_call_ids.iter().cloned().collect() { return Err(invalid("discard would hide an admitted effect")); }
                discarded = true;
                if let AgentEngineEvent::ModelResponseRejected { continuation, .. } = event {
                    crate::protocol_recovery::validate_rejection(event)?;
                    if !protocol_verdicts.insert(*current) { return Err(invalid("duplicate response verdict")); }
                    correction_sources.clear();
                    if *continuation { correction_sources.insert(*current); }
                } else { correction_sources.clear(); }
            }
            AgentEngineEvent::SteeringInputs { inputs: applied } => {
                correction_sources.clear();
                if !applied.is_empty() {
                    next.pending_user_input = None;
                    next.control_rejections.reset();
                }
                for input in applied {
                    input.validate()?;
                    let id = nomifun_agent_contracts::OperationId::from(input.receipt_operation_id.clone());
                    if !inputs.insert(id.clone()) { return Err(invalid("accepted input was applied twice")); }
                    next.applied_steering_receipts.push(id);
                }
                next.completion_review.align_inputs(1 + next.applied_steering_receipts.len());
            },
            AgentEngineEvent::UserInputRequested { step: current, request } => {
                let Some((source_step, call)) = calls.get(&request.request_id) else { return Err(invalid("input handoff has no exact proposal")); };
                if next.pending_user_input.is_some() || *current != step
                    || source_step != current || crate::user_input_request::from_call(call).as_ref() != Ok(request)
                    || proposed.len() != 1 || admitted.contains(&request.request_id)
                    { return Err(invalid("input handoff differs from its native source question")); }
                input_requests.insert(request.request_id.clone(), request.clone());
                next.pending_user_input = Some(request.clone());
            }
            AgentEngineEvent::TurnWaiting { .. } => {
                if next.work.running_processes.is_empty() { return Err(invalid("live wait has no original owned processes")); }
                live_wait_unresolved = true;
            }
            AgentEngineEvent::TurnWaitResolved { .. } => { live_wait_unresolved = false; },
            AgentEngineEvent::AwaitingChildren { model_steps, request } => {
                request.validate()?;
                if *model_steps != step || (next.awaiting_children.as_ref() != Some(request)
                    && next.collaboration.wait().as_ref() != Some(request)) {
                    return Err(invalid("child wait differs from its exact accepted control"));
                }
                next.awaiting_children = Some(request.clone());
            }
            AgentEngineEvent::ChildrenWaitResolved { model_steps, cursor } => {
                if *model_steps != step || next.awaiting_children.is_none() {
                    return Err(invalid("child wake has no exact pending child wait"));
                }
                next.awaiting_children.as_ref().expect("checked pending wait").validate_wake(cursor)?;
                crate::collaboration::wake_message(cursor)?;
                next.collaboration.record_wake(cursor)?;
                next.awaiting_children = None;
            }
            AgentEngineEvent::OwnedProcessObserved { observation, .. } => {
                observation.validate()?;
                next.work.observe_owned_process(observation)?;
                if let Some(segments) = next.segments.as_mut() { segments.observe_owner(observation)?; }
            }
            AgentEngineEvent::EndIntentReview { .. } => {
                if next.completion_review.is_report_only() { return Err(invalid("ending intent cannot reopen a locked completion review")); }
                next.completion_review.phase = crate::AgentExecutionPhase::EndIntentReview;
                next.completion_review.input_revision = 1 + next.applied_steering_receipts.len();
            }
            AgentEngineEvent::CompletionReview { .. } => {
                next.completion_review.phase = crate::AgentExecutionPhase::CompletionReview;
                next.completion_review.input_revision = 1 + next.applied_steering_receipts.len();
            }
            AgentEngineEvent::TurnInputRequired { request, .. } => {
                if next.pending_user_input.as_ref() != Some(request) { return Err(invalid("terminal input disposition has no pending source question")); }
            }
            AgentEngineEvent::PlanUpdated { plan } => { next.plan = plan.clone(); last_plan = plan.clone(); },
            AgentEngineEvent::WorkStatus { status } => { next.work = status.clone(); last_work = status.clone(); },
            AgentEngineEvent::PatchRecoveryUpdated { state } => next.patch_recovery = state.clone(),
            // The host separately proves the canonical Turn is nonterminal.
            // A private terminal proposal can precede a failed terminal commit.
            AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::TurnCancelled { .. } | AgentEngineEvent::TurnFailed { .. } => {},
            AgentEngineEvent::TurnStarted { .. } | AgentEngineEvent::ExecutionCheckpointSaved { .. } => return Err(invalid("a different root or checkpoint cannot be resumed")),
            _ => {}
        }
    }
    if !discarded && calls.iter().any(|(id,(call_step,call))| *call_step==step && call.name==crate::collaboration::MESSAGE
        && !completed.contains(id) && !outcomes.contains_key(id)) {
        return Err(invalid("an unpaired child message requires its exact original command owner reconciliation before any proposal can be discarded"));
    }
    let keep_batch = !current_results.is_empty() || proposed.iter().any(|id| admitted.contains(id) || input_requests.contains_key(id)
        || calls.get(id).is_some_and(|(_,call)|call.name==crate::collaboration::MESSAGE && outcomes.contains_key(id)));
    if keep_batch && proposed.iter().any(|id| !calls.contains_key(id)) { return Err(invalid("admitted batch contains incomplete arguments")); }
    let mut observations = Vec::new();
    for id in &journal_call_order {
        let Some((call_step,call)) = calls.get(id) else { continue; };
        if completed.contains(id) {continue;}
        if *call_step > 0 && (!keep_batch || *call_step != step) {
            if !admitted.contains(id) {
                let held=AgentToolResult::text(id.clone(),"Not dispatched: the fenced journal has no owner admission for this reserved exact action.",true);
                next.plan.apply_exact_outcome(call,&held,true);
            }
            continue;
        }
        let outcome = if input_requests.contains_key(id) {
            AgentReconciledOutcome { result: AgentToolResult::text(id.clone(), crate::user_input_request::ACK, false),
                source: AgentReconciliationSource::NativeControlReceipt, evidence_event_id: None, owner_operation_id: None }
        } else if let Some(outcome) = outcomes.get(id) {
            outcome.clone()
        } else if call.name == crate::collaboration::MESSAGE {
            return Err(invalid("the original child-message owner has no reconciled command receipt; absence of a Kernel admission cannot prove this internal control was not applied"));
        } else if !admitted.contains(id) {
            AgentReconciledOutcome { result: AgentToolResult::text(id.clone(), "Not dispatched: the paused execution never admitted this proposal. Reconsider under the current task before any new call.", true),
                source: AgentReconciliationSource::NotDispatched, evidence_event_id: None, owner_operation_id: None }
        } else { return Err(invalid("admitted invocation has no owner-backed reconciliation outcome")); };
        outcome.result.validate_for(id)?;
        if call.name == crate::collaboration::MESSAGE
            && (!matches!(outcome.source, AgentReconciliationSource::NativeControlReceipt
                | AgentReconciliationSource::OwnerReceipt | AgentReconciliationSource::OwnerAttestation)
                || outcome.evidence_event_id.as_deref().is_none_or(str::is_empty)
                || outcome.owner_operation_id.as_ref().is_none_or(|id|id.as_ref().is_empty())) {
            return Err(invalid("a child message requires the exact command owner's receipt or fenced attestation; NotDispatched is not an internal-control outcome proof"));
        }
        if call.name == crate::collaboration::MESSAGE && outcome.result.is_error {
            let proof:serde_json::Value=serde_json::from_str(&outcome.result.output_text())
                .map_err(|_|invalid("child message non-application needs a typed owner proof"))?;
            if proof["state"]!="not_executed" || proof["control_applied"]!=false || proof["code"]!="CHILD_MESSAGE_NOT_COMMITTED" {
                return Err(invalid("unknown child guidance is not proof that no command was committed"));
            }
            next.work.observe_deferred();
        }
        if next.collaboration.observe_tool_result(call, &outcome.result, delegation_calls.contains(id))? {
            next.work.observe_child_workspace_change();
        }
        if *call_step > 0 && let Some(contracts) = contracts {
            let attempted = admitted.contains(id) && !matches!(outcome.source, AgentReconciliationSource::NotDispatched);
            // Internal guidance failures were already accounted above.
            if matches!(outcome.source, AgentReconciliationSource::OwnerAttestation) {
                next.work.observe_child_workspace_change();
                if outcome.result.is_error { next.work.observe_deferred(); }
            } else if call.name != crate::collaboration::MESSAGE || !outcome.result.is_error {
                observe_work(&mut next.work, &mut commands, call, &outcome.result, attempted, &identities, contracts)?;
            }
        }
        if admitted.contains(id) && matches!(outcome.source, AgentReconciliationSource::NotDispatched)
            && outcome.evidence_event_id.is_none() { return Err(invalid("missing dispatch needs a fenced journal witness")); }
        next.plan.apply_exact_outcome(call,&outcome.result,
            matches!(outcome.source,AgentReconciliationSource::NotDispatched)
                ||crate::exact_actions::proven_nonstart(call,&outcome.result));
        if *call_step > 0 { new_results = new_results.saturating_add(1); }
        if *call_step > 0 && admitted.contains(id) {
            if let Some(segments) = next.segments.as_mut() { segments.observe(call,&outcome.result)?; }
            if next.completion_review.phase == crate::AgentExecutionPhase::EndIntentReview {
                next.completion_review.phase = crate::AgentExecutionPhase::Execution;
            }
        }
        observations.extend([AgentEngineEvent::ToolOutcomeReconciled { step: *call_step, result: outcome.result,
            source: outcome.source, evidence_event_id: outcome.evidence_event_id, owner_operation_id: outcome.owner_operation_id }]);
    }
    if live_wait_unresolved {
        if !next.work.running_processes.is_empty() {
            return Err(invalid("the original live wait owner has no reaped terminal observation; keep this Turn paused for reconciliation instead of republishing its old user-input request"));
        }
        // Resolve only the machine observation boundary. A process result
        // cannot resolve an independent question or manufacture user input.
        observations.push(AgentEngineEvent::TurnWaitResolved { model_steps: step });
    }
    let discard_last = !keep_batch;
    next.plan.needs_replan = true;
    crate::exact_actions::close_receivers(&mut next.plan.exact_actions);
    if next.plan != last_plan {
        observations.push(AgentEngineEvent::PlanUpdated {plan:next.plan.clone()});
    }
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
    next.work.running_processes.clear(); next.work.recent_commands.clear();
    next.work.command_observed_after_latest_mutation = false;
    if next.work != last_work { observations.push(AgentEngineEvent::WorkStatus {status:next.work.clone()}); }
    if grant.retry_stall_guards { next.control_rejections.reset(); }
    if let Some(segments) = next.segments.as_mut() { segments.authorize_resume(grant, step)?; }
    else if grant.additional_segments != 0 { return Err(invalid("no segmented model budget exists at this checkpoint")); }
    next.validate()?;
    Ok(AgentReconciledResume { checkpoint: next, observations })
}

fn observe_work(work: &mut crate::AgentWorkStatus, commands: &mut crate::workflow::CommandTracker,
    call: &ChatToolCall, result: &AgentToolResult, attempted: bool,
    identities: &BTreeMap<ToolCallId,(nomifun_agent_contracts::CapabilityId,nomifun_agent_contracts::ActionId)>,
    contracts: &AgentReconciliationContracts) -> Result<(),AgentEngineError> {
    if attempted {
        let identity = identities.get(&call.call_id).ok_or_else(||
            AgentEngineError::ReplayContract("dispatched result has no canonical tool identity".into()))?;
        let effect = contracts.get(identity).ok_or_else(||
            AgentEngineError::ReplayContract("dispatched result lacks its frozen effect contract".into()))?;
        work.observe_contract(identity.0.as_ref(),identity.1.as_ref(),*effect,call,result,commands);
    } else if result.is_error {
        work.observe_deferred();
    }
    Ok(())
}

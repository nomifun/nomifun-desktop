//! Owner-facing pause/reconcile/resume. No model call or tool replay occurs
//! while preparing a resume; the Store commits the exact prepared boundary.
use super::*;
use nomifun_agent_runtime::{AgentExecutionCheckpoint, AgentReconciledOutcome, AgentReconciliationSource, AgentToolResult};
use nomifun_agent_session::{AgentEffectState, NativeEffectReconciliationRequest, NativeResumePreparation, NativeResumeRequest};
use nomifun_chat_model_broker::ToolCallId;
#[path = "native_resume_waiter.rs"]
mod native_resume_waiter;

#[cfg(test)]
type AttachmentFailure=(AppError,Arc<tokio::sync::Notify>);
#[cfg(test)]
static ATTACHMENT_FAILURES:std::sync::OnceLock<std::sync::Mutex<BTreeMap<(String,String),AttachmentFailure>>>=std::sync::OnceLock::new();

#[cfg(test)]
pub(in crate::router) async fn prepare_resume_for_test(owner:&NomiCoreSessionOwner,
    principal:&PrincipalRef,session:&AgentSessionId,request:&NativeResumeRequest)
    ->Result<NativeResumePreparation,AppError> {
    owner.prepare_native_resume(principal,session,request).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PauseRequest {
    operation_id: OperationId,
    idempotency_key: String,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EffectQuery { operation_id: OperationId }

pub(super) async fn pause(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>,
    Path(id): Path<String>, Json(request): Json<PauseRequest>,
) -> Result<Json<ApiResponse<nomifun_agent_contracts::SessionEventAck>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&id)?;
    let _operation_guard = state.session_owner.session_operation_lock(session.as_ref()).write_owned().await;
    let principal = authenticated_principal(&owner);
    let store = state.session_owner.canonical().store();
    store.inspect_latest_native_execution(&principal,&session).await.map_err(agent_session_store_error)?;
    if store.native_execution_deadline(&session,&request.operation_id).await.map_err(agent_session_store_error)? <= now_ms() {
        let facts = store.chat_causality_facts(&session,&request.operation_id).await.map_err(agent_session_store_error)?;
        if facts.session.owner_ref != principal { return Err(AppError::Forbidden("execution belongs to another owner".into()).into()); }
        let _ = store.quarantine_native_recovery(&principal,&session,&request.operation_id,facts.execution_fence).await.map_err(agent_session_store_error)?;
    }
    let ack = store.request_native_pause(&principal, &session,
        &request.operation_id, &request.idempotency_key, &request.reason).await.map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::ok(ack)))
}

pub(super) async fn effects(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>,
    Path(id): Path<String>, Query(query): Query<EffectQuery>,
) -> Result<Json<ApiResponse<Value>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&id)?;
    let store = state.session_owner.canonical().store();
    let candidates = store.native_reconciliation_candidates(&authenticated_principal(&owner), &session, &query.operation_id)
        .await.map_err(agent_session_store_error)?;
    let facts = store.chat_causality_facts(&session,&query.operation_id).await.map_err(agent_session_store_error)?;
    let mut calls = BTreeMap::new();
    let mut dispatched = BTreeMap::new();
    let mut observed = BTreeSet::new();
    let mut returned = BTreeSet::new();
    for event in facts.events.iter().filter(|event| event.kind.0 == "runtime/progress-recorded" && event.correlation_id.as_ref() == query.operation_id.as_ref()) {
        let Some(value) = facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event")) else { continue; };
        match value.get("event").and_then(Value::as_str) {
            Some("tool_call_completed") => if let (Some(id),Some(arguments)) = (value.pointer("/call/call_id").and_then(Value::as_str),value.pointer("/call/arguments")) {
                calls.insert(id.to_owned(),digest_payload(arguments).map_err(|error|AppError::Conflict(error.to_string()))?);
            },
            Some("tool_completed"|"tool_outcome_reconciled") => if let Some(id) = value.pointer("/result/call_id").and_then(Value::as_str) { observed.insert(id.to_owned()); },
            Some("host_tool_dispatch"|"host_resource_dispatch") => {
                let intent = value.get("dispatch").unwrap_or(value);
                if let (Some(id),Some(operation)) = (intent.get("call_id").and_then(Value::as_str),intent.get("operation_id").and_then(Value::as_str)) {
                    dispatched.insert(id.to_owned(),operation.to_owned());
                }
            }
            Some("host_tool_settled"|"host_resource_settled") => if let Some(operation) = value.get("operation_id").and_then(Value::as_str) { returned.insert(operation.to_owned()); },
            _ => {}
        }
    }
    let missing: Vec<_> = dispatched.iter().filter(|(id,_)|!observed.contains(*id)).take(256).map(|(id,operation)|json!({
        "call_id":id,"operation_id":operation,"expected_input_digest":calls.get(id),
        "owner_receipt_available":returned.contains(operation),"not_permission_to_repeat":true,
    })).collect();
    Ok(Json(ApiResponse::ok(json!({"items":candidates.items,"has_more":candidates.has_more,
        "unresolved_invocations":missing,"automatic_replay_authorized":false}))))
}

pub(super) async fn reconcile(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>,
    Path(id): Path<String>, Json(request): Json<NativeEffectReconciliationRequest>,
) -> Result<Json<ApiResponse<nomifun_agent_contracts::SessionEventAck>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&id)?;
    let _operation_guard = state.session_owner.session_operation_lock(session.as_ref()).write_owned().await;
    let ack = state.session_owner.canonical().store().reconcile_native_effect_by_owner(
        &authenticated_principal(&owner), &session, &request).await.map_err(agent_session_store_error)?;
    state.session_owner.schedule_native_settlement_recovery().await?;
    state.session_owner.schedule_session_input_drain(owner.as_ref().to_owned(),session,None);
    Ok(Json(ApiResponse::ok(ack)))
}

pub(in crate::router) async fn resume(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>,
    Path(id): Path<String>, Json(request): Json<NativeResumeRequest>,
) -> Result<Json<ApiResponse<nomifun_agent_session::NativeResumeCommandReceipt>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&id)?;
    let principal = authenticated_principal(&owner);
    let receipt=state.session_owner.submit_native_resume(principal,session,request).await?;
    Ok(Json(ApiResponse::ok(receipt)))
}

impl NomiCoreSessionOwner {
    /// One owner command boundary shared by the native control endpoint and
    /// the authenticated logical-task restart command. Preparation itself
    /// never runs business work or changes the original checkpoint identity.
    pub(super) async fn submit_native_resume(self:&Arc<Self>, principal:PrincipalRef,
        session:AgentSessionId, request:NativeResumeRequest)
        ->Result<nomifun_agent_session::NativeResumeCommandReceipt,AppError> {
    let _operation_guard = self.session_operation_lock(session.as_ref()).write_owned().await;
    let store = self.canonical().store();
    if let Some(receipt) = store.native_resume_request_receipt(&principal,&session,&request).await.map_err(agent_session_store_error)? {
        self.enqueue_requested_native_resume(principal,session,request)?;
        return Ok(nomifun_agent_session::NativeResumeCommandReceipt::Queued {receipt});
    }
    let receipt = if let Some(receipt) = store.native_resume_receipt(&principal,&session,&request).await.map_err(agent_session_store_error)? {
        receipt
    } else {
        match self.prepare_native_resume(&principal,&session,&request).await {
            Ok(prepared) => match store.commit_native_resume(&principal,&session,&request,prepared).await {
                Ok(receipt) => receipt,
                Err(nomifun_agent_session::SessionStoreError::ExecutionCapacityPending) => {
                    let receipt = store.request_native_resume(&principal,&session,&request).await.map_err(agent_session_store_error)?;
                    self.enqueue_requested_native_resume(principal,session,request)?;
                    return Ok(nomifun_agent_session::NativeResumeCommandReceipt::Queued {receipt});
                }
                Err(error) => return Err(agent_session_store_error(error)),
            },
            Err(error) => {
                // A concurrent identical authorization may have changed the
                // checkpoint while this read-only preparation was in flight.
                // Only its exact authenticated receipt makes this a replay.
                match store.native_resume_receipt(&principal,&session,&request).await.map_err(agent_session_store_error)? {
                    Some(receipt) => receipt,
                    None => return Err(error),
                }
            }
        }
    };
    self.enqueue_authorized_native_resume(session,request.operation_id,receipt.seq).await?;
    Ok(nomifun_agent_session::NativeResumeCommandReceipt::Authorized {receipt})
    }
}

impl NomiCoreSessionOwner {
    pub(in crate::router) async fn require_ordinary_owner_input(&self,owner:&PrincipalRef,session:&AgentSessionId) -> Result<(),AppError> {
        let observed=self.canonical.get(owner,session).await?;
        let attempt:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE conversation_id=? AND active=1 AND relation IN ('attempt','automation'))")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if observed.session.metadata.purpose != nomifun_agent_contracts::SessionPurpose::Conversation
            || observed.session.remote_binding_provenance.is_some() || attempt
            || self.is_companion_associated_task(&owner.principal_id,session).await? {
            return Err(AppError::Conflict("This Session's inputs require their exact domain authority".into()));
        }
        Ok(())
    }

    async fn prepare_native_resume(&self, owner: &PrincipalRef, session: &AgentSessionId,
        request: &NativeResumeRequest) -> Result<NativeResumePreparation, AppError> {
        let reject = |message: &str| AppError::Conflict(format!("Native resume: {message}"));
        let store = self.canonical.store();
        let saved = store.load_native_checkpoint(owner,session,&request.operation_id).await.map_err(agent_session_store_error)?
            .ok_or_else(|| reject("there is no recoverable checkpoint; a terminal/uninitialized task is not reopened"))?;
        let facts = store.native_recovery_facts(session,&request.operation_id).await.map_err(agent_session_store_error)?;
        let pause = store.native_pause_state(session,&request.operation_id).await.map_err(agent_session_store_error)?
            .ok_or_else(|| reject("execution is not paused"))?;
        if facts.session.owner_ref != *owner || facts.head.status != "paused" || saved.turn_state != "running"
            || saved.revision != request.expected_checkpoint_revision || saved.digest != request.expected_checkpoint_digest
            || pause.revision != request.expected_pause_revision {
            return Err(reject("pause/checkpoint/owner changed"));
        }
        if pause.reason.contains("NO_PROGRESS") && !request.budget.retry_stall_guards {
            return Err(reject("the owner must explicitly acknowledge retrying the stalled execution"));
        }
        if !pause.cleanup_proven { request.cleanup_attestation.as_ref().ok_or_else(|| reject("resource cleanup still needs owner verification"))?
            .validate().map_err(agent_session_store_error)?; }
        if store.has_unsettled_effects(session).await.map_err(agent_session_store_error)? {
            return Err(reject("reconcile every pending/unknown effect before authorizing continuation"));
        }
        let checkpoint: AgentExecutionCheckpoint = serde_json::from_value(saved.state.0.clone()).map_err(|error| reject(&error.to_string()))?;
        checkpoint.validate().map_err(|error| reject(&error.to_string()))?;
        let installed = self.official_runtime.get().ok_or_else(|| reject("runtime is not installed"))?.binding()?;
        if checkpoint.binding.agent_session_id() != session || checkpoint.turn_operation_id != request.operation_id
            || checkpoint.binding.resolved_snapshot_ref() != &facts.session.agent_binding.resolved_snapshot_ref
            || checkpoint.active_set_generation != facts.head.active_set_generation
            || checkpoint.binding.build_id().as_ref() != installed.build_id || checkpoint.binding.build_digest().as_ref() != installed.build_digest {
            return Err(reject("resume cannot change build, Snapshot, capability generation or task identity"));
        }
        let control_plane = self.runtime_control_plane.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| reject("the original Snapshot reader is unavailable"))?;
        let binding = agent_binding_dto(&facts.session.agent_binding).map_err(|error|AppError::Conflict(error.message))?;
        let (_, _, snapshot) = control_plane.saved_binding_artifacts(
            &UserId::from(owner.principal_id.clone()), &binding)
            .await.map_err(control_plane_error_to_app)?;
        if snapshot.snapshot_ref != facts.session.agent_binding.resolved_snapshot_ref {
            return Err(reject("frozen observation contracts differ from the original Snapshot"));
        }
        let mut contracts = nomifun_agent_runtime::AgentReconciliationContracts::new();
        for capability in &snapshot.content.enabled_capabilities {
            for action in capability.actions.iter().filter(|action| capability.action_allowlist.contains(&action.action_id)) {
                let identity = (capability.capability.id.clone(), action.action_id.clone());
                let effect = nomifun_engine_core::engine_effect(action.effect_class).0;
                if contracts.insert(identity, effect).is_some() {
                    return Err(reject("frozen observation contracts contain duplicate Action identities"));
                }
            }
        }
        let effect_rows = store.list_effects(session).await.map_err(agent_session_store_error)?;
        let effects: Vec<_> = effect_rows.iter().filter(|effect| effect.turn_id == request.operation_id).collect();
        let mut tail = Vec::new();
        let mut sequence = 0u64;
        let mut bytes = 0usize;
        let mut intents = BTreeMap::<ToolCallId,(OperationId,String)>::new();
        let mut owner_results = BTreeMap::<ToolCallId,AgentReconciledOutcome>::new();
        let mut native_started = BTreeMap::<ToolCallId,(nomifun_agent_contracts::CapabilityId,nomifun_agent_contracts::ActionId)>::new();
        let mut attestations = BTreeMap::<String,(&Value,String)>::new();
        let pause_event = facts.events.iter().rev().find(|event| event.kind.0 == "turn/paused" && event.correlation_id.as_ref() == request.operation_id.as_ref())
            .ok_or_else(|| reject("pause event missing"))?.event_id.as_ref().to_owned();
        for event in &facts.events {
            if event.seq > saved.through_seq && event.kind.0 == "runtime/effect-reconciliation-attested" && event.correlation_id.as_ref() == request.operation_id.as_ref() {
                let payload = facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(|| reject("attestation payload missing"))?;
                if let Some(id) = payload.get("call_id").or_else(|| payload.get("effect_id")).and_then(Value::as_str) {
                    attestations.insert(id.to_owned(),(payload,event.event_id.as_ref().to_owned()));
                }
            }
            if event.kind.0 != "runtime/progress-recorded" || event.correlation_id.as_ref() != request.operation_id.as_ref() { continue; }
            let payload = facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(|| reject("runtime payload missing"))?;
            sequence += 1;
            if payload.get("producer_seq").and_then(Value::as_u64) != Some(sequence) { return Err(reject("runtime producer sequence is incomplete")); }
            bytes = bytes.saturating_add(serde_json::to_vec(payload).map_err(|error| reject(&error.to_string()))?.len() + 256);
            if bytes > nomifun_agent_contracts::MAX_NATIVE_APPROVED_REPLAY_BYTES { return Err(reject("resume journal exceeds its bounded reader")); }
            if event.seq <= saved.through_seq { continue; }
            let value = payload.get("event").ok_or_else(|| reject("runtime event missing"))?;
            match value.get("event").and_then(Value::as_str) {
                Some("host_tool_dispatch" | "host_resource_dispatch") => {
                    let dispatch = value.get("dispatch").unwrap_or(value);
                    let id = dispatch.get("call_id").and_then(Value::as_str).ok_or_else(|| reject("dispatch call identity missing"))?;
                    let operation = dispatch.get("operation_id").and_then(Value::as_str).ok_or_else(|| reject("dispatch operation identity missing"))?;
                    if intents.insert(id.into(),(operation.into(),event.event_id.as_ref().to_owned())).is_some() { return Err(reject("duplicate dispatch intent")); }
                }
                Some("host_tool_settled") => {
                    let id: ToolCallId = value.get("call_id").and_then(Value::as_str).ok_or_else(|| reject("settled call identity missing"))?.into();
                    let result = if let Some(result) = value.get("result").filter(|value| !value.is_null()) {
                        serde_json::from_value::<AgentToolResult>(result.clone()).map_err(|error| reject(&error.to_string()))?
                    } else {
                        let error = value.get("error").and_then(Value::as_str).ok_or_else(|| reject("owner settlement has neither result nor error"))?;
                        AgentToolResult::text(id.clone(),format!("Original owner returned an error: {error}"),true)
                    };
                    result.validate_for(&id).map_err(|error| reject(&error.to_string()))?;
                    owner_results.insert(id,AgentReconciledOutcome { result,source:AgentReconciliationSource::OwnerReceipt,
                        evidence_event_id:Some(event.event_id.as_ref().to_owned()), owner_operation_id:value.get("operation_id").and_then(Value::as_str).map(Into::into) });
                }
                Some("host_resource_settled") => {
                    let operation = value.get("operation_id").and_then(Value::as_str).ok_or_else(|| reject("resource settlement operation missing"))?;
                    let id = intents.iter().find(|(_, (known,_))| known.as_ref() == operation).map(|(id,_)|id.clone())
                        .ok_or_else(|| reject("resource settlement has no dispatch"))?;
                    let returned = value.get("owner_returned").and_then(Value::as_bool).ok_or_else(|| reject("resource settlement state missing"))?;
                    owner_results.insert(id.clone(),AgentReconciledOutcome { result:AgentToolResult::text(id,
                        "Historical resource-owner return observed; the response body is not retained here. Obtain a fresh authorized observation if needed.",!returned),
                        source:AgentReconciliationSource::OwnerReceipt,evidence_event_id:Some(event.event_id.as_ref().to_owned()),owner_operation_id:Some(operation.into()) });
                }
                Some("host_process_dispatch" | "host_process_quiescent" | "host_cleanup_proven") => {},
                _ => {
                    let native: AgentEngineEvent = serde_json::from_value(value.clone()).map_err(|error| reject(&error.to_string()))?;
                    if let AgentEngineEvent::ToolStarted { call_id,capability_id,action_id,.. } = &native {
                        native_started.insert(call_id.clone(),(capability_id.clone(),action_id.clone()));
                    }
                    tail.push(native);
                }
            }
        }
        let dispatched: BTreeSet<_> = intents.keys().cloned().collect();
        for (call_id,(operation,witness)) in &intents {
            if owner_results.contains_key(call_id) { continue; }
            let delegation = native_started.get(call_id).is_some_and(|(capability,action)|
                capability.as_ref() == "agent.collaboration" && matches!(action.as_ref(),"agent/delegate"|"agent/fork"));
            if !delegation && let Some((attestation,event_id)) = attestations.get(call_id.as_ref()) {
                let succeeded = attestation.get("outcome").and_then(Value::as_str) == Some("confirmed_succeeded");
                owner_results.insert(call_id.clone(),AgentReconciledOutcome { result:AgentToolResult::text(call_id.clone(),
                    "Session owner explicitly verified this prior invocation. This attestation is not a fresh tool result; inspect current state before completion.",!succeeded),
                    source:AgentReconciliationSource::OwnerAttestation,evidence_event_id:Some(event_id.clone()),owner_operation_id:Some(operation.clone()) });
                continue;
            }
            let owned: Vec<_> = effects.iter().filter(|effect| effect.operation_id == *operation).collect();
            if !owned.is_empty() {
                if owned.iter().any(|effect| matches!(effect.state,AgentEffectState::Pending|AgentEffectState::Unknown)) { return Err(reject("effect outcome is still unknown")); }
                let attested = owned.iter().find_map(|effect| attestations.get(&effect.effect_id));
                if owned.len() != 1 { return Err(reject("the invocation has no unique exact Effect result")); }
                let effect = owned[0];
                let identity = native_started.get(call_id).ok_or_else(|| reject("Effect has no original native Action admission"))?;
                if effect.capability_module != identity.0 || effect.action_id != identity.1 {
                    return Err(reject("Effect differs from the original native Action admission"));
                }
                store.committed_tool_call_causation(session,&request.operation_id,operation,
                    &effect.capability_module,&effect.action_id).await.map_err(agent_session_store_error)?;
                let result = original_effect_result(effect,call_id).map_err(|error| reject(&error))?;
                if delegation && !result.is_error {
                    self.verify_recovered_delegation(owner, effect, &result).await?;
                }
                owner_results.insert(call_id.clone(),AgentReconciledOutcome { result,
                    source:if attested.is_some() { AgentReconciliationSource::OwnerAttestation } else { AgentReconciliationSource::OwnerReceipt },
                    evidence_event_id:attested.map(|(_,id)|id.clone()).or_else(||owned[0].terminal_event_id.as_ref().map(|id|id.as_ref().to_owned())),owner_operation_id:Some(operation.clone()) });
            } else if native_started.get(call_id).is_some_and(|(_,action)| action.as_ref() == "workspace.files/read") {
                owner_results.insert(call_id.clone(),AgentReconciledOutcome { result:AgentToolResult::text(call_id.clone(),
                    "The read outcome was not durably observed; it was not repeated during reconciliation. Perform a fresh authorized read if needed.",true),
                    source:AgentReconciliationSource::ReadOutcomeUnavailable,evidence_event_id:Some(witness.clone()),owner_operation_id:Some(operation.clone()) });
            }
        }
        for (id,_) in &native_started {
            if !intents.contains_key(id) {
                owner_results.insert(id.clone(),AgentReconciledOutcome { result:AgentToolResult::text(id.clone(),
                    "No host dispatch occurred before the canonical pause fence. The proposal was not executed.",true),source:AgentReconciliationSource::NotDispatched,
                    evidence_event_id:Some(pause_event.clone()),owner_operation_id:None });
            }
        }
        // Internal child guidance has a real durable domain owner even though
        // it does not dispatch an OS Action. Absence of a Kernel Effect alone
        // cannot classify it as untouched after a lost Runtime reply.
        let completed_controls=tail.iter().filter_map(|event|match event {
            AgentEngineEvent::ToolCompleted {result,..}|AgentEngineEvent::ToolOutcomeReconciled {result,..}=>Some(result.call_id.clone()),
            _=>None,
        }).collect::<BTreeSet<_>>();
        for event in &tail {
            let AgentEngineEvent::ToolCallCompleted {call,..}=event else {continue;};
            if call.name!="agent_message"||completed_controls.contains(&call.call_id)||owner_results.contains_key(&call.call_id) {continue;}
            let command:nomifun_agent_runtime::AgentCollaborationMessage=serde_json::from_value(call.arguments.0.clone())
                .map_err(|_|reject("the original child-message command is not readable"))?;
            let engine=self.collaboration_engine()?;
            let known=engine.message_receipt(&owner.principal_id,session.as_ref(),request.operation_id.as_ref(),&command).await?;
            let (body,failed,witness,operation)=if let Some(receipt)=known {
                let witness=format!("agent-execution-event:{}:{}",receipt.execution_id,receipt.sequence);
                let operation=receipt.operation_id.clone();
                let mut body=serde_json::to_value(receipt).map_err(|error|reject(&error.to_string()))?;
                body["accepted"]=serde_json::json!(true);
                (body,false,witness,operation)
            } else {
                // Preparation requires the exact committed pause fence.
                // Every new message write checks that producer's original
                // native epoch in the writer, so it cannot commit afterward.
                (serde_json::json!({"state":"not_executed","control_applied":false,
                    "code":"CHILD_MESSAGE_NOT_COMMITTED"}),true,pause_event.clone(),
                    format!("{}:internal-message:{}",request.operation_id.as_ref(),call.call_id.as_ref()))
            };
            owner_results.insert(call.call_id.clone(),AgentReconciledOutcome {
                result:AgentToolResult::text(call.call_id.clone(),body.to_string(),failed),
                source:AgentReconciliationSource::NativeControlReceipt,evidence_event_id:Some(witness),
                owner_operation_id:Some(operation.into()),
            });
        }
        let mut reconciled = nomifun_agent_runtime::reconcile_execution_tail_with_contracts(
            &checkpoint,saved.revision,&tail,&dispatched,&owner_results,&request.budget,&contracts)
            .map_err(|error| reject(&error.to_string()))?;
        let mut notices = Vec::new();
        for (payload,event_id) in attestations.values() {
            if notices.len() >= 64 { return Err(reject("too many owner reconciliation notices for one resume")); }
            notices.push(AgentEngineEvent::OwnerOutcomeReconciled {
                call_id:payload.get("call_id").and_then(Value::as_str).map(Into::into),
                effect_id:payload.get("effect_id").and_then(Value::as_str).map(str::to_owned),
                outcome:payload.get("outcome").and_then(Value::as_str).unwrap_or("verified_outcome").to_owned(),
                evidence_event_id:event_id.clone(),source:AgentReconciliationSource::OwnerAttestation,
            });
        }
        for event in facts.events.iter().filter(|event| event.seq > saved.through_seq && event.kind.0 == "effect/reconciled") {
            let Some(payload) = facts.event_payloads.get(event.event_id.as_ref()) else { continue; };
            if payload.get("turn_id").and_then(Value::as_str) != Some(request.operation_id.as_ref()) { continue; }
            let effect_id = payload.get("effect_id").and_then(Value::as_str).ok_or_else(||reject("reconciled owner effect identity missing"))?;
            if attestations.contains_key(effect_id) { continue; }
            if notices.len() >= 64 { return Err(reject("too many owner reconciliation notices for one resume")); }
            notices.push(AgentEngineEvent::OwnerOutcomeReconciled { call_id:None,effect_id:Some(effect_id.to_owned()),
                outcome:payload.get("outcome").and_then(Value::as_str).unwrap_or("owner_reconciled").to_owned(),
                evidence_event_id:event.event_id.as_ref().to_owned(),source:AgentReconciliationSource::OwnerReceipt });
        }
        notices.append(&mut reconciled.observations);
        reconciled.observations = notices;
        let inspection = store.inspect_latest_native_execution(owner,session).await.map_err(agent_session_store_error)?.ok_or_else(||reject("execution missing"))?;
        let budget = inspection.budget.increased(&request.budget).map_err(reject)?;
        if bytes as u64 >= budget.journal_bytes.saturating_sub(4 * 1024 * 1024)
            || sequence >= budget.journal_records.saturating_sub(12_000)
            || inspection.session_payload_bytes >= budget.session_payload_bytes.saturating_sub(4 * 1024 * 1024) {
            return Err(reject("explicit allowance is still insufficient for the next execution window"));
        }
        Ok(NativeResumePreparation { expected_head_seq:facts.head.last_seq, expected_fence:saved.execution_fence,
            snapshot:facts.session.agent_binding.resolved_snapshot_ref,active_set_generation:facts.head.active_set_generation,
            checkpoint_state:StrictJsonValue(serde_json::to_value(reconciled.checkpoint).map_err(|error|reject(&error.to_string()))?),
            observations:reconciled.observations.into_iter().map(|event|serde_json::to_value(event).map(StrictJsonValue)).collect::<Result<Vec<_>,_>>().map_err(|error|reject(&error.to_string()))? })
    }

    async fn verify_recovered_delegation(&self, owner: &PrincipalRef,
        effect: &nomifun_agent_session::AgentEffectRecord, result: &AgentToolResult) -> Result<(),AppError> {
        let reject = |message:&str| AppError::Conflict(format!("Native delegation recovery: {message}"));
        if effect.owner_domain != "wave5.product" || effect.state != AgentEffectState::Returned {
            return Err(reject("the original delegation Effect has no returned owner result"));
        }
        let receipt: nomifun_common::AgentExecutionReceipt = serde_json::from_str(&result.output_text())
            .map_err(|_| reject("the original delegation has no typed Execution receipt"))?;
        let observation = self.collaboration_engine()?.collaboration_observation(
            &owner.principal_id, &receipt.execution_id, None).await?;
        let expected = nomifun_common::AgentExecutionSource {
            agent_session_id:effect.agent_session_id.as_ref().into(), turn_operation_id:effect.turn_id.as_ref().into(),
            tool_operation_id:effect.operation_id.as_ref().into(), effect_id:effect.effect_id.clone(),
        };
        let mut sources = Vec::new();
        if let Some(source) = nomifun_agent_execution::execution_source_from_initial_plan(&observation.execution.initial_plan_input)? {
            sources.push(source);
        }
        let state = observation.state.ok_or_else(|| reject("delegation source snapshot is unavailable"))?;
        for event in &state.events {
            if let Some(source) = event.payload.get("delegation_source") {
                sources.push(serde_json::from_value(source.clone()).map_err(|_| reject("delegation source is malformed"))?);
            }
        }
        if !sources.iter().any(|source| source == &expected)
            || receipt.step_ids.iter().any(|id| !state.steps.iter().any(|step| &step.step_id == id)) {
            return Err(reject("the original Effect receipt differs from its domain source"));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::router) fn inject_authorized_native_resume_attachment_error(session:&str,stage:&str,error:AppError)->Arc<tokio::sync::Notify> {
        let reached=Arc::new(tokio::sync::Notify::new());
        ATTACHMENT_FAILURES.get_or_init(Default::default).lock().unwrap().insert((session.into(),stage.into()),(error,reached.clone()));
        reached
    }

    #[cfg(test)]
    pub(super) async fn authorized_resume_attachment_cutpoint(&self,session:&AgentSessionId,stage:&str)->Result<(),AppError> {
        let failure=ATTACHMENT_FAILURES.get_or_init(Default::default).lock().unwrap().remove(&(session.as_ref().into(),stage.into()));
        if let Some((error,reached))=failure {reached.notify_one();return Err(error);}
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::router) async fn join_authorized_native_resume_for_test(&self,session:&AgentSessionId,operation:&OperationId,generation:u64) {
        let _joined=self.session_operation_lock(&format!("native-resume-attach:{}:{}:{generation}",session.as_ref(),operation.as_ref())).write_owned().await;
    }

    pub(in crate::router) async fn enqueue_authorized_native_resume(self: &Arc<Self>, session: AgentSessionId, operation: OperationId, authorization_generation: u64) -> Result<(),AppError> {
        let engines = self.native_engines.get().cloned().ok_or_else(||AppError::Conflict("native recovery scheduler is not installed".into()))?;
        let key=format!("native-resume-attach:{}:{}:{authorization_generation}",session.as_ref(),operation.as_ref());
        let Ok(flight)=self.session_operation_lock(&key).try_write_owned() else {return Ok(());};
        let owner = Arc::downgrade(self);
        let tasks=self.background_tasks.clone();
        if !self.background_tasks.spawn(Box::pin(async move {
            let _flight=flight;
            let mut preparation_rejected=false;
            loop {
                let (Some(owner),Some(engines)) = (owner.upgrade(),engines.upgrade()) else { return; };
                let result=async {
                    let store=owner.canonical.store();
                    let receipt=match store.read_turn_receipt(&session,&operation).await {
                        Err(nomifun_agent_session::SessionStoreError::Deleted(_)|nomifun_agent_session::SessionStoreError::NotFound(_))=>return Ok(true),
                        result=>result.map_err(agent_session_store_error)?,
                    };
                    if receipt.status!=nomifun_agent_session::TurnReceiptStatus::Running {return Ok(true);}
                    #[cfg(test)]
                    owner.authorized_resume_attachment_cutpoint(&session,"generation").await?;
                    let generation=store.native_execution_generation(&session,&operation).await.map_err(agent_session_store_error)?;
                    if store.native_pause_state(&session,&operation).await.map_err(agent_session_store_error)?.is_some() {return Ok(true);}
                    if engines.owns_attached_recovery(&session,&operation,generation).await {return Ok(true);}
                    if generation!=authorization_generation||preparation_rejected {
                        // Claim is not attachment. A failed loader may have
                        // changed the generation, and its reply may be lost.
                        // Only inspect that exact claim; never invoke recovery
                        // again with another generation or another input.
                        let facts=store.native_recovery_facts(&session,&operation).await.map_err(agent_session_store_error)?;
                        if generation!=facts.execution_generation {return Ok(false);}
                        if generation!=authorization_generation&&!engines.owns_recovery_claim(&facts,generation) {return Ok(true);}
                        if store.native_execution_deadline(&session,&operation).await.map_err(agent_session_store_error)?>now_ms() {return Ok(false);}
                        if store.quarantine_native_recovery(&facts.session.owner_ref,&session,&operation,facts.execution_fence).await.map_err(agent_session_store_error)? {
                            if let Some(root)=receipt.started_event.as_ref().and_then(|start|facts.event_payloads.get(start.event_id.as_ref()))
                                .and_then(|payload|payload.get("source_message_id")).and_then(Value::as_str) {
                                owner.user_events.send_to_user(&facts.session.owner_ref.principal_id,Self::canonical_turn_paused_wire_event(&session,root));
                            }
                        }
                        return Ok(true);
                    }
                    if owner.runtime_sessions.active_turn_generation(session.as_ref()).is_some()
                        ||owner.runtime_sessions.get_runtime(session.as_ref()).is_some_and(|runtime|runtime.status()==Some(ConversationStatus::Running)) {return Ok(false);}
                    let attached=tokio::select! {
                        _=tasks.shutdown_signal()=>return Ok(true),
                        result=owner.recover_native_turn(engines.clone(),&session,&operation,Some(authorization_generation))=>result,
                    };
                    if let Err(error)=attached {
                        preparation_rejected=!matches!(error,AppError::DatabaseUnavailable{..}|AppError::ExecutionAdmissionPending);
                        return Err(error);
                    }
                    Ok::<bool,AppError>(false)
                }.await;
                match result {
                    Ok(true)=>return,
                    Ok(false)=>{},
                    Err(error)=>tracing::warn!(session_id=session.as_ref(),authorization_generation,
                        error=%nomi_redact::redact_secrets_owned(error.to_string()),"original authorized resume retains attachment responsibility; no business replay"),
                }
                drop(owner);
                tokio::select! {_=tasks.shutdown_signal()=>return,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
            }
        })) { return Err(AppError::Conflict("resume authorization is durable but the local scheduler is shutting down".into())); }
        Ok(())
    }
}

fn original_effect_result(effect:&nomifun_agent_session::AgentEffectRecord,call_id:&ToolCallId)
    ->Result<AgentToolResult,String> {
    let observation=effect.bounded_observation.as_ref().ok_or_else(||"Effect has no original bounded observation".to_owned())?;
    match effect.state {
        AgentEffectState::Returned => {
            let result=observation.get("result").ok_or_else(||"returned Effect has no original typed result; it cannot be replaced by a success summary".to_owned())?;
            Ok(AgentToolResult::text(call_id.clone(),serde_json::to_string(result).map_err(|error|error.to_string())?,false))
        }
        AgentEffectState::Rejected | AgentEffectState::Cancelled => {
            let error=observation.get("error").ok_or_else(||"settled Effect has no original rejection observation".to_owned())?;
            Ok(AgentToolResult::text(call_id.clone(),serde_json::to_string(error).map_err(|error|error.to_string())?,true))
        }
        AgentEffectState::Pending | AgentEffectState::Unknown => Err("Effect outcome is not settled".into()),
    }
}

#[cfg(test)]
mod original_effect_result_tests {
    use super::*;

    fn effect(state:AgentEffectState,observation:Value)->nomifun_agent_session::AgentEffectRecord {
        nomifun_agent_session::AgentEffectRecord {effect_id:"original-effect".into(),agent_session_id:"session".into(),
            turn_id:"turn".into(),operation_id:"delegate-operation".into(),owner_domain:"wave5.product".into(),
            capability_module:"agent.collaboration".into(),action_id:"agent/delegate".into(),resource_binding_id:None,
            resource_key:Some("agent-collaboration".into()),input_digest:"a".repeat(64).into(),
            strategy:nomifun_agent_session::EffectStrategy::ManagedEffect,state,bounded_observation:Some(observation),
            started_event_id:"original-start".into(),terminal_event_id:Some("original-returned".into()),created_at:1,settled_at:Some(2)}
    }

    #[test]
    fn returned_delegation_restores_the_original_json_without_an_owner_invocation() {
        let body=json!({"execution_id":"original-execution","status":"running","message":"accepted","step_ids":["child"]});
        let record=effect(AgentEffectState::Returned,json!({"result":body}));
        let result=original_effect_result(&record,&"original-call".into()).unwrap();
        assert!(!result.is_error);
        assert_eq!(serde_json::from_str::<Value>(&result.output_text()).unwrap(),body);
        let typed:nomifun_common::AgentExecutionReceipt=serde_json::from_str(&result.output_text()).unwrap();
        assert_eq!(typed.execution_id,"original-execution");
        assert_eq!(typed.step_ids,["child"]);
    }

    #[test]
    fn absent_or_unknown_effect_output_never_becomes_a_success_summary() {
        for state in [AgentEffectState::Pending,AgentEffectState::Unknown,AgentEffectState::Returned] {
            assert!(original_effect_result(&effect(state,json!({"observation_truncated":true})),&"original-call".into()).is_err());
        }
        let original=json!({"code":"ORIGINAL_REJECTION","message":"declined before execution"});
        let result=original_effect_result(&effect(AgentEffectState::Rejected,json!({"error":original})),&"original-call".into()).unwrap();
        assert!(result.is_error);
        assert_eq!(serde_json::from_str::<Value>(&result.output_text()).unwrap(),original);
    }
}

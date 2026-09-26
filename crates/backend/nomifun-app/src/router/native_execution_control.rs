//! Owner-facing pause/reconcile/resume. No model call or tool replay occurs
//! while preparing a resume; the Store commits the exact prepared boundary.
use super::*;
use nomifun_agent_runtime::{AgentExecutionCheckpoint, AgentReconciledOutcome, AgentReconciliationSource, AgentToolResult};
use nomifun_agent_session::{AgentEffectState, NativeEffectReconciliationRequest, NativeResumePreparation, NativeResumeRequest};
use nomifun_chat_model_broker::ToolCallId;

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
    let ack = state.session_owner.canonical().store().reconcile_native_effect_by_owner(
        &authenticated_principal(&owner), &session, &request).await.map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::ok(ack)))
}

pub(super) async fn resume(
    State(state): State<NomiCoreAgentApiState>, Extension(owner): Extension<AuthenticatedOwner>,
    Path(id): Path<String>, Json(request): Json<NativeResumeRequest>,
) -> Result<Json<ApiResponse<nomifun_agent_session::NativeResumeReceipt>>, NomiCoreApiError> {
    let session = parse_agent_session_id(&id)?;
    let principal = authenticated_principal(&owner);
    let store = state.session_owner.canonical().store();
    let receipt = if let Some(receipt) = store.native_resume_receipt(&principal,&session,&request).await.map_err(agent_session_store_error)? {
        receipt
    } else {
        match state.session_owner.prepare_native_resume(&principal,&session,&request).await {
            Ok(prepared) => store.commit_native_resume(&principal,&session,&request,prepared).await.map_err(agent_session_store_error)?,
            Err(error) => {
                // A concurrent identical authorization may have changed the
                // checkpoint while this read-only preparation was in flight.
                // Only its exact authenticated receipt makes this a replay.
                match store.native_resume_receipt(&principal,&session,&request).await.map_err(agent_session_store_error)? {
                    Some(receipt) => receipt,
                    None => return Err(error.into()),
                }
            }
        }
    };
    state.session_owner.enqueue_authorized_native_resume(session,request.operation_id,receipt.seq).await?;
    Ok(Json(ApiResponse::ok(receipt)))
}

impl NomiCoreSessionOwner {
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
        let effect_rows = store.list_effects(session).await.map_err(agent_session_store_error)?;
        let effects: Vec<_> = effect_rows.iter().filter(|effect| effect.turn_id == request.operation_id).collect();
        let mut tail = Vec::new();
        let mut sequence = 0u64;
        let mut bytes = 0usize;
        let mut intents = BTreeMap::<ToolCallId,(OperationId,String)>::new();
        let mut owner_results = BTreeMap::<ToolCallId,AgentReconciledOutcome>::new();
        let mut native_started = BTreeMap::<ToolCallId,String>::new();
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
                    if let AgentEngineEvent::ToolStarted { call_id,action_id,.. } = &native { native_started.insert(call_id.clone(),action_id.as_ref().to_owned()); }
                    tail.push(native);
                }
            }
        }
        let dispatched: BTreeSet<_> = intents.keys().cloned().collect();
        for (call_id,(operation,witness)) in &intents {
            if owner_results.contains_key(call_id) { continue; }
            if let Some((attestation,event_id)) = attestations.get(call_id.as_ref()) {
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
                let returned = owned.iter().all(|effect| effect.state == AgentEffectState::Returned);
                owner_results.insert(call_id.clone(),AgentReconciledOutcome { result:AgentToolResult::text(call_id.clone(),
                    format!("Historical canonical owner receipts for operation {} confirm {}. No invocation was replayed; re-observe current state before claiming completion.",operation.as_ref(),if returned { "a returned outcome" } else { "a non-success outcome" }),!returned),
                    source:if attested.is_some() { AgentReconciliationSource::OwnerAttestation } else { AgentReconciliationSource::OwnerReceipt },
                    evidence_event_id:attested.map(|(_,id)|id.clone()).or_else(||owned[0].terminal_event_id.as_ref().map(|id|id.as_ref().to_owned())),owner_operation_id:Some(operation.clone()) });
            } else if native_started.get(call_id).is_some_and(|action| action == "workspace.files/read") {
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
        let mut reconciled = nomifun_agent_runtime::reconcile_execution_tail(&checkpoint,saved.revision,&tail,&dispatched,&owner_results,&request.budget)
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

    async fn enqueue_authorized_native_resume(self: &Arc<Self>, session: AgentSessionId, operation: OperationId, authorization_generation: u64) -> Result<(),AppError> {
        let engines = self.native_engines.get().cloned().ok_or_else(||AppError::Conflict("native recovery scheduler is not installed".into()))?;
        let owner = Arc::downgrade(self);
        if !self.background_tasks.spawn(Box::pin(async move {
            for _ in 0..300 {
                let (Some(owner),Some(engines)) = (owner.upgrade(),engines.upgrade()) else { return; };
                // The authorization cursor is replaced by the execution claim
                // cursor. A duplicate request must not dispatch that owner a
                // second time or wait until its long task times out.
                match owner.canonical.store().native_execution_generation(&session,&operation).await {
                    Ok(generation) if generation != authorization_generation => return,
                    Err(_) => return,
                    _ => {}
                }
                if owner.canonical.store().native_pause_state(&session,&operation).await.ok().flatten().is_some() { return; }
                if owner.runtime_sessions.active_turn_generation(session.as_ref()).is_some()
                    || owner.runtime_sessions.get_runtime(session.as_ref()).is_some_and(|runtime| runtime.status() == Some(ConversationStatus::Running)) {
                    drop(owner); tokio::time::sleep(Duration::from_millis(100)).await; continue;
                }
                if let Err(error) = owner.recover_native_turn(engines,&session,&operation,Some(authorization_generation)).await {
                    tracing::warn!(session_id=session.as_ref(),%error,"authorized native resume was not attached");
                }
                return;
            }
            if let Some(owner) = owner.upgrade() {
                if let Ok(facts) = owner.canonical.store().native_recovery_facts(&session,&operation).await {
                    let _ = owner.canonical.store().quarantine_native_recovery(&facts.session.owner_ref,&session,&operation,facts.execution_fence).await;
                }
            }
        })) { return Err(AppError::Conflict("resume authorization is durable but the local scheduler is shutting down".into())); }
        Ok(())
    }
}

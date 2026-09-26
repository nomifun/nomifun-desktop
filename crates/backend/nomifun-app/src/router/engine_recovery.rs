//! Rehydrate only after exact checkpoint validation and canonical lease CAS.
//! Neither the checkpoint nor a model-supplied flag selects an owner or grants
//! a capability. A post-checkpoint effect blocks automatic recovery.
use super::*;
use nomifun_agent_runtime::{AgentEngineEvent, AgentExecutionCheckpoint, AgentSteeringInput, AgentTurnRecovery};
use nomifun_agent_session::{AgentSessionStore, NativeCheckpoint, NativeExecutionClaim};
use nomifun_agent_contracts::ChatRouteFeature;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

fn error(message: impl std::fmt::Display) -> AppError { AppError::Conflict(format!("Native recovery: {message}")) }

impl EngineSessionHost {
    pub(super) async fn open_recovered_journal(&self, store: AgentSessionStore, receipt: &EngineTurnReceipt,
        cancellation: CancellationToken) -> Result<super::super::engine_journal::EngineTurnJournal, AppError> {
        let session_id = receipt.session().session().conversation_id.clone().into();
        let operation = receipt.operation_id().into();
        let saved = store.load_native_checkpoint(receipt.session().principal(), &session_id, &operation).await.map_err(error)?;
        let Some(saved) = saved else {
            let facts = store.native_recovery_facts(&session_id, &operation).await.map_err(error)?;
            let lease = store.claim_native_empty_recovery(NativeExecutionClaim {
                owner: receipt.session().principal().clone(), agent_session_id: session_id, operation_id: operation,
                snapshot: receipt.session().snapshot().snapshot_ref.clone(), active_set_generation: receipt.session().active_set_generation(),
                holder: self.execution_instance_id.clone(), expected_fence: facts.execution_fence, checkpoint: None,
            }).await.map_err(error)?;
            let result = self.bootstrap_empty_recovery(&store, receipt, cancellation, &lease).await;
            if result.is_err() { let _ = store.release_unattached_native_claim(&lease).await; }
            return result;
        };
        if saved.turn_state != "running" { return Err(error("a terminal Turn cannot be reopened by recovery")); }
        let checkpoint: AgentExecutionCheckpoint = serde_json::from_value(saved.state.0.clone()).map_err(error)?;
        checkpoint.validate().map_err(error)?;
        if checkpoint.binding.agent_session_id() != &session_id || checkpoint.turn_operation_id != operation
            || checkpoint.binding.build_id().as_ref() != receipt.session().engine_binding().build_id
            || checkpoint.binding.build_digest().as_ref() != receipt.session().engine_binding().build_digest
            || checkpoint.binding.resolved_snapshot_ref() != &receipt.session().snapshot().snapshot_ref
            || checkpoint.active_set_generation != receipt.session().active_set_generation() {
            return Err(error("checkpoint is not compatible with the exact admitted build and Snapshot"));
        }
        let (prefix, tail, _, _) = recovery_records(&store, &saved).await?;
        let image_input = receipt.session().revision().payload.chat_route_records.values().any(|record|
            std::iter::once(&record.primary).chain(record.failovers.iter()).any(|model| model.features.contains(&ChatRouteFeature::ImageInput)));
        let mut prepared = Vec::new();
        for event in &prefix {
            if let AgentEngineEvent::SteeringInputs { inputs } = event {
                for recorded in inputs {
                    let mut input: AgentSteeringInput = recorded.clone();
                    input.prepared_images = super::super::runtime_attachments::prepare_images(
                        &input.files, &receipt.session().session().extra, image_input).await?;
                    if input.prepared_images.len() != input.image_count { return Err(error("accepted steering images cannot be rehydrated")); }
                    prepared.push(input);
                }
            }
        }
        let next_fence = saved.execution_fence.checked_add(1).ok_or_else(|| error("execution fence exhausted"))?;
        AgentTurnRecovery::new(checkpoint.clone(), saved.revision, next_fence, prefix, tail, prepared.clone()).map_err(error)?;
        let lease = store.claim_native_execution(NativeExecutionClaim {
            owner: receipt.session().principal().clone(), agent_session_id: session_id, operation_id: operation,
            snapshot: receipt.session().snapshot().snapshot_ref.clone(), active_set_generation: receipt.session().active_set_generation(),
            holder: self.execution_instance_id.clone(), expected_fence: saved.execution_fence,
            checkpoint: Some((saved.revision, saved.digest.clone(), saved.through_seq)),
        }).await.map_err(error)?;
        // A model-only prefix could have arrived during preparation. The CAS
        // fenced it; reread the now-stable native tail before choosing IDs.
        let result = async {
            let (prefix, tail, sequence, total_bytes) = recovery_records(&store, &saved).await?;
            let recovery = AgentTurnRecovery::new(checkpoint, saved.revision, lease.fence(), prefix, tail, prepared).map_err(error)?;
            let journal = super::super::engine_journal::EngineTurnJournal::new_with_recovery(store.clone(), receipt, cancellation, lease.clone(), Some(recovery), sequence, total_bytes)?;
            journal.restore_public_projection().await?;
            Ok(journal)
        }.await;
        if result.is_err() { let _ = store.release_unattached_native_claim(&lease).await; }
        result
    }

    async fn bootstrap_empty_recovery(&self, store: &AgentSessionStore, receipt: &EngineTurnReceipt,
        cancellation: CancellationToken, lease: &nomifun_agent_session::NativeExecutionLease)
        -> Result<super::super::engine_journal::EngineTurnJournal, AppError> {
        use nomifun_agent_contracts::{SessionEventAppend, SemanticSessionEventDraft, SessionEventKind, SessionEventPayloadRef, StrictJsonValue, digest_payload};
        let facts = store.native_recovery_facts(lease.session_id(), lease.operation_id()).await.map_err(error)?;
        let mut prefix = Vec::new();
        let mut bytes = 0usize;
        for record in facts.events.iter().filter(|event| event.kind.0 == "runtime/progress-recorded" && event.correlation_id.as_ref() == lease.operation_id().as_ref()) {
            let payload = facts.event_payloads.get(record.event_id.as_ref()).ok_or_else(|| error("empty recovery payload missing"))?;
            if prefix.len() >= 2 || payload.get("producer_seq").and_then(Value::as_u64) != Some(prefix.len() as u64 + 1) {
                return Err(error("empty recovery contains native work"));
            }
            bytes += serde_json::to_vec(payload).map_err(error)?.len() + 256;
            prefix.push(serde_json::from_value::<AgentEngineEvent>(payload.get("event").cloned().ok_or_else(|| error("empty recovery event missing"))?).map_err(error)?);
        }
        if prefix.is_empty() { return super::super::engine_journal::EngineTurnJournal::new(store.clone(), receipt, cancellation, lease.clone()); }
        let Some(AgentEngineEvent::TurnStarted { binding, turn_operation_id }) = prefix.first() else { return Err(error("empty recovery has no native root")); };
        if turn_operation_id != lease.operation_id() || binding.agent_session_id() != lease.session_id()
            || binding.build_id().as_ref() != receipt.session().engine_binding().build_id
            || binding.build_digest().as_ref() != receipt.session().engine_binding().build_digest
            || binding.resolved_snapshot_ref() != &receipt.session().snapshot().snapshot_ref
            || prefix.get(1).is_some_and(|event| !matches!(event, AgentEngineEvent::TurnInputScope { .. })) {
            return Err(error("empty recovery differs from the admitted native root"));
        }
        let checkpoint = AgentExecutionCheckpoint {
            version: 1, binding: binding.clone(), turn_operation_id: turn_operation_id.clone(),
            active_set_generation: receipt.session().active_set_generation(), model_steps: 0, tool_call_count: 0,
            accepted_input_count: 1, applied_steering_receipts: vec![], plan: Default::default(), work: Default::default(),
            patch_recovery: Default::default(), segments: None, control_rejections: Default::default(),
        };
        checkpoint.validate().map_err(error)?;
        let state = serde_json::to_value(&checkpoint).map_err(error)?;
        let digest = digest_payload(&state).map_err(error)?;
        let metadata = AgentEngineEvent::ExecutionCheckpointSaved { step: 0, revision: 1, digest };
        let sequence = prefix.len() as u64 + 1;
        let payload = serde_json::json!({"runtime_binding_id":format!("nomi:{}",lease.session_id().as_ref()),"producer_seq":sequence,"event":metadata});
        bytes += serde_json::to_vec(&payload).map_err(error)?.len() + 256;
        let identity = format!("runtime-progress:{}:{}:{sequence}", lease.session_id().as_ref(), lease.operation_id().as_ref());
        let saved = store.save_native_checkpoint(&SessionEventAppend {
            agent_session_id: lease.session_id().clone(), event_id: identity.clone().into(), producer_id: "runtime_supervisor".into(),
            idempotency_key: identity.into(), runtime_binding_id: None, runtime_producer_seq: None,
            semantic_event: SemanticSessionEventDraft { kind: SessionEventKind("runtime/progress-recorded".into()), kind_version: 1,
                correlation_id: lease.operation_id().as_ref().into(), causation_event_id: Some(receipt.root_message_id().into()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)) },
        }, nomifun_agent_session::NativeCheckpointWrite {
            owner: receipt.session().principal().clone(), operation_id: lease.operation_id().clone(),
            snapshot: receipt.session().snapshot().snapshot_ref.clone(), active_set_generation: receipt.session().active_set_generation(),
            expected_revision: 0, execution_fence: lease.fence(), lease: Some(lease.clone()), state: StrictJsonValue(state),
        }).await.map_err(error)?;
        prefix.push(metadata);
        let recovery = AgentTurnRecovery::new(checkpoint, saved.revision, lease.fence(), prefix, vec![], vec![]).map_err(error)?;
        super::super::engine_journal::EngineTurnJournal::new_with_recovery(store.clone(), receipt, cancellation, lease.clone(), Some(recovery), sequence, bytes)
    }
}

async fn recovery_records(store: &AgentSessionStore, checkpoint: &NativeCheckpoint)
    -> Result<(Vec<AgentEngineEvent>, Vec<AgentEngineEvent>, u64, usize), AppError> {
    let facts = store.native_recovery_facts(&checkpoint.agent_session_id, &checkpoint.operation_id).await.map_err(error)?;
    if facts.head.status != "running" || facts.head.active_turn_id.as_deref() != Some(checkpoint.operation_id.as_ref()) {
        return Err(error("recovery lost the active Turn"));
    }
    let mut prefix = Vec::new(); let mut tail = Vec::new(); let mut sequence = 0_u64; let mut bytes = 0usize;
    for record in facts.events.iter().filter(|event| event.kind.0 == "runtime/progress-recorded"
        && event.correlation_id.as_ref() == checkpoint.operation_id.as_ref()) {
        let value = facts.event_payloads.get(record.event_id.as_ref()).ok_or_else(|| error("journal payload missing"))?;
        sequence += 1;
        if value.get("producer_seq").and_then(Value::as_u64) != Some(sequence) { return Err(error("journal sequence is incomplete")); }
        let event = value.get("event").ok_or_else(|| error("native event missing"))?;
        bytes = bytes.saturating_add(serde_json::to_vec(value).map_err(error)?.len().saturating_add(256));
        if bytes > nomifun_agent_contracts::MAX_NATIVE_APPROVED_REPLAY_BYTES || sequence > nomifun_agent_contracts::MAX_NATIVE_APPROVED_JOURNAL_RECORDS + 4000 {
            return Err(error("recovery journal exceeds its bounded reader"));
        }
        if event.get("event").and_then(Value::as_str).is_some_and(|kind| matches!(kind,
            "host_tool_dispatch" | "host_tool_settled" | "host_resource_dispatch" | "host_resource_settled"
            | "host_process_dispatch" | "host_process_quiescent" | "host_cleanup_proven")) { continue; }
        let event: AgentEngineEvent = serde_json::from_value(event.clone()).map_err(error)?;
        if record.seq <= checkpoint.through_seq { prefix.push(event); } else { tail.push(event); }
    }
    if !matches!(prefix.last(), Some(AgentEngineEvent::ExecutionCheckpointSaved { revision, digest, .. })
        if *revision == checkpoint.revision && digest == &checkpoint.digest) {
        return Err(error("checkpoint cursor does not identify its committed metadata"));
    }
    Ok((prefix, tail, sequence, bytes))
}

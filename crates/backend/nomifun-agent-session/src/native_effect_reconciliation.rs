//! Explicit Session-owner verification of an uncertain outcome. This is an
//! auditable attestation, not automatic owner-probe success or replay authority.
use super::*;
use super::native_pause::{native_control_event, native_key};

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeVerifiedOutcome { ConfirmedSucceeded, ConfirmedFailed }

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEffectReconciliationRequest {
    pub operation_id: OperationId,
    pub expected_pause_revision: u64,
    pub idempotency_key: String,
    #[serde(default)]
    pub effect_id: String,
    /// Alternative target when dispatch was recorded but its owner receipt
    /// was lost. Existing unknown effect rows still need separate resolution.
    #[serde(default)]
    pub call_id: Option<String>,
    pub expected_input_digest: DigestHex,
    pub outcome: NativeVerifiedOutcome,
    pub evidence: NativeOwnerEvidence,
}

#[derive(serde::Serialize)]
pub struct NativeEffectReconciliationCandidate {
    pub effect_id: String,
    pub operation_id: OperationId,
    pub owner_domain: String,
    pub action_id: ActionId,
    pub input_digest: DigestHex,
    pub state: String,
}

#[derive(serde::Serialize)]
pub struct NativeEffectReconciliationCandidates {
    pub items: Vec<NativeEffectReconciliationCandidate>,
    pub has_more: bool,
}

impl AgentSessionStore {
    pub async fn native_reconciliation_candidates(&self, owner: &PrincipalRef, session: &AgentSessionId, operation: &OperationId)
        -> Result<NativeEffectReconciliationCandidates, SessionStoreError> {
        let mut tx = self.pool.begin().await?;
        let live = live_session_by_id_tx(&mut tx, session.as_ref()).await?;
        if &live.owner_ref != owner { return Err(SessionStoreError::ExecutionFenced); }
        let mut rows: Vec<(String,String,String,String,String,String)> = sqlx::query_as(
            "SELECT effect_id,operation_id,owner_domain,action_id,input_digest,state FROM agent_effects WHERE session_id=? AND turn_id=? AND state IN ('pending','unknown') ORDER BY effect_id LIMIT 257")
            .bind(session.as_ref()).bind(operation.as_ref()).fetch_all(&mut *tx).await?;
        let has_more = rows.len() > 256; rows.truncate(256);
        Ok(NativeEffectReconciliationCandidates { has_more, items: rows.into_iter().map(|(effect_id,operation_id,owner_domain,action_id,input_digest,state)|
            NativeEffectReconciliationCandidate { effect_id, operation_id: operation_id.into(), owner_domain, action_id: action_id.into(), input_digest: input_digest.into(), state }).collect() })
    }

    pub async fn reconcile_native_effect_by_owner(&self, owner: &PrincipalRef, session: &AgentSessionId,
        request: &NativeEffectReconciliationRequest) -> Result<SessionEventAck, SessionStoreError> {
        request.evidence.validate()?;
        if request.call_id.as_ref().is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
            || (request.call_id.is_some() && !request.effect_id.is_empty())
            || (request.call_id.is_none() && (request.effect_id.trim().is_empty() || request.effect_id.len() > 1024)) {
            return Err(SessionStoreError::InvalidEvent("reconciliation requires exactly one bounded effect or invocation identity".into()));
        }
        let id = native_key("effect-attestation",session,&request.idempotency_key)?;
        let request_digest = digest_payload(request)?;
        let mut tx = self.begin_write_transaction().await?;
        let live = live_session_by_id_tx(&mut tx,session.as_ref()).await?;
        if &live.owner_ref != owner { return Err(SessionStoreError::ExecutionFenced); }
        if let Some(row) = event_by_event_id_tx(&mut tx,&id).await? {
            let event = event_from_row(row)?;
            if payload_value_for_event_tx(&mut tx,&event).await?.get("request_digest").and_then(Value::as_str) != Some(request_digest.as_ref()) {
                return Err(SessionStoreError::IdempotencyConflict("effect reconciliation request changed".into()));
            }
            return Ok(event_ack(&event));
        }
        let pause: (String,Option<String>,i64,String,Option<String>) = sqlx::query_as("SELECT started_event_id,native_pause_json,native_pause_revision,state,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=? AND (native_pause_json IS NOT NULL OR state IN ('failed','cancelled','interrupted'))")
            .bind(session.as_ref()).bind(request.operation_id.as_ref()).fetch_one(&mut *tx).await?;
        if as_u64(pause.2,"pause revision")? != request.expected_pause_revision { return Err(SessionStoreError::Conflict("effect reconciliation targets an old pause".into())); }
        let authority_event = if pause.1.is_some() {
            format!("native-paused:{}:{}:{}",session.as_ref(),request.operation_id.as_ref(),request.expected_pause_revision)
        } else { pause.4.clone().ok_or_else(||SessionStoreError::InvalidEvent("stopped execution boundary is missing".into()))? };
        if let Some(call_id) = &request.call_id {
            let rows = event_rows_for_session_tx(&mut tx,session.as_ref()).await?;
            let mut arguments = None;
            for row in rows {
                let event = event_from_row(row)?;
                if event.kind.0 != "runtime/progress-recorded" || event.correlation_id.as_ref() != request.operation_id.as_ref() { continue; }
                let payload = payload_value_for_event_tx(&mut tx,&event).await?;
                if payload.pointer("/event/event").and_then(Value::as_str) == Some("tool_call_completed")
                    && payload.pointer("/event/call/call_id").and_then(Value::as_str) == Some(call_id.as_str()) {
                    if arguments.is_some() { return Err(SessionStoreError::InvalidEvent("invocation identity is ambiguous".into())); }
                    arguments = payload.pointer("/event/call/arguments").cloned();
                }
            }
            if arguments.as_ref().map(digest_payload).transpose()?.as_ref() != Some(&request.expected_input_digest) {
                return Err(SessionStoreError::Conflict("invocation attestation does not match its exact recorded arguments".into()));
            }
            let ack = required_ack(self.append_event_tx(&mut tx,&native_control_event(session,&request.operation_id,id,
                "runtime/effect-reconciliation-attested",authority_event.into(),json!({
                    "request_digest":request_digest,"call_id":call_id,"input_digest":request.expected_input_digest,
                    "authority":"session_owner_verified","verifier_digest":digest_payload(owner)?,"outcome":request.outcome,"evidence":request.evidence,
                    "does_not_resolve_unknown_effect_rows":true,
                })),None).await?)?;
            tx.commit().await?;
            return Ok(ack);
        }
        let row = sqlx::query_as::<_, StoredEffectRow>(
            "SELECT effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id,resource_binding_id,resource_key,input_digest,strategy,state,bounded_observation_json,started_event_id,terminal_event_id,created_at,settled_at FROM agent_effects WHERE session_id=? AND turn_id=? AND effect_id=?")
            .bind(session.as_ref()).bind(request.operation_id.as_ref()).bind(&request.effect_id).fetch_one(&mut *tx).await?;
        let effect = effect_from_row(row)?;
        if !matches!(effect.state,AgentEffectState::Unknown|AgentEffectState::Pending) || effect.input_digest != request.expected_input_digest {
            return Err(SessionStoreError::Conflict("effect is not the exact uncertain input being verified".into()));
        }
        let started = event_by_event_id_tx(&mut tx,effect.started_event_id.as_ref()).await?.ok_or_else(|| SessionStoreError::InvalidEvent("effect start receipt missing".into()))?;
        let pending = effect.state == AgentEffectState::Pending;
        let terminal = effect.terminal_event_id.clone();
        if !pending && terminal.is_none() { return Err(SessionStoreError::InvalidEvent("uncertain effect has no uncertainty receipt".into())); }
        let audit = native_control_event(session,&request.operation_id,id.clone(),"runtime/effect-reconciliation-attested",authority_event.into(),
            json!({"request_digest":request_digest,"effect_id":effect.effect_id,"input_digest":effect.input_digest,
                "authority":"session_owner_verified","verifier_digest":digest_payload(owner)?,"outcome":request.outcome,"evidence":request.evidence}));
        let ack = required_ack(self.append_event_tx(&mut tx,&audit,None).await?)?;
        let outcome = match request.outcome {
            NativeVerifiedOutcome::ConfirmedSucceeded => EffectReconcileOutcome::ConfirmedSucceeded { receipt: json!({
                "authority":"session_owner_verified","attestation_event_id":ack.event_id,"evidence_digest":request.evidence.evidence_digest,
                "not_original_execution_receipt":true,
            }) },
            NativeVerifiedOutcome::ConfirmedFailed => EffectReconcileOutcome::ConfirmedFailed { error: "NATIVE_OWNER_VERIFIED_FAILURE".into() },
        };
        let mut record = EffectEventRequest {
            agent_session_id: session.clone(), effect_id: effect.effect_id.clone(), turn_id: effect.turn_id, operation_id: effect.operation_id,
            owner_domain: effect.owner_domain, capability_module: effect.capability_module, action_id: effect.action_id,
            resource_binding_id: effect.resource_binding_id, resource_key: effect.resource_key, input_digest: effect.input_digest,
            recorded_at: wall_clock_now_ms(), event_id: format!("{id}:effect").into(), producer_id: "native_owner_reconciliation".into(),
            idempotency_key: started.idempotency_key.into(), correlation_id: effect.effect_id.into(), strategy: effect.strategy,
            causation_event_id: terminal, payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(outcome)?)),
        };
        if pending {
            // A stopped owner has explicitly verified the outcome but the
            // original terminal receipt was lost. Preserve that uncertainty
            // transition rather than inventing an original successful return.
            let mut uncertain = record.clone();
            // The effect keeps its original idempotency key across lifecycle
            // events. Uncertainty and reconciliation need distinct producers
            // so the canonical (producer, key) fence does not conflate them.
            uncertain.producer_id = "runtime_supervisor".into();
            uncertain.event_id = format!("{id}:uncertain").into();
            uncertain.causation_event_id = Some(effect.started_event_id);
            uncertain.payload = SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "recovery":"native_owner_verified_reconciliation","reconciliation_event_id":ack.event_id,
            })));
            let uncertainty = required_ack(self.append_event_tx_with_policy(&mut tx,&effect_append(uncertain,"effect/uncertain")?,None,AppendSessionStatePolicy::EffectSettlement).await?)?;
            record.causation_event_id = Some(uncertainty.event_id);
        }
        self.append_event_tx_with_policy(&mut tx,&effect_append(record,"effect/reconciled")?,None,AppendSessionStatePolicy::EffectSettlement).await?;
        tx.commit().await?;
        Ok(ack)
    }
}

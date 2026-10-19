//! A question remains playable only while its exact DecisionRequested epoch is
//! pending. The short SQLite write reservation is held through synchronous
//! writer acceptance, serializing that acceptance against Execution cancel.
//! Every read below uses the caller's transaction, including stored Payloads.

use super::*;
use nomifun_agent_contracts::{AgentBindingValue, AgentHandoffBindingRefV1, EventId};
use nomifun_db::sqlx::{Sqlite, Transaction};
use nomifun_robot::services::{RobotQuestionPlaybackAuthority, RobotTurnRequest};

#[derive(sqlx::FromRow)]
struct SessionScope {
    state: String,
    owner_ref_json: String,
    agent_binding_json: Option<String>,
    remote_binding_id: Option<String>,
}

async fn scope_tx(
    tx: &mut Transaction<'_, Sqlite>,
    owner: &str,
    session: &str,
) -> Result<Option<AgentBindingValue>, AppError> {
    let row: Option<SessionScope> = sqlx::query_as("SELECT state,owner_ref_json,agent_binding_json,remote_binding_id FROM agent_sessions WHERE agent_session_id=?")
        .bind(session).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let principal: PrincipalRef = serde_json::from_str(&row.owner_ref_json).map_err(invalid)?;
    if principal.principal_kind != "user" || principal.principal_id != owner {
        return Err(AppError::Forbidden(
            "Question belongs to another owner".into(),
        ));
    }
    if row.state != "live" || row.remote_binding_id.is_some() {
        return Ok(None);
    }
    let binding = serde_json::from_str(
        &row.agent_binding_json
            .ok_or_else(|| invalid("live question Session has no binding"))?,
    )
    .map_err(invalid)?;
    Ok(Some(binding))
}

fn db(error: sqlx::Error) -> AppError {
    AppError::Internal(error.to_string())
}

async fn payload_tx(
    owner: &NomiCoreSessionOwner,
    tx: &mut Transaction<'_, Sqlite>,
    session: &AgentSessionId,
    id: &str,
) -> Result<Value, AppError> {
    let mut payloads = owner
        .canonical
        .store()
        .resolved_event_payloads_in_transaction(tx, session, &[EventId::from(id)])
        .await
        .map_err(agent_session_store_error)?;
    payloads
        .remove(id)
        .ok_or_else(|| invalid("question's exact canonical Payload is missing"))
}

pub(super) struct ValidQuestion {
    pub(super) seq: u64,
}

/// This validates identities rather than aggregate metadata CAS versions.
/// Outbox acknowledgements may advance those versions without replacing a
/// pending question; a new question sequence, target or state always revokes it.
pub(super) async fn pending_question_tx(
    owner: &NomiCoreSessionOwner,
    tx: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    request: &RobotTurnRequest,
    authority: &RobotQuestionPlaybackAuthority,
) -> Result<Option<ValidQuestion>, AppError> {
    let Some(context) = request.captured_context.as_ref() else {
        return Ok(None);
    };
    let session = AgentSessionId::from(request.conversation_id.clone());
    let Some(binding) = scope_tx(tx, owner_id, session.as_ref()).await? else {
        return Ok(None);
    };
    if binding.binding_version != context.binding_version {
        return Ok(None);
    }
    let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared' AND kind_version=1")
        .bind(session.as_ref()).fetch_one(&mut **tx).await.map_err(db)?;
    if u64::try_from(floor).map_err(invalid)? != context.context_floor {
        return Ok(None);
    }
    let event:Option<(i64,String,String,Option<String>,String,Option<String>)>=sqlx::query_as(
        "SELECT seq,correlation_id,idempotency_key,causation_event_id,producer_id,payload_id FROM agent_events WHERE session_id=? AND event_id=? AND kind='message/assistant-projected' AND kind_version=1")
        .bind(session.as_ref()).bind(&authority.source.message_id).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((seq, correlation, key, cause, producer, _)) = event else {
        return Err(invalid("exact question notice is missing"));
    };
    let seq = u64::try_from(seq).map_err(invalid)?;
    if producer != "session_api"
        || correlation != authority.source.message_id
        || authority.source.revision != seq
        || authority.source.through_seq != seq
    {
        return Err(invalid(
            "question speech source differs from its canonical notice",
        ));
    }
    if seq <= u64::try_from(floor).map_err(invalid)? {
        return Ok(None);
    }
    let notice: QuestionNotice = serde_json::from_value(
        payload_tx(owner, tx, &session, &authority.source.message_id).await?,
    )
    .map_err(invalid)?;
    let target = &notice.decision.target;
    if target.expected_execution_version < 0
        || target.expected_step_version < 0
        || target.expected_attempt_version < 0
        || target.request_event_sequence <= 0
        || target.interaction_binding_version == 0
        || target.work_target.binding_version == 0
        || [
            target.question_digest.as_ref(),
            target.presentation_digest.as_ref(),
        ]
        .iter()
        .any(|digest| {
            digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        || target.presentation_digest
            != crate::router::voice_work_host::approval_digest(target).map_err(invalid)?
    {
        return Err(invalid("original question presentation proof is invalid"));
    }
    if notice.origin != "agent_execution_input_required"
        || notice.source.agent_session_id != session.as_ref()
        || notice.connection_id != request.connection_id
        || target.interaction_agent_session_id != session
        || target.interaction_binding_version != context.binding_version
        || target.presentation_id != authority.presentation_id
        || target.presentation_id
            != format!(
                "execution:{}:decision:{}",
                target.execution_id, target.request_event_sequence
            )
        || target.action_id.as_ref() != "agent/request_user_decision"
        || notice.operation_id
            != format!(
                "exec-lead-input:{}:event:{}",
                target.execution_id, target.request_event_sequence
            )
        || key != format!("agent-execution-input:{}", notice.operation_id)
    {
        return Err(invalid(
            "notice, presentation, connection or task identity changed",
        ));
    }
    // Verify the immutable physical opening and source input, not a current
    // device selected from task text or a newer interaction Turn.
    let opening_id:String=sqlx::query_scalar("SELECT event_id FROM agent_events WHERE session_id=? AND seq=1 AND kind='session/opening' AND kind_version=1")
        .bind(session.as_ref()).fetch_one(&mut **tx).await.map_err(db)?;
    let opening = payload_tx(owner, tx, &session, &opening_id).await?;
    let metadata: nomifun_agent_contracts::AgentSessionMetadata = serde_json::from_value(
        opening
            .get("metadata")
            .cloned()
            .ok_or_else(|| invalid("question opening has no metadata"))?,
    )
    .map_err(invalid)?;
    if metadata.purpose != nomifun_agent_contracts::SessionPurpose::Conversation
        || metadata.app_project_id.is_some()
        || !metadata.companion.as_ref().is_some_and(|scope| {
            scope.role == CompanionSessionRole::Robot
                && scope.companion_id == request.companion_id
                && scope.robot_id.as_deref() == Some(request.robot_id.as_str())
        })
    {
        return Ok(None);
    }
    let parent:Option<(String,String,String,String,i64)>=sqlx::query_as("SELECT turn.state,turn.started_event_id,turn.terminal_event_id,turn.source_message_id,started.seq FROM agent_turns turn JOIN agent_events started ON started.event_id=turn.started_event_id AND started.session_id=turn.session_id WHERE turn.session_id=? AND turn.operation_id=? AND started.kind='turn/started' AND started.kind_version=1 AND started.producer_id='session_api' AND started.correlation_id=turn.operation_id AND started.causation_event_id=turn.source_message_id")
        .bind(session.as_ref()).bind(&notice.source.turn_operation_id).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((state, started_id, terminal_id, root_id, start_seq)) = parent else {
        return Err(invalid("question source Turn is missing"));
    };
    if state != "completed" || cause.as_deref() != Some(terminal_id.as_str()) {
        return Ok(None);
    }
    if start_seq <= floor {
        return Ok(None);
    }
    let terminal_valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND event_id=? AND kind='turn/completed' AND kind_version=1 AND correlation_id=? AND seq>?)")
        .bind(session.as_ref()).bind(&terminal_id).bind(&notice.source.turn_operation_id).bind(start_seq).fetch_one(&mut **tx).await.map_err(db)?;
    if !terminal_valid {
        return Err(invalid(
            "question source terminal differs from its original Turn",
        ));
    }
    let started = payload_tx(owner, tx, &session, &started_id).await?;
    if started.get("operation_id").and_then(Value::as_str)
        != Some(notice.source.turn_operation_id.as_str())
        || started.get("source_message_id").and_then(Value::as_str) != Some(root_id.as_str())
    {
        return Err(invalid("source Turn claim differs from its input"));
    }
    if super::super::canonical_admission_binding_ref(&started)?.as_ref()
        != Some(&AgentHandoffBindingRefV1::from(&binding))
    {
        return Ok(None);
    }
    let input = payload_tx(owner, tx, &session, &root_id).await?;
    let input: AgentInputSource = serde_json::from_value(
        input
            .get("input_source")
            .cloned()
            .ok_or_else(|| invalid("question source lacks authenticated ingress"))?,
    )
    .map_err(invalid)?;
    if !matches!(input,AgentInputSource::AuthenticatedRobot{companion_id,robot_id,connection_id,..}
        if companion_id==request.companion_id&&robot_id==request.robot_id&&connection_id==request.connection_id)
    {
        return Ok(None);
    }
    let effect:Option<(String,String,String,String,String,String)>=sqlx::query_as("SELECT session_id,turn_id,operation_id,owner_domain,action_id,state FROM agent_effects WHERE effect_id=? AND capability_module='agent.collaboration'")
        .bind(&notice.source.effect_id).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((effect_session, effect_turn, effect_operation, domain, action, state)) = effect
    else {
        return Err(invalid("question source Effect is missing"));
    };
    if effect_session != session.as_ref()
        || effect_turn != notice.source.turn_operation_id
        || effect_operation != notice.source.tool_operation_id
        || domain != "wave5.product"
        || !matches!(action.as_str(), "agent/delegate" | "agent/fork")
    {
        return Err(invalid("question source Effect identity changed"));
    }
    if state != "returned" {
        return Ok(None);
    }
    let execution:Option<(String,String,String,Option<i64>)>=sqlx::query_as("SELECT status,decision_policy,initial_plan_input,deleted_at FROM agent_executions WHERE execution_id=? AND user_id=?")
        .bind(&target.execution_id).bind(owner_id).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((status, policy, initial, deleted)) = execution else {
        return Ok(None);
    };
    if status != "waiting_input" || deleted.is_some() {
        return Ok(None);
    }
    if nomifun_agent_execution::execution_source_from_initial_plan(&initial)?.as_ref()
        != Some(&notice.source)
    {
        return Err(invalid("pending execution has another original source"));
    }
    let linked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE execution_id=? AND conversation_id=? AND relation='lead' AND active=1)")
        .bind(&target.execution_id).bind(session.as_ref()).fetch_one(&mut **tx).await.map_err(db)?;
    if !linked {
        return Ok(None);
    }
    let attempt:Option<(String,String,Option<String>,Option<String>,Option<i64>)>=sqlx::query_as(
        "SELECT attempt.status,step.status,attempt.question,attempt.runtime_state,step.superseded_in_revision FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id WHERE attempt.execution_id=? AND attempt.step_id=? AND attempt.attempt_id=?")
        .bind(&target.execution_id).bind(&target.step_id).bind(&target.attempt_id).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((attempt_status, step_status, question, runtime, superseded)) = attempt else {
        return Ok(None);
    };
    let Some(question) = question else {
        return Ok(None);
    };
    let child_sessions: Vec<String> = sqlx::query_scalar(
        "SELECT conversation_id FROM conversation_execution_links WHERE execution_id=? AND step_id=? AND attempt_id=? AND relation='attempt' AND active=1 LIMIT 2",
    )
    .bind(&target.execution_id)
    .bind(&target.step_id)
    .bind(&target.attempt_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(db)?;
    let [child] = child_sessions.as_slice() else {
        return Ok(None);
    };
    let child = child.clone();
    if attempt_status != "waiting_input"
        || step_status != "waiting_input"
        || superseded.is_some()
        || child != target.work_target.agent_session_id.as_ref()
        || digest_bytes(question.as_bytes()) != target.question_digest
    {
        return Ok(None);
    }
    let request_event:Option<(i64,String)>=sqlx::query_as("SELECT sequence,payload FROM agent_execution_events WHERE execution_id=? AND step_id=? AND attempt_id=? AND event_type='decision_requested' AND on_behalf_of_user_id=? AND json_extract(payload,'$.question')=? ORDER BY sequence DESC LIMIT 1")
        .bind(&target.execution_id).bind(&target.step_id).bind(&target.attempt_id).bind(owner_id).bind(&question).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((request_sequence, request_payload)) = request_event else {
        return Err(invalid("pending question has no DecisionRequested event"));
    };
    if request_sequence != target.request_event_sequence {
        return Ok(None);
    }
    let requested: Value = serde_json::from_str(&request_payload).map_err(invalid)?;
    let mode = if requested.get("force_click").and_then(Value::as_bool) == Some(true)
        || requested
            .get("requires_explicit_click")
            .and_then(Value::as_bool)
            == Some(true)
    {
        ApprovalInteractionMode::ExplicitClick
    } else {
        ApprovalInteractionMode::VoiceAllowed
    };
    if mode != target.interaction_mode {
        return Ok(None);
    }
    let child_id = AgentSessionId::from(child.clone());
    let Some(child_binding) = scope_tx(tx, owner_id, &child).await? else {
        return Ok(None);
    };
    if child_binding.binding_version != target.work_target.binding_version {
        return Ok(None);
    }
    let work:Option<(i64,String,Option<String>,String,i64)>=sqlx::query_as("SELECT turn.execution_generation,turn.started_event_id,turn.terminal_event_id,turn.state,started.seq FROM agent_turns turn JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id WHERE turn.session_id=? AND turn.operation_id=? AND started.kind='turn/started' AND started.kind_version=1 AND started.producer_id='session_api' AND started.correlation_id=turn.operation_id AND started.causation_event_id=turn.source_message_id")
        .bind(&child).bind(target.work_target.turn_operation_id.as_ref()).fetch_optional(&mut **tx).await.map_err(db)?;
    let Some((generation, child_start, child_terminal, child_state, child_seq)) = work else {
        return Ok(None);
    };
    if u64::try_from(generation).map_err(invalid)? != target.work_target.execution_generation {
        return Ok(None);
    }
    let child_cleared:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='context/cleared' AND seq>?)")
        .bind(&child).bind(child_seq).fetch_one(&mut **tx).await.map_err(db)?;
    if child_cleared {
        return Ok(None);
    }
    let claim = payload_tx(owner, tx, &child_id, &child_start).await?;
    if super::super::canonical_admission_binding_ref(&claim)?.as_ref()
        != Some(&AgentHandoffBindingRefV1::from(&child_binding))
    {
        return Ok(None);
    }
    if let Some(stop) = requested
        .get("stop_turn_operation_id")
        .and_then(Value::as_str)
    {
        // Automatic decisions still cannot ask the user to choose for the
        // Agent. A canonical InputRequired below is a different case: the
        // original Turn completed because it needs missing human input.
        if policy != "ask_user" {
            return Ok(None);
        }
        let effects = nomifun_db::AttemptConversationEffects::decode(runtime.as_deref())?;
        let pending=effects.pending_conversation_effects.iter().any(|effect|matches!(effect,
            nomifun_db::PendingConversationEffect::StopTurn {operation_id,target_operation_id}
                if operation_id==stop&&target_operation_id==target.work_target.turn_operation_id.as_ref()));
        if !pending {
            let cancel_id:Option<String>=sqlx::query_scalar("SELECT event_id FROM agent_events WHERE session_id=? AND kind='turn/cancelled' AND kind_version=1 AND producer_id='session_api' AND idempotency_key=?")
                .bind(&child).bind(format!("user:{owner_id}:{child}:{stop}:cancel")).fetch_optional(&mut **tx).await.map_err(db)?;
            let Some(cancel_id) = cancel_id else {
                return Ok(None);
            };
            let cancelled = payload_tx(owner, tx, &child_id, &cancel_id).await?;
            if cancelled.get("target_operation_id").and_then(Value::as_str)
                != Some(target.work_target.turn_operation_id.as_ref())
            {
                return Err(invalid("question stop receipt has another target"));
            }
        }
    } else {
        if requested
            .get("canonical_operation_id")
            .and_then(Value::as_str)
            != Some(target.work_target.turn_operation_id.as_ref())
            || requested.get("terminal_event_id").and_then(Value::as_str)
                != child_terminal.as_deref()
            || child_state != "completed"
        {
            return Ok(None);
        }
        let typed: nomifun_agent_contracts::AgentUserInputRequest = serde_json::from_value(
            requested
                .get("user_input_request")
                .cloned()
                .ok_or_else(|| invalid("question lost its typed request"))?,
        )
        .map_err(invalid)?;
        typed.validate().map_err(invalid)?;
        if typed.question != question {
            return Err(invalid("pending question differs from its typed request"));
        }
        let terminal_id = child_terminal
            .as_deref()
            .ok_or_else(|| invalid("question child Turn has no terminal"))?;
        let exact_terminal: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND event_id=? AND kind='turn/completed' AND kind_version=1 AND correlation_id=? AND seq>?)",
        )
        .bind(&child)
        .bind(terminal_id)
        .bind(target.work_target.turn_operation_id.as_ref())
        .bind(child_seq)
        .fetch_one(&mut **tx)
        .await
        .map_err(db)?;
        if !exact_terminal {
            return Err(invalid(
                "question child terminal differs from its exact Turn",
            ));
        }
        let terminal = payload_tx(owner, tx, &child_id, terminal_id).await?;
        let terminal_request: nomifun_agent_contracts::AgentUserInputRequest =
            serde_json::from_value(
                terminal
                    .get("user_input_request")
                    .cloned()
                    .ok_or_else(|| invalid("question child terminal has no typed input request"))?,
            )
            .map_err(invalid)?;
        terminal_request.validate().map_err(invalid)?;
        if terminal_request != typed {
            return Err(invalid(
                "pending question differs from its exact terminal request",
            ));
        }
    }
    Ok(Some(ValidQuestion { seq }))
}

impl NomiCoreSessionOwner {
    pub(crate) async fn hold_execution_question_playback(
        &self,
        owner_id: &str,
        request: &RobotTurnRequest,
        authority: &RobotQuestionPlaybackAuthority,
    ) -> Result<Option<Transaction<'static, Sqlite>>, AppError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        if pending_question_tx(self, &mut tx, owner_id, request, authority)
            .await?
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(tx))
    }

    pub(crate) async fn record_execution_question_presented(
        &self,
        owner_id: &str,
        request: &RobotTurnRequest,
        authority: &RobotQuestionPlaybackAuthority,
    ) -> Result<(), AppError> {
        let session = AgentSessionId::from(request.conversation_id.clone());
        let _fence = self
            .session_operation_lock(session.as_ref())
            .write_owned()
            .await;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;
        let Some(valid) = pending_question_tx(self, &mut tx, owner_id, request, authority).await?
        else {
            return Err(invalid(
                "question is no longer pending at writer presentation",
            ));
        };
        let payload = QuestionPlaybackAccepted {
            notice_message_id: authority.source.message_id.clone(),
            notice_seq: valid.seq,
            connection_id: request.connection_id.clone(),
            presentation_id: authority.presentation_id.clone(),
        };
        let key = format!(
            "robot-question-playback:{}:{}",
            payload.notice_message_id, payload.connection_id
        );
        let existing:Option<String>=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND producer_id='session_api' AND idempotency_key=? AND kind='robot/question-playback-accepted' AND kind_version=1 AND payload_id IS NULL")
            .bind(session.as_ref()).bind(&key).fetch_optional(&mut *tx).await.map_err(db)?;
        if let Some(existing) = existing {
            if serde_json::from_str::<QuestionPlaybackAccepted>(&existing).map_err(invalid)?
                != payload
            {
                return Err(invalid(
                    "question presentation replay changed its original source",
                ));
            }
            tx.commit().await.map_err(db)?;
            return Ok(());
        }
        self.canonical
            .store()
            .append_robot_question_playback_in_transaction(
                &mut tx,
                &SessionEventAppend {
                    agent_session_id: session,
                    event_id: uuid::Uuid::now_v7().to_string().into(),
                    producer_id: "session_api".into(),
                    idempotency_key: key.into(),
                    semantic_event: SemanticSessionEventDraft {
                        kind: SessionEventKind("robot/question-playback-accepted".into()),
                        kind_version: 1,
                        correlation_id: payload.notice_message_id.clone().into(),
                        causation_event_id: Some(payload.notice_message_id.clone().into()),
                        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                            serde_json::to_value(payload).map_err(invalid)?,
                        )),
                    },
                },
            )
            .await
            .map_err(agent_session_store_error)?;
        tx.commit().await.map_err(db)?;
        Ok(())
    }
}

//! Explicit owner replay of the selected Session's completed canonical answer.
//! No message projection, live draft, previous-success fallback or new Turn.
use super::*;
use nomifun_agent_contracts::{AgentTurnOutcome, CompanionSessionRole, digest_bytes};
use nomifun_agent_session::{ChatCausalityFacts, TurnReceipt, TurnReceiptStatus};
use nomifun_voice_contracts::VoiceSpeechSourceRef;

fn invalid(detail: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Saved Robot response: {detail}"))
}

impl NomiCoreSessionOwner {
    /// `session_id` is the owner's explicit source selection. A desktop source
    /// remains a desktop source; its input is never relabeled as Robot ingress.
    pub(crate) async fn robot_saved_response(&self, owner: &str, companion: &str,
        robot: &str, session_id: &str) -> Result<nomifun_robot::services::RobotSavedResponse, AppError> {
        let session = AgentSessionId::from(session_id.to_owned());
        let _fence = self.session_operation_lock(session_id).read_owned().await;
        let principal = PrincipalRef {principal_kind:"user".into(), principal_id:owner.into()};
        let current = self.canonical.session_record(&principal, &session).await?;
        let opening = if current.metadata.companion.is_none() {
            self.canonical.store().read_events(&session,None,1).await.map_err(agent_session_store_error)?.events
        } else {Vec::new()};
        let scope = super::companion_sessions::canonical_companion_origin_from_opening(owner,&current,&opening)?
            .ok_or_else(||invalid("source Session has no canonical Companion affiliation"))?;
        if !robot_playback_scope_matches(&scope,companion,robot) {
            return Err(AppError::Forbidden("Saved response belongs to another Companion or device".into()));
        }
        let head = self.canonical.store().head(&session).await.map_err(agent_session_store_error)?;
        if head.active_turn_id.is_some() || head.status != "ready" {
            return Err(invalid("wait for the selected Session's current work to settle"));
        }
        // Select the newest Turn identity, then validate that exact Turn. A
        // newer failed/paused Turn must never expose an older successful reply.
        let operation: Option<String> = sqlx::query_scalar("SELECT turn.operation_id FROM agent_turns turn JOIN agent_events started ON started.session_id=turn.session_id AND started.event_id=turn.started_event_id WHERE turn.session_id=? ORDER BY started.seq DESC LIMIT 1")
            .bind(session_id).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let operation = OperationId::from(operation.ok_or_else(||invalid("no completed answer is available"))?);
        let facts = self.canonical.store().turn_output_facts(&session,&operation).await.map_err(agent_session_store_error)?;
        let receipt = self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
        let floor: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session_id).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let floor = u64::try_from(floor).map_err(invalid)?;
        validate_saved_answer_context(&facts,&receipt,&principal,floor)?;
        let terminal = receipt.terminal_event.as_ref().ok_or_else(||invalid("answer has no terminal proof"))?;
        let successor: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='message/user-accepted' AND seq>?)")
            .bind(session_id).bind(i64::try_from(terminal.seq).map_err(invalid)?)
            .fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if successor {return Err(invalid("the selected Session has a newer accepted request"));}
        let (_, input) = self.canonical.store().turn_accepted_input(&session,&operation).await.map_err(agent_session_store_error)?;
        if input.0.get("hidden").and_then(Value::as_bool) == Some(true) {
            return Err(invalid("hidden work is not an owner replay source"));
        }
        let segments = saved_answer_segments(&facts,&receipt)?;
        Ok(nomifun_robot::services::RobotSavedResponse {segments,
            context:nomifun_robot::registry::RobotPlaybackContext {session_id:session_id.into(),
                input_context:nomifun_robot::services::RobotInputContext {
                    binding_version:current.agent_binding.binding_version,context_floor:floor,
                    presented_question_id:None,work_operation_id:None,execution_generation:None,
                    background_work:None,background_work_ambiguous:false,
                }}})
    }
}

pub(super) fn robot_playback_scope_matches(scope:&nomifun_agent_contracts::CompanionSessionOrigin,
    companion:&str,robot:&str)->bool {
    scope.companion_id==companion&&(scope.role!=CompanionSessionRole::Robot||scope.robot_id.as_deref()==Some(robot))
}

fn validate_saved_answer_context(facts:&ChatCausalityFacts,receipt:&TurnReceipt,
    principal:&PrincipalRef,floor:u64)->Result<(),AppError> {
    if facts.session.owner_ref != *principal || receipt.agent_session_id != facts.session.agent_session_id {
        return Err(AppError::Forbidden("Saved answer belongs to another owner or Session".into()));
    }
    if receipt.status != TurnReceiptStatus::Completed {return Err(invalid("the latest Turn has no successful answer"));}
    let started = receipt.started_event.as_ref().ok_or_else(||invalid("answer has no original admission"))?;
    if started.seq <= floor {return Err(invalid("answer source was cleared"));}
    let payload = facts.event_payloads.get(started.event_id.as_ref()).ok_or_else(||invalid("answer admission is missing"))?;
    if canonical_admission_binding_ref(payload)?.as_ref() != Some(&AgentHandoffBindingRefV1::from(&facts.session.agent_binding)) {
        return Err(invalid("answer binding authority was replaced"));
    }
    Ok(())
}

fn saved_answer_segments(facts:&ChatCausalityFacts,receipt:&TurnReceipt)
    ->Result<Vec<(VoiceSpeechSourceRef,String)>,AppError> {
    if receipt.status != TurnReceiptStatus::Completed {return Err(invalid("only a successful Turn may be replayed"));}
    let public = nomifun_agent_session::canonical_turn_public_result(facts,receipt).map_err(agent_session_store_error)?;
    let Some(AgentTurnOutcome::Answered {message_ids}) = public.turn_outcome else {
        return Err(invalid("the exact latest Turn is not an Answered result"));
    };
    let started = receipt.started_event.as_ref().ok_or_else(||invalid("answer has no original Turn"))?;
    let terminal = receipt.terminal_event.as_ref().ok_or_else(||invalid("answer has no successful terminal"))?;
    let root = facts.event_payloads.get(started.event_id.as_ref()).and_then(|value|value.get("source_message_id"))
        .and_then(Value::as_str).ok_or_else(||invalid("answer has no accepted root"))?;
    for event in facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"
        &&event.correlation_id.as_ref()==receipt.operation_id.as_ref()) {
        let Some(native)=facts.event_payloads.get(event.event_id.as_ref()).and_then(|value|value.get("event")) else {continue;};
        if !matches!(native.get("event").and_then(Value::as_str),
            Some("voice_model_step_superseded"|"model_response_rejected"|"delivery_review_superseded")) {continue;}
        if event.kind_version!=1||event.producer_id.as_ref()!="runtime_supervisor" {
            return Err(invalid("withdrawal has a different producer or version"));
        }
        let step=match serde_json::from_value::<AgentEngineEvent>(native.clone()).map_err(invalid)? {
            AgentEngineEvent::VoiceModelStepSuperseded {step,..}|AgentEngineEvent::ModelResponseRejected {step,..}
                |AgentEngineEvent::DeliveryReviewSuperseded {step,..}=>step,
            _=>unreachable!(),
        };
        let withdrawn=nomifun_agent_contracts::canonical_assistant_step_message_id(root,step).map_err(invalid)?;
        if message_ids.contains(&withdrawn) {return Err(invalid("the selected answer was withdrawn"));}
    }
    let mut segments = Vec::new();
    for id in message_ids {
        let mut text = String::new(); let mut count = 0; let mut last_part = None; let mut seal = None;
        for event in facts.events.iter().filter(|event|event.seq>started.seq&&event.seq<terminal.seq
            &&event.correlation_id.as_ref()==id) {
            if !matches!(event.kind.0.as_str(),"message/content-part"|"message/completed") {continue;}
            if event.producer_id.as_ref()!="runtime_supervisor"||event.kind_version!=1 {
                return Err(invalid("answer publication has a different producer or version"));
            }
            let payload = facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(||invalid("answer publication has no payload"))?;
            if event.kind.0=="message/content-part" {
                if seal.is_some()||payload.get("turn_id").and_then(Value::as_str)!=Some(root) {
                    return Err(invalid("answer part has a different root or follows its seal"));
                }
                let part = payload.get("content").and_then(Value::as_str).ok_or_else(||invalid("answer part has no text"))?;
                text.push_str(part); count += 1; last_part = Some(event.event_id.clone());
            } else {
                if seal.replace(event).is_some()||payload.get("part_count").and_then(Value::as_u64)!=Some(count)
                    ||payload.get("content_digest").and_then(Value::as_str)!=Some(digest_bytes(text.as_bytes()).as_ref())
                    ||event.causation_event_id.as_ref()!=last_part.as_ref() {
                    return Err(invalid("answer seal does not prove its exact immutable parts"));
                }
            }
        }
        let seal = seal.ok_or_else(||invalid("Answered message has no final publication proof"))?;
        if text.trim().is_empty() {return Err(invalid("Answered message is empty"));}
        let source = VoiceSpeechSourceRef {message_id:seal.event_id.as_ref().into(),revision:seal.seq,through_seq:terminal.seq};
        source.validate().map_err(invalid)?;
        segments.push((source,text));
    }
    if segments.is_empty()||Some(segments.iter().map(|(_,text)|text.as_str()).collect::<String>())!=public.result_text {
        return Err(invalid("answer source parts differ from the canonical result"));
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::engine_journal::{EngineJournalWrite,EngineTurnJournal};

    async fn record(journal:&EngineTurnJournal,event:AgentEngineEvent) {
        journal.append(serde_json::to_string(&event).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    }

    async fn completed(rejected:bool)->(ChatCausalityFacts,TurnReceipt) {
        let (journal,pool)=crate::router::engine_journal::test_fixture().await;
        record(&journal,AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).await;
        record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"exact recorded answer".into(),source:None}).await;
        if rejected {
            record(&journal,AgentEngineEvent::ModelResponseRejected {step:1,discarded_tool_call_ids:vec![],continuation:true,
                tool_hint:None,rejection:None,feedback:None}).await;
        }
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
        journal.append(serde_json::to_string(&AgentEngineEvent::TurnCompleted {model_steps:1,
            finish_reason:nomifun_chat_model_broker::ChatFinishReason::Completed}).unwrap(),None,EngineJournalWrite::Terminal).await.unwrap();
        let store=nomifun_agent_session::AgentSessionStore::from_pool(pool).await.unwrap();
        let session=AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let facts=store.turn_output_facts(&session,&"turn".into()).await.unwrap();
        let receipt=store.read_turn_receipt(&session,&"turn".into()).await.unwrap();
        (facts,receipt)
    }

    #[tokio::test]
    async fn saved_reply_requires_successful_answered_and_preserves_exact_seal_terminal_source() {
        let (facts,receipt)=completed(false).await;
        let source=saved_answer_segments(&facts,&receipt).unwrap();
        assert_eq!(source.len(),1);assert_eq!(source[0].1,"exact recorded answer");
        let seal=facts.events.iter().find(|event|event.kind.0=="message/completed").unwrap();
        assert_eq!(source[0].0.message_id,seal.event_id.as_ref());
        assert_eq!(source[0].0.revision,seal.seq);
        assert_eq!(source[0].0.through_seq,receipt.terminal_event.as_ref().unwrap().seq);
        assert!(source[0].0.through_seq>source[0].0.revision);
        // Re-reading a source is allowed; the explicit owner's distinct media
        // request controls replay deduplication, not a fabricated source ID.
        assert_eq!(saved_answer_segments(&facts,&receipt).unwrap(),source);
        let owner=facts.session.owner_ref.clone();
        validate_saved_answer_context(&facts,&receipt,&owner,0).unwrap();
        let mut rebound=facts.clone();rebound.session.agent_binding.binding_version+=1;
        assert!(validate_saved_answer_context(&rebound,&receipt,&owner,0).is_err());
        assert!(validate_saved_answer_context(&facts,&receipt,&owner,receipt.started_event.as_ref().unwrap().seq).is_err());
        let foreign=PrincipalRef {principal_kind:"user".into(),principal_id:"another owner".into()};
        assert!(validate_saved_answer_context(&facts,&receipt,&foreign,0).is_err());
    }

    #[tokio::test]
    async fn saved_reply_rejects_failed_paused_and_cancelled_turns_even_with_public_draft_text() {
        for terminal in [
            AgentEngineEvent::TurnFailed {model_steps:1,message:"fixture failure".into(),failure:None},
            AgentEngineEvent::TurnPaused {model_steps:1,reason:"HOST_CLEANUP_UNPROVEN".into()},
            AgentEngineEvent::TurnCancelled {model_steps:1},
        ] {
            let (journal,pool)=crate::router::engine_journal::test_fixture().await;
            record(&journal,AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).await;
            record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"draft that must never be replayed".into(),source:None}).await;
            journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
            journal.append(serde_json::to_string(&terminal).unwrap(),None,EngineJournalWrite::Terminal).await.unwrap();
            let store=nomifun_agent_session::AgentSessionStore::from_pool(pool).await.unwrap();
            let session=AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
            let facts=store.turn_output_facts(&session,&"turn".into()).await.unwrap();
            let receipt=store.read_turn_receipt(&session,&"turn".into()).await.unwrap();
            assert!(saved_answer_segments(&facts,&receipt).is_err(),"{terminal:?}");
        }
    }

    #[tokio::test]
    async fn saved_reply_rejects_withdrawn_output_and_corrupt_immutable_seal() {
        let (withdrawn,receipt)=completed(true).await;
        assert!(saved_answer_segments(&withdrawn,&receipt).is_err());
        let (mut corrupt,receipt)=completed(false).await;
        let seal=corrupt.events.iter().find(|event|event.kind.0=="message/completed").unwrap();
        corrupt.event_payloads.get_mut(seal.event_id.as_ref()).unwrap()["content_digest"]=json!("a".repeat(64));
        assert!(saved_answer_segments(&corrupt,&receipt).is_err());
    }

    #[test]
    fn saved_desktop_reply_keeps_its_selected_session_and_same_companion_authority() {
        let desktop=nomifun_agent_contracts::CompanionSessionOrigin {companion_id:"partner-a".into(),
            role:CompanionSessionRole::Companion,robot_id:None};
        assert!(robot_playback_scope_matches(&desktop,"partner-a","current-device"));
        assert!(!robot_playback_scope_matches(&desktop,"partner-b","current-device"));
        let physical=nomifun_agent_contracts::CompanionSessionOrigin {role:CompanionSessionRole::Robot,
            robot_id:Some("device-a".into()),..desktop};
        assert!(robot_playback_scope_matches(&physical,"partner-a","device-a"));
        assert!(!robot_playback_scope_matches(&physical,"partner-a","device-b"));
    }
}

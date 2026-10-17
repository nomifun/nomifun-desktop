//! One host-authored follow-up Turn for an exact, already committed desktop
//! report. The existing Execution lead-report outbox owns retries; this module
//! owns no queue, checkpoint, transcript or playback receipt ledger.

use super::{NomiCoreSessionOwner, canonical_companion_origin, agent_session_store_error};
use async_trait::async_trait;
use nomifun_agent_contracts::{AgentInputSource, AgentSessionId, AgentTurnOutcome,
    CompanionSessionOrigin, CompanionSessionRole, OperationId, PrincipalRef, SessionEventAppend,
    SemanticSessionEventDraft,SessionEventKind,SessionEventPayloadRef,StrictJsonValue,digest_bytes};
use nomifun_agent_execution::DirectedDelegationReport;
use nomifun_agent_session::TurnReceiptStatus;
use nomifun_api_types::SendMessageRequest;
use nomifun_common::AppError;
use nomifun_db::sqlx;
use nomifun_voice_contracts::VoiceSpeechProjection;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

#[async_trait]
pub(crate) trait DirectedReportDeliveryPort: Send + Sync {
    async fn deliver_report(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<(), AppError>;
    async fn deliver_input_required(&self,_owner_id:&str,_input:&nomifun_agent_execution::DirectedDelegationInput)
        ->Result<(),AppError>{Err(AppError::Conflict("Robot question delivery owner is unavailable".into()))}
    async fn hold_input_connection(&self,_owner_id:&str,_companion_id:&str,_robot_id:&str,_connection_id:&str)
        ->Result<Option<nomifun_robot::registry::RobotActionLease>,AppError>{Ok(None)}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReport {
    content: String,
    origin: String,
    operation_id: String,
    delegation_report: DirectedDelegationReport,
}

/// Construction is restricted to the canonical source reader below. Transport
/// code cannot select another Session, device or connection from model text.
pub(crate) struct PreparedDirectedReport {
    owner_id: String,
    session_id: AgentSessionId,
    report_message_id: String,
    report_seq: u64,
    report: StoredReport,
    companion_id: String,
    robot_id: String,
    connection_id: String,
}

impl PreparedDirectedReport {
    pub(crate) fn companion_id(&self) -> &str { &self.companion_id }
    pub(crate) fn robot_id(&self) -> &str { &self.robot_id }
    pub(crate) fn connection_id(&self) -> &str { &self.connection_id }
    fn input_key(&self) -> String {
        format!("directed-result:{}:{}",self.report.delegation_report.execution_id,
            self.report.delegation_report.execution_event_sequence)
    }
}

fn invalid(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("directed result delivery: {message}"))
}

fn bounded(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) { end -= 1; }
    &text[..end]
}

const RESULT_FAILURE_NOTICE:&str="电脑端任务的结果已保存，但机器人回应失败，请在桌面查看。";

fn authenticated_robot_source(scope: &CompanionSessionOrigin, value: Value) -> Result<(String,String,String),AppError> {
    let AgentInputSource::AuthenticatedRobot { companion_id,robot_id,connection_id,.. } =
        serde_json::from_value(value).map_err(invalid)? else {
            return Err(invalid("a host-result Turn cannot create another result callback"));
        };
    if scope.role!=CompanionSessionRole::Robot || companion_id!=scope.companion_id
        || scope.robot_id.as_deref()!=Some(robot_id.as_str()) || connection_id.trim().is_empty() {
        return Err(invalid("source device differs from canonical Robot affiliation"));
    }
    Ok((companion_id,robot_id,connection_id))
}

impl NomiCoreSessionOwner {
    pub(crate) fn install_directed_report_delivery(&self, port: &Arc<dyn DirectedReportDeliveryPort>) -> Result<(), String> {
        self.directed_report_delivery.set(Arc::downgrade(port))
            .map_err(|_| "Directed report delivery owner was already installed".into())
    }

    pub(super) async fn deliver_committed_directed_report(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<(), AppError> {
        let port = self.directed_report_delivery.get().and_then(Weak::upgrade)
            .ok_or_else(|| invalid("Robot delivery owner is unavailable"))?;
        port.deliver_report(owner_id,session_id,operation_id,report_message_id).await
    }

    pub(crate) async fn prepare_directed_report_delivery(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<Option<PreparedDirectedReport>, AppError> {
        let session = AgentSessionId::from(session_id);
        let principal = PrincipalRef { principal_kind:"user".into(),principal_id:owner_id.into() };
        let observation = self.canonical.get(&principal,&session).await?;
        let scope = canonical_companion_origin(owner_id,&observation)?
            .filter(|scope|scope.role==CompanionSessionRole::Robot)
            .ok_or_else(||invalid("report Session is no longer the canonical Robot role"))?;
        let row: Option<(i64,String)> = sqlx::query_as(
            "SELECT seq,inline_json FROM agent_events WHERE session_id=? AND event_id=? \
             AND producer_id='session_api' AND kind='message/assistant-projected' AND kind_version=1 \
             AND idempotency_key=? AND payload_id IS NULL")
            .bind(session_id).bind(report_message_id)
            .bind(format!("agent-execution-projection:{operation_id}"))
            .fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let (seq,payload) = row.ok_or_else(||invalid("exact committed report is missing"))?;
        let report: StoredReport = serde_json::from_str(&payload).map_err(invalid)?;
        if report.origin!="agent_execution_report" || report.operation_id!=operation_id
            || report.delegation_report.source.agent_session_id!=session_id {
            return Err(invalid("report identity differs from its original source"));
        }
        let source_operation = OperationId::from(report.delegation_report.source.turn_operation_id.clone());
        let receipt = self.canonical.store().read_turn_receipt(&session,&source_operation).await.map_err(agent_session_store_error)?;
        let start = receipt.started_event.as_ref().ok_or_else(||invalid("source has no exact Turn start"))?;
        if receipt.status!=TurnReceiptStatus::Completed { return Ok(None); }
        let context_floor: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared' AND kind_version=1")
            .bind(session_id).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if u64::try_from(context_floor).map_err(invalid)? > start.seq { return Ok(None); }
        let (_,accepted)=self.canonical.store().turn_accepted_input(&session,&source_operation).await.map_err(agent_session_store_error)?;
        let Some(original) = accepted.0.get("input_source").filter(|source|!source.is_null()).cloned()
            else { return Ok(None); }; // ordinary UI input has no physical reply authority
        let (companion_id,robot_id,connection_id)=authenticated_robot_source(&scope,original)?;
        Ok(Some(PreparedDirectedReport { owner_id:owner_id.into(),session_id:session,
            report_message_id:report_message_id.into(),report_seq:u64::try_from(seq).map_err(invalid)?,
            report,companion_id,robot_id,connection_id }))
    }

    /// Each retry revalidates the same source under the Session input fence.
    /// Exact-key replays query the first accepted Turn, including after a crash.
    pub(crate) async fn run_directed_report_result(&self, prepared: &PreparedDirectedReport)
        -> Result<Option<Vec<VoiceSpeechProjection>>, AppError> {
        let _fence = self.session_operation_lock(prepared.session_id.as_ref()).write_owned().await;
        let Some(current) = self.prepare_directed_report_delivery(&prepared.owner_id,prepared.session_id.as_ref(),
            &prepared.report.operation_id,&prepared.report_message_id).await? else { return Ok(None); };
        if current.report_seq!=prepared.report_seq || current.connection_id!=prepared.connection_id {
            return Err(invalid("prepared callback source changed"));
        }
        let key=prepared.input_key();
        let operation=Self::turn_operation_id(&prepared.owner_id,prepared.session_id.as_ref(),&key);
        let existing=self.canonical.store().read_turn_receipt(&prepared.session_id,&operation).await.map_err(agent_session_store_error)?;
        if existing.status==TurnReceiptStatus::NotFound {
            let observation=self.canonical.get(&PrincipalRef {principal_kind:"user".into(),principal_id:prepared.owner_id.clone()},&prepared.session_id).await?;
            if observation.head.status!="ready" || observation.head.active_turn_id.is_some()
                || self.canonical.store().has_unsettled_effects(&prepared.session_id).await.map_err(agent_session_store_error)? {
                return Err(invalid("Robot Session is busy; result callback remains in its Execution outbox"));
            }
        }
        let content=json!({"kind":"directed_delegation_result","execution_id":prepared.report.delegation_report.execution_id,
            "status":prepared.report.delegation_report.status,"report_message_id":prepared.report_message_id,
            "report_seq":prepared.report_seq,"result":bounded(&prepared.report.content,8192),
            "result_truncated":prepared.report.content.len()>8192}).to_string();
        let source=AgentInputSource::DirectedDelegationResult {report_message_id:prepared.report_message_id.clone(),
            report_seq:prepared.report_seq,execution_id:prepared.report.delegation_report.execution_id.clone(),
            source_turn_operation_id:prepared.report.delegation_report.source.turn_operation_id.clone().into()};
        let request=SendMessageRequest {content,files:vec![],inject_skills:vec![],hidden:true,
            origin:Some("directed_delegation_result".into()),channel_platform:Some("robot".into())};
        let delivery=self.dispatch_canonical_turn_locked(&prepared.owner_id,&prepared.session_id,&key,request,
            false,None,None,None,false,None,Some(source)).await?;
        if !delivery.completed { return Err(invalid("exact result Turn is accepted and still running")); }
        let facts=self.canonical.store().turn_output_facts(&prepared.session_id,&operation).await.map_err(agent_session_store_error)?;
        let receipt=self.canonical.store().read_turn_receipt(&prepared.session_id,&operation).await.map_err(agent_session_store_error)?;
        if receipt.status==TurnReceiptStatus::Cancelled { return Ok(None); }
        let canonical=nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,true)?;
        if delivery.result_ok!=Some(true)||!matches!(&canonical.turn_outcome,Some(AgentTurnOutcome::Answered{..})) {
            return self.directed_result_failure_notice(prepared,&operation,&receipt).await.map(|speech|Some(vec![speech]));
        }
        let Some(AgentTurnOutcome::Answered {message_ids})=canonical.turn_outcome else {unreachable!("public answer checked above")};
        let mut speech=Vec::new();
        for id in message_ids {
            let mut text=String::new(); let mut revision=None;
            for event in &facts.events {
                if event.correlation_id.as_ref()!=id { continue; }
                let Some(payload)=facts.event_payloads.get(event.event_id.as_ref()) else { return Err(invalid("answer source payload is missing")); };
                match event.kind.0.as_str() {
                    "message/content-part"=>text.push_str(payload.get("content").and_then(Value::as_str).ok_or_else(||invalid("answer part is invalid"))?),
                    "message/completed"=>{
                        if payload.get("content_digest").and_then(Value::as_str)!=Some(digest_bytes(text.as_bytes()).as_ref()) {
                            return Err(invalid("answer source digest differs from committed parts"));
                        }
                        revision=Some(event.seq);
                    }, _=>{}
                }
            }
            let revision=revision.ok_or_else(||invalid("answer source has no canonical seal"))?;
            if !text.trim().is_empty() { speech.push(VoiceSpeechProjection {message_id:id,revision,through_seq:revision,text}); }
        }
        Ok(Some(speech))
    }

    /// The caller already holds the Session fence. This is a bounded Host
    /// notice from an exact committed failure, not another model/tool attempt.
    async fn directed_result_failure_notice(&self,prepared:&PreparedDirectedReport,operation:&OperationId,
        receipt:&nomifun_agent_session::TurnReceipt)->Result<VoiceSpeechProjection,AppError>{
        let terminal=receipt.terminal_event.as_ref().ok_or_else(||invalid("result failure has no committed terminal"))?;
        let payload=json!({"content":RESULT_FAILURE_NOTICE,"origin":"directed_result_failure",
            "operation_id":format!("{}:failure-notice",prepared.input_key()),
            "report_message_id":prepared.report_message_id,"result_turn_operation_id":operation,
            "result_terminal_event_id":terminal.event_id});
        let key=format!("directed-result-failure:{}",prepared.report_message_id);
        let existing:Option<(String,i64,String)>=sqlx::query_as("SELECT event_id,seq,inline_json FROM agent_events WHERE session_id=? AND producer_id='session_api' AND idempotency_key=? AND kind='message/assistant-projected' AND kind_version=1")
            .bind(prepared.session_id.as_ref()).bind(&key).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let (message_id,revision)=if let Some((id,seq,stored))=existing{
            if serde_json::from_str::<Value>(&stored).map_err(invalid)?!=payload{return Err(invalid("committed result failure notice changed"));}
            (id,u64::try_from(seq).map_err(invalid)?)
        }else{
            let id=uuid::Uuid::now_v7().to_string();
            let appended=self.canonical.store().append_event(&SessionEventAppend{agent_session_id:prepared.session_id.clone(),event_id:id.clone().into(),producer_id:"session_api".into(),idempotency_key:key.into(),
                semantic_event:SemanticSessionEventDraft{kind:SessionEventKind("message/assistant-projected".into()),kind_version:1,correlation_id:id.clone().into(),
                    causation_event_id:Some(terminal.event_id.clone()),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))}}).await.map_err(agent_session_store_error)?;
            self.user_events.send_to_user(&prepared.owner_id,Self::canonical_projected_assistant_wire_event(&prepared.session_id,&id,RESULT_FAILURE_NOTICE));
            (id,appended.cursor.seq)
        };
        Ok(VoiceSpeechProjection{message_id,revision,through_seq:revision,text:RESULT_FAILURE_NOTICE.into()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn result_callback_keeps_the_original_authenticated_device_and_connection() {
        let scope=CompanionSessionOrigin {companion_id:"persona".into(),role:CompanionSessionRole::Robot,robot_id:Some("robot-a".into())};
        let input=json!({"kind":"authenticated_robot","companion_id":"persona","robot_id":"robot-a","connection_id":"original-socket"});
        assert_eq!(authenticated_robot_source(&scope,input.clone()).unwrap(),
            ("persona".into(),"robot-a".into(),"original-socket".into()));
        for field in ["companion_id","robot_id","connection_id"] {
            let mut invalid=input.clone();invalid[field]=json!(if field=="connection_id" {""} else {"another"});
            assert!(authenticated_robot_source(&scope,invalid).is_err());
        }
        assert!(authenticated_robot_source(&scope,json!({"kind":"directed_delegation_result",
            "report_message_id":"report","report_seq":5,"execution_id":"execution",
            "source_turn_operation_id":"turn"})).is_err());
    }
}

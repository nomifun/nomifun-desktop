//! Deliver a committed background result through its original authenticated entry.
//! The canonical report is already public output: delivery never creates a
//! model Turn, callback input, polling loop, or second content ledger.

use super::{NomiCoreSessionOwner, canonical_companion_origin, agent_session_store_error,canonical_admission_binding_ref};
use async_trait::async_trait;
use nomifun_agent_contracts::{AgentInputSource, AgentSessionId,
    CompanionSessionOrigin, CompanionSessionRole, OperationId, PrincipalRef,AgentHandoffBindingRefV1};
use nomifun_agent_execution::AgentExecutionReport;
use nomifun_agent_session::TurnReceiptStatus;
use nomifun_common::AppError;
use nomifun_db::sqlx;
use nomifun_voice_contracts::VoiceSpeechProjection;
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, Weak};

#[async_trait]
pub(crate) trait ExecutionReportDeliveryPort: Send + Sync {
    async fn deliver_report(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<(), AppError>;
    async fn deliver_input_required(&self,_owner_id:&str,_input:&nomifun_agent_execution::AgentExecutionInput)
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
    delegation_report: AgentExecutionReport,
}

/// Construction is restricted to the canonical source reader below. Transport
/// code cannot select another Session, device or connection from model text.
pub(crate) struct PreparedExecutionReport {
    owner_id: String,
    session_id: AgentSessionId,
    report_message_id: String,
    report_seq: u64,
    binding_version: u64,
    context_floor: u64,
    report: StoredReport,
    companion_id: String,
    robot_id: String,
    connection_id: String,
}

impl PreparedExecutionReport {
    pub(crate) fn companion_id(&self) -> &str { &self.companion_id }
    pub(crate) fn robot_id(&self) -> &str { &self.robot_id }
    pub(crate) fn connection_id(&self) -> &str { &self.connection_id }
    pub(crate) fn playback_context(&self) -> nomifun_robot::registry::RobotPlaybackContext {
        nomifun_robot::registry::RobotPlaybackContext {session_id:self.session_id.as_ref().into(),
            input_context:nomifun_robot::services::RobotInputContext {binding_version:self.binding_version,
                context_floor:self.context_floor,presented_question_id:None,work_operation_id:None,execution_generation:None,
                background_work:None,background_work_ambiguous:false}}
    }

}

fn invalid(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Execution result delivery: {message}"))
}

/// Snapshot and lifecycle revocation are read from the original accepted
/// input and canonical events. A resource-only rebind may retain the Snapshot,
/// so any later binding transition still revokes automatic physical delivery.
pub(super) fn reply_authority_matches(started: &Value, current: &AgentHandoffBindingRefV1,
    source_start: u64, revocation_floor: u64) -> Result<bool, AppError> {
    // Earlier accepted facts lack the resource binding version. They remain
    // readable history but cannot grant automatic playback after a resource
    // CAS that preserved the exact model Snapshot.
    let Some(binding)=canonical_admission_binding_ref(started)? else {return Ok(false);};
    Ok(revocation_floor <= source_start && binding == *current)
}

fn authenticated_robot_source(scope: &CompanionSessionOrigin, value: Value) -> Result<(String,String,String),AppError> {
    let AgentInputSource::AuthenticatedRobot { companion_id,robot_id,connection_id,.. } =
        serde_json::from_value(value).map_err(invalid)?;
    if scope.role!=CompanionSessionRole::Robot || companion_id!=scope.companion_id
        || scope.robot_id.as_deref()!=Some(robot_id.as_str()) || connection_id.trim().is_empty() {
        return Err(invalid("source device differs from canonical Robot affiliation"));
    }
    Ok((companion_id,robot_id,connection_id))
}

impl NomiCoreSessionOwner {
    pub(crate) fn install_execution_report_delivery(&self, port: &Arc<dyn ExecutionReportDeliveryPort>) -> Result<(), String> {
        self.execution_report_delivery.set(Arc::downgrade(port))
            .map_err(|_| "Execution report delivery owner was already installed".into())
    }

    pub(super) async fn deliver_committed_execution_report(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<(), AppError> {
        let port = self.execution_report_delivery.get().and_then(Weak::upgrade)
            .ok_or_else(|| invalid("Robot delivery owner is unavailable"))?;
        port.deliver_report(owner_id,session_id,operation_id,report_message_id).await
    }

    pub(crate) async fn prepare_execution_report_delivery(&self, owner_id: &str, session_id: &str,
        operation_id: &str, report_message_id: &str) -> Result<Option<PreparedExecutionReport>, AppError> {
        let session = AgentSessionId::from(session_id);
        let principal = PrincipalRef { principal_kind:"user".into(),principal_id:owner_id.into() };
        let observation = self.canonical.get(&principal,&session).await?;
        let Some(scope) = canonical_companion_origin(owner_id,&observation)?
            .filter(|scope|scope.role==CompanionSessionRole::Robot) else { return Ok(None); };
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
        let (revocation_floor,context_floor): (i64,i64) = sqlx::query_as(
            "SELECT COALESCE(MAX(seq),0),COALESCE(MAX(CASE WHEN kind='context/cleared' THEN seq END),0) FROM agent_events WHERE session_id=? AND kind IN ('context/cleared','session/agent-binding-changed') AND kind_version=1")
            .bind(session_id).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let (_,accepted)=self.canonical.store().turn_accepted_input(&session,&source_operation).await.map_err(agent_session_store_error)?;
        let mut start_payload=self.canonical.store().resolved_event_payloads(&session,std::slice::from_ref(&start.event_id)).await.map_err(agent_session_store_error)?;
        let start_payload=start_payload.remove(start.event_id.as_ref()).ok_or_else(||invalid("source claim admission is missing"))?;
        if !reply_authority_matches(&start_payload,&AgentHandoffBindingRefV1::from(&observation.session.agent_binding),
            start.seq,u64::try_from(revocation_floor).map_err(invalid)?)? { return Ok(None); }
        let Some(original) = accepted.0.get("input_source").filter(|source|!source.is_null()).cloned()
            else { return Ok(None); }; // ordinary UI input has no physical reply authority
        let (companion_id,robot_id,connection_id)=authenticated_robot_source(&scope,original)?;
        Ok(Some(PreparedExecutionReport { owner_id:owner_id.into(),session_id:session,
            report_message_id:report_message_id.into(),report_seq:u64::try_from(seq).map_err(invalid)?,
            binding_version:observation.session.agent_binding.binding_version,
            context_floor:u64::try_from(context_floor).map_err(invalid)?,
            report,companion_id,robot_id,connection_id }))
    }

    /// Revalidate the original report under the existing Session fence, then
    /// return its exact committed public text. A newer accepted user input is
    /// never borrowed as model context or authority for this delivery.
    pub(crate) async fn read_execution_report_speech(&self, prepared: &PreparedExecutionReport)
        -> Result<Option<Vec<VoiceSpeechProjection>>, AppError> {
        let _fence = self.session_operation_lock(prepared.session_id.as_ref()).write_owned().await;
        let Some(current) = self.prepare_execution_report_delivery(&prepared.owner_id,prepared.session_id.as_ref(),
            &prepared.report.operation_id,&prepared.report_message_id).await? else { return Ok(None); };
        if current.report_seq!=prepared.report_seq || current.connection_id!=prepared.connection_id
            || current.binding_version!=prepared.binding_version
            || current.context_floor!=prepared.context_floor
            || current.report.content!=prepared.report.content {
            return Err(invalid("prepared report source changed"));
        }
        Ok(Some(vec![VoiceSpeechProjection {
            message_id: prepared.report_message_id.clone(),
            revision: prepared.report_seq,
            through_seq: prepared.report_seq,
            text: prepared.report.content.clone(),
        }]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::ResolvedSnapshotRef;
    use serde_json::json;
    #[test]
    fn automatic_report_delivery_drops_changed_agent_resource_or_cleared_context() {
        let original=AgentHandoffBindingRefV1 {preset_revision_ref:nomifun_agent_contracts::PresetRevisionRef{preset_id:"original".into(),revision:1,revision_digest:"b".repeat(64).into()},
            resolved_snapshot_ref:ResolvedSnapshotRef {snapshot_id:"snapshot-original".into(),snapshot_digest:"a".repeat(64).into()},binding_version:1};
        let accepted=json!({"admission":{"resolved_snapshot_ref":original.resolved_snapshot_ref,"binding_ref":original}});
        assert!(reply_authority_matches(&accepted,&original,10,0).unwrap());
        assert!(reply_authority_matches(&accepted,&original,10,9).unwrap());
        // A resource-only transition keeps the same Snapshot but revokes the
        // earlier physical reply authority. Switching back cannot revive it.
        assert!(!reply_authority_matches(&accepted,&original,10,11).unwrap());
        let mut changed=original.clone();changed.binding_version+=1;
        assert!(!reply_authority_matches(&accepted,&changed,10,0).unwrap());
        assert!(!reply_authority_matches(&json!({"admission":{"resolved_snapshot_ref":original.resolved_snapshot_ref}}),&original,10,0).unwrap());
        assert!(reply_authority_matches(&json!({}),&original,10,0).is_err());
    }

    #[tokio::test]
    async fn committed_resource_rebind_revokes_original_report_without_rewriting_accepted_input() {
        use nomifun_agent_contracts::*;
        use nomifun_agent_session::{AgentSessionStore,CreateSessionRequest};
        use std::collections::{BTreeMap,BTreeSet};
        let database=nomifun_db::init_database_memory().await.unwrap();
        let store=AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
        let session=AgentSessionId::from(uuid::Uuid::now_v7().to_string());
        let principal=PrincipalRef{principal_kind:"user".into(),principal_id:uuid::Uuid::now_v7().to_string()};
        let binding=AgentBindingValue {
            preset_revision_ref:PresetRevisionRef{preset_id:"companion-fixture".into(),revision:1,revision_digest:"a".repeat(64).into()},
            resolved_snapshot_ref:ResolvedSnapshotRef{snapshot_id:"report-fixture".into(),snapshot_digest:"b".repeat(64).into()},
            typed_resource_bindings:vec![],binding_version:1,
        };
        let created=store.create_session(CreateSessionRequest::new(AgentSessionLiveRecord {
            agent_session_id:session.clone(),owner_ref:principal.clone(),metadata:AgentSessionMetadata {
                purpose:Default::default(),app_project_id:None,companion:None,title:None,archived:false,pinned:false,reasoning_effort:None,
            },agent_binding:binding.clone(),remote_binding_provenance:None,parent_session_id:None,fork_base_payload_id:None,next_seq:1,
        },1,"opening","session_api".into(),"report-open".into(),"report-open".into())).await.unwrap();
        let append=|id:&str,kind:&str,correlation:&str,cause:EventId,payload:Value|SessionEventAppend {
            agent_session_id:session.clone(),event_id:id.into(),producer_id:"runtime_supervisor".into(),idempotency_key:id.into(),
            semantic_event:SemanticSessionEventDraft{kind:SessionEventKind(kind.into()),kind_version:1,correlation_id:correlation.into(),
                causation_event_id:Some(cause),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))},
        };
        store.append_event(&append("report-ready","session/ready","report-ready",created.opening_ack.event_id,json!({}))).await.unwrap();
        let input=json!({"content":"produce a background result",
            "input_source":{"kind":"authenticated_robot","companion_id":"persona","robot_id":"device","connection_id":"original"}});
        let operation=OperationId::from("report-parent");
        let accepted=store.accept_session_input(&principal,&session,"session_api".into(),"report-parent".into(),StrictJsonValue(input.clone())).await.unwrap();
        let started=store.claim_session_input_as_turn(&principal,&session,&accepted.ack.unwrap().event_id,&operation,
            StrictJsonValue(json!({"resolved_snapshot_ref":binding.resolved_snapshot_ref,"binding_ref":AgentHandoffBindingRefV1::from(&binding)}))).await.unwrap();
        let source_start=started.cursor.seq;
        let start_event=started.record.unwrap();
        store.append_turn_terminal(&append("report-parent-completed","turn/completed",operation.as_ref(),start_event.event_id.clone(),json!({})),&operation).await.unwrap();
        let (_,accepted)=store.turn_accepted_input(&session,&operation).await.unwrap();
        assert!(accepted.0.get("admission").is_none(),"permanent input does not mirror its execution admission");
        let mut payloads=store.resolved_event_payloads(&session,&[start_event.event_id.clone()]).await.unwrap();
        let source_admission=payloads.remove(start_event.event_id.as_ref()).unwrap();
        assert!(reply_authority_matches(&source_admission,&AgentHandoffBindingRefV1::from(&binding),source_start,0).unwrap());
        let mut replacement=binding.clone();replacement.binding_version+=1;
        replacement.typed_resource_bindings.push(TypedResourceBinding{binding_id:"replacement-device".into(),resource_kind:"robot".into(),
            resource_id:"different-device".into(),owner_id:principal.principal_id.clone(),operations:BTreeSet::from(["motion".into()]),
            connection_config_ref:None,typed_parameters:BTreeMap::new()});
        store.replace_session_resource_bindings(&principal,&session,&binding,replacement.clone(),"robot").await.unwrap();
        let current=store.get_live_session(&session).await.unwrap();
        assert_eq!(current.agent_binding.resolved_snapshot_ref,binding.resolved_snapshot_ref,"resource changes can retain the model Snapshot");
        assert_eq!(current.agent_binding.binding_version,2);
        let transitions=store.agent_binding_transitions(&session).await.unwrap();
        assert!(transitions.is_empty(),"the resource CAS may retain its Snapshot without an Agent transition event");
        assert!(!reply_authority_matches(&source_admission,&AgentHandoffBindingRefV1::from(&current.agent_binding),source_start,0).unwrap());
        assert_eq!(store.turn_accepted_input(&session,&operation).await.unwrap().1.0,input,"a rebind cannot rewrite original reply authority");
    }

    #[test]
    fn result_delivery_keeps_the_original_authenticated_device_and_connection() {
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

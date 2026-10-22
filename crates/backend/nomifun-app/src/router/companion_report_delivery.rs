//! Original authenticated reply authority and execution-question delivery.
//! Settled child results return through the parent Runtime; this module only
//! retains question/control delivery and its exact binding revocation rules.

use super::{NomiCoreSessionOwner,canonical_admission_binding_ref};
use async_trait::async_trait;
use nomifun_agent_contracts::AgentHandoffBindingRefV1;
use nomifun_common::AppError;
use serde_json::Value;
use std::sync::Arc;
#[async_trait]
pub(crate) trait ExecutionReportDeliveryPort: Send + Sync {
    async fn deliver_input_required(&self,_owner_id:&str,_input:&nomifun_agent_execution::AgentExecutionInput)
        ->Result<(),AppError>{Err(AppError::Conflict("Robot question delivery owner is unavailable".into()))}
    async fn hold_input_connection(&self,_owner_id:&str,_companion_id:&str,_robot_id:&str,_connection_id:&str)
        ->Result<Option<nomifun_robot::registry::RobotActionLease>,AppError>{Ok(None)}
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

impl NomiCoreSessionOwner {
    pub(crate) fn install_execution_report_delivery(&self, port: &Arc<dyn ExecutionReportDeliveryPort>) -> Result<(), String> {
        self.execution_report_delivery.set(Arc::downgrade(port))
            .map_err(|_| "Execution report delivery owner was already installed".into())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::ResolvedSnapshotRef;
    use serde_json::json;
    #[test]
    fn reply_authority_drops_changed_agent_resource_or_cleared_context() {
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
    async fn committed_resource_rebind_revokes_original_reply_without_rewriting_accepted_input() {
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

}

//! The live and recovered Channel delivery paths share the same typed tool
//! presentation. This module reads no store and grants no execution authority.
use std::collections::BTreeMap;
use nomifun_agent_runtime::{AgentEngineEvent,AgentEngineError};
use crate::protocol::events::{AgentStreamEvent,ToolCallEventData,ToolCallStatus};

pub(crate) struct PendingToolProjection {
    public:ToolCallEventData,
    arguments:serde_json::Value,
}

pub(crate) fn is_tool_projection_event(event:&AgentEngineEvent)->bool {
    matches!(event,AgentEngineEvent::ToolCallCompleted {..}|AgentEngineEvent::ToolStarted {..}|AgentEngineEvent::ToolCompleted {..}
        |AgentEngineEvent::ModelOutputTruncated {..}|AgentEngineEvent::ModelResponseRejected {..}|AgentEngineEvent::VoiceModelStepSuperseded {..})
}

pub(crate) fn project_tool_event(calls:&mut BTreeMap<String,PendingToolProjection>,event:AgentEngineEvent,event_seq:Option<u64>)
    ->Result<Option<ToolCallEventData>,AgentEngineError> {
    Ok(match event {
            AgentEngineEvent::ToolCallCompleted { call, .. } => {
                calls.insert(
                    call.call_id.as_ref().to_owned(),
                    PendingToolProjection {
                        public: ToolCallEventData {
                            identity: Default::default(),
                            call_id: call.call_id.as_ref().to_owned(),
                            description: nomifun_engine_core::tool_display_target(None, None, &call.name, &call.arguments.0),
                            name: call.name,
                            status: ToolCallStatus::Running,
                            artifacts: Vec::new(),
                        },
                        arguments: call.arguments.0,
                    },
                );
                None
            }
            AgentEngineEvent::ModelOutputTruncated { discarded_tool_call_ids, .. }
            | AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, .. }
            | AgentEngineEvent::VoiceModelStepSuperseded {discarded_tool_call_ids,..} => {
                for id in discarded_tool_call_ids { calls.remove(id.as_ref()); }
                None
            }
            AgentEngineEvent::ToolStarted { call_id, capability_id, action_id, .. } => {
                calls.get_mut(call_id.as_ref())
                    .filter(|call| !call.public.call_id.starts_with("agent-instructions:"))
                    .filter(|call| call.public.name != nomifun_agent_runtime::FINISH_WITHOUT_REPLY_TOOL_NAME)
                    .map(|pending| {
                        let call = &mut pending.public;
                        call.identity.capability_id = Some(capability_id.as_ref().to_owned());
                        call.identity.action_id = Some(action_id.as_ref().to_owned());
                        call.description = nomifun_engine_core::tool_display_target(
                            Some(capability_id.as_ref()), Some(action_id.as_ref()), &call.name, &pending.arguments);
                        call.identity.display.operation = nomifun_engine_core::tool_display_operation(Some(capability_id.as_ref()), &pending.arguments);
                        call.clone()
                    })
            }
            AgentEngineEvent::ToolCompleted { result, runtime_hold, .. } => {
                let pending = calls.remove(result.call_id.as_ref())
                    .ok_or_else(|| {
                        AgentEngineError::InvalidModelEvent(
                            "tool result has no admitted call".to_owned(),
                        )
                    })?;
                let mut call = pending.public;
                // These are engine-owned instruction preflight reads, not
                // model-selected work. Keep their durable observations but do
                // not flood the conversation with synthetic read/error rows.
                if call.call_id.starts_with("agent-instructions:")
                    || call.name == nomifun_agent_runtime::FINISH_WITHOUT_REPLY_TOOL_NAME {
                    return Ok(None);
                }
                call.status = if result.is_error {
                    ToolCallStatus::Error
                } else {
                    ToolCallStatus::Completed
                };
                call.identity.result_seq = event_seq;
                call.identity.runtime_hold = runtime_hold;
                call.identity.permission_required = nomifun_engine_core::project_system_permission_required(
                    call.identity.capability_id.as_deref(), call.identity.action_id.as_deref(),
                    result.is_error, &result.output_text(),
                );
                call.identity.error_code = call.identity.permission_required.as_ref()
                    .map(|_| nomifun_engine_core::SYSTEM_PERMISSION_REQUIRED.to_owned());
                let operation = call.identity.display.operation.take();
                call.identity.display = nomifun_engine_core::tool_result_display(
                    call.identity.capability_id.as_deref(), call.identity.action_id.as_deref(), &result);
                call.identity.display.operation = operation;
                if call.identity.runtime_hold.is_some() || call.identity.permission_required.is_some() { call.identity.display.error = None; }
                if call.identity.display.cancelled { call.status = ToolCallStatus::Canceled; }
                call.identity.display.denied_stop_conversation_id = nomifun_engine_core::tool_denied_stop_target(
                    &call.name, &pending.arguments, &result);
                if let Some(target) = nomifun_engine_core::tool_result_target(
                    call.identity.capability_id.as_deref(), call.identity.action_id.as_deref(), &result) {
                    call.description = Some(target);
                }
                Some(call)
            }
        _=>None,
    })
}

/// Project only delivery-bearing, already committed tool results from one
/// authenticated exact Turn window. Callers supply the original typed native
/// events and canonical sequence, never text or current tool/provider state.
/// The output must still pass the ordinary Channel relay's upload validation.
pub fn project_canonical_tool_delivery(observations:&[(u64,AgentEngineEvent)])
    ->Result<Vec<AgentStreamEvent>,AgentEngineError> {
    if observations.len()>1_000_000 {return Err(AgentEngineError::InvalidContract("canonical tool delivery event budget exceeded".into()));}
    let mut previous=0;let mut calls=BTreeMap::new();let mut output=Vec::new();
    for(seq,event)in observations {
        if *seq<=previous {return Err(AgentEngineError::InvalidContract("canonical tool delivery sequence is not strictly increasing".into()));}
        previous=*seq;
        if let Some(call)=project_tool_event(&mut calls,event.clone(),Some(*seq))?
            && call.status==ToolCallStatus::Completed
            && (!call.artifacts.is_empty() || !call.identity.display.result_asset_ids.is_empty()) {
            output.push(AgentStreamEvent::ToolCall(call));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_runtime::AgentToolResult;
    const ASSET:&str="0190f5fe-7c00-7a00-8000-000000000081";
    fn observations(capability:&str,error:bool)->Vec<(u64,AgentEngineEvent)> {
        vec![(1,AgentEngineEvent::ToolCallCompleted {step:1,call:nomifun_chat_model_broker::ChatToolCall {call_id:"media".into(),name:"generate_media".into(),
            arguments:nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"prompt":"PRIVATE_PROMPT"})),provider_metadata:None}}),
            (2,AgentEngineEvent::ToolStarted {step:1,call_id:"media".into(),capability_id:capability.into(),action_id:"creation.media/image".into()}),
            (3,AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,result:AgentToolResult::text("media".into(),
                serde_json::json!({"result_asset_ids":[ASSET],"content":"PRIVATE_TOOL_BODY"}).to_string(),error)})]
    }
    #[test]
    fn canonical_tool_delivery_reuses_live_typed_media_projection_without_inventing_artifacts() {
        let actual=project_canonical_tool_delivery(&observations("creation.media",false)).unwrap();
        let [AgentStreamEvent::ToolCall(call)]=actual.as_slice() else {panic!("one typed media result expected")};
        assert_eq!(call.identity.result_seq,Some(3));assert_eq!(call.identity.display.result_asset_ids,vec![ASSET]);
        assert!(call.artifacts.is_empty(),"the live SDK does not invent filesystem artifacts from tool text");
        let encoded=serde_json::to_string(call).unwrap();assert!(!encoded.contains("PRIVATE_TOOL_BODY"));assert!(!encoded.contains("PRIVATE_PROMPT"));
        let mut live=BTreeMap::new();let mut result=None;
        for(seq,event)in observations("creation.media",false) {result=project_tool_event(&mut live,event,Some(seq)).unwrap();}
        assert_eq!(serde_json::to_value(result.unwrap()).unwrap(),serde_json::to_value(call).unwrap());
    }
    #[test]
    fn canonical_tool_delivery_rejects_missing_admission_and_never_guesses_media_from_unrelated_or_failed_results() {
        assert!(project_canonical_tool_delivery(&observations("unrelated",false)).unwrap().is_empty());
        assert!(project_canonical_tool_delivery(&observations("creation.media",true)).unwrap().is_empty());
        let values=observations("creation.media",false);
        assert!(project_canonical_tool_delivery(&values[2..]).is_err());
        assert!(project_canonical_tool_delivery(&[values[1].clone(),values[0].clone()]).is_err());
    }
}

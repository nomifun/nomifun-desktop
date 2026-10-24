//! Intent hints read the admitted input's source and current task separately.
//! Role/background text cannot mint a fresh human delegation requirement.
use std::borrow::Cow;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnIntentSource {
    Human,
    DelegatedTask,
    HostTask,
}

/// A source scope already checked by the existing current-Attempt reader.
/// It permits task-envelope interpretation, not Turn admission or tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExecutionSourceScope {
    VerifiedDelegatedTask,
    NoDelegationSource,
}

/// A transient view of the existing canonical accepted input, never another
/// task/permission ledger. Callers supply the admitted receipt, not model text.
pub(super) struct TurnIntentInput<'a> {
    pub(super) source: TurnIntentSource,
    task: Cow<'a, str>,
}

impl<'a> TurnIntentInput<'a> {
    pub(super) fn from_accepted(input: &'a Value, event:&nomifun_agent_contracts::SessionEventRecord,
        owner:&nomifun_agent_contracts::PrincipalRef, scope:ExecutionSourceScope) -> Result<Self, &'static str> {
        let text=input.get("content").and_then(Value::as_str).ok_or("accepted input has no current task text")?;
        let delivery=super::runtime_attachments::delivery(input);
        if !delivery.is_object() {return Err("accepted input delivery metadata is not an object");}
        let origin=delivery.get("origin").filter(|value|!value.is_null())
            .map(|value|value.as_str().ok_or("accepted input origin is not a string")).transpose()?;
        let human_intake=nomifun_agent_session::is_session_input_event(event,owner);
        let source=match origin {
            None|Some(""|"idmm"|"robot") if input.get("delivery").is_none()
                && human_intake=>TurnIntentSource::Human,
            Some("agent_execution"|"agent_execution_decision"|"agent")
                if !human_intake&&scope==ExecutionSourceScope::VerifiedDelegatedTask=>TurnIntentSource::DelegatedTask,
            _=>TurnIntentSource::HostTask,
        };
        let task=if source==TurnIntentSource::DelegatedTask {
            // Both the original internal input and its existing Execution
            // source scope are required. A client's origin string alone does
            // not turn a document into a delegated task envelope.
            let envelope=serde_json::from_str::<Value>(text).ok();
            match envelope.as_ref().filter(|value|value.get("task_brief").is_some_and(Value::is_string))
                .and_then(|value|value.get("step_spec")).and_then(Value::as_str) {
                Some(step)=>Cow::Owned(step.to_owned()),None=>Cow::Borrowed(text),
            }
        } else {Cow::Borrowed(text)};
        Ok(Self {source,task})
    }

    pub(super) fn task(&self)->&str {&self.task}

    pub(super) fn human_request(&self)->Option<&str> {
        (self.source==TurnIntentSource::Human).then(||self.task()).and_then(request_text)
    }
}

#[cfg(test)]
pub(super) fn input_event_fixture(input:&Value)->(nomifun_agent_contracts::PrincipalRef,nomifun_agent_contracts::SessionEventRecord) {
    let owner=nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:"routing-user".into()};
    let session=nomifun_agent_contracts::AgentSessionId::from("routing-session");
    let event=nomifun_agent_contracts::SessionEventRecord {agent_session_id:session.clone(),event_id:"routing-input".into(),seq:2,
        producer_id:"session_api".into(),idempotency_key:format!("{}{}",nomifun_agent_session::session_input_namespace(&owner,&session),"a".repeat(64)).into(),
        kind:nomifun_agent_contracts::SessionEventKind("message/user-accepted".into()),kind_version:1,
        correlation_id:"routing-input".into(),causation_event_id:None,
        payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(nomifun_agent_contracts::StrictJsonValue(input.clone()))};
    (owner,event)
}

#[cfg(test)]
pub(super) fn human_input_fixture(input:&Value)->TurnIntentInput<'_> {
    let (owner,event)=input_event_fixture(input);
    TurnIntentInput::from_accepted(input,&event,&owner,ExecutionSourceScope::NoDelegationSource).unwrap()
}

#[cfg(test)]
pub(super) fn delegated_input_fixture(input:&Value)->TurnIntentInput<'_> {
    let (owner,mut event)=input_event_fixture(input);
    event.idempotency_key="internal-execution-turn".into();
    TurnIntentInput::from_accepted(input,&event,&owner,ExecutionSourceScope::VerifiedDelegatedTask).unwrap()
}

pub(super) fn request_text(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        // Arbitrary JSON/code is source data, not an imperative media request.
        return None;
    }
    Some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_background_and_arbitrary_json_are_not_current_requests() {
        let envelope = serde_json::json!({
            "task_brief":"Use subagents to generate videos for the overall goal",
            "step_spec":"Implement the HTML game",
        }).to_string();
        assert!(request_text(&envelope).is_none());
        assert!(request_text(r#"{"example":"generate a video"}"#).is_none());
        assert!(request_text("{invalid JSON").is_none());
        assert_eq!(request_text("Generate a video"), Some("Generate a video"));
    }

    #[test]
    fn admitted_source_separates_human_requests_from_delegated_roles_and_envelopes() {
        let text="You are SubAgent A. Implement this module; the parent coordinates multiple agents.";
        for origin in ["agent_execution","agent_execution_decision","agent"] {
            let accepted=serde_json::json!({"origin":origin,"content":text});
            let input=delegated_input_fixture(&accepted);
            assert_eq!(input.task(),text);assert_eq!(input.source,TurnIntentSource::DelegatedTask);
            assert!(input.human_request().is_none());
        }
        let envelope=serde_json::json!({"task_brief":"Use multiple agents to build the product","step_spec":"Generate a video"}).to_string();
        let child=serde_json::json!({"origin":"agent_execution","content":envelope});
        let child=delegated_input_fixture(&child);
        assert_eq!(child.task(),"Generate a video");assert!(child.human_request().is_none());
        let document=serde_json::json!({"content":envelope});
        let document=human_input_fixture(&document);
        assert_eq!(document.task(),envelope);assert!(document.human_request().is_none());
        for origin in [serde_json::Value::Null,serde_json::json!("idmm"),serde_json::json!("robot")] {
            let accepted=serde_json::json!({"origin":origin,"content":"Use subagents; ask each child to delegate an independent check."});
            let input=human_input_fixture(&accepted);
            assert!(input.human_request().unwrap().contains("each child to delegate"));
        }
        for origin in ["cron","autowork","unknown-host-source"] {
            let accepted=serde_json::json!({"origin":origin,"content":"Use subagents to implement the change"});
            let input=human_input_fixture(&accepted);
            assert_eq!(input.source,TurnIntentSource::HostTask);assert!(input.human_request().is_none());
        }
        let continuation=serde_json::json!({"content":"Use subagents to finish the original task",
            "delivery":{"origin":null,"files":[],"inject_skills":[]}});
        let continuation=human_input_fixture(&continuation);
        assert_eq!(continuation.source,TurnIntentSource::HostTask);
        assert!(continuation.human_request().is_none(),"a host continuation is not a fresh human requirement even if its original text requests delegation");
    }

    #[test]
    fn origin_text_cannot_replace_the_original_canonical_human_intake_fact() {
        let payload=serde_json::json!({"origin":"robot","content":"Use multiple agents to verify this change"});
        let (owner,event)=input_event_fixture(&payload);
        let scope=ExecutionSourceScope::NoDelegationSource;
        assert!(TurnIntentInput::from_accepted(&payload,&event,&owner,scope).unwrap().human_request().is_some());
        let mut foreign=event.clone();foreign.producer_id="runtime_supervisor".into();
        assert!(TurnIntentInput::from_accepted(&payload,&foreign,&owner,scope).unwrap().human_request().is_none());
        let mut unknown=event.clone();unknown.idempotency_key="internal-task-source".into();
        assert!(TurnIntentInput::from_accepted(&payload,&unknown,&owner,scope).unwrap().human_request().is_none());
        let mut mismatched=event.clone();mismatched.correlation_id="another-message".into();
        assert!(TurnIntentInput::from_accepted(&payload,&mismatched,&owner,scope).unwrap().human_request().is_none());
        let other=nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:"another-owner".into()};
        assert!(TurnIntentInput::from_accepted(&payload,&event,&other,scope).unwrap().human_request().is_none());
    }

    #[test]
    fn task_envelopes_require_a_verified_execution_source_and_an_internal_input() {
        let envelope=serde_json::json!({"task_brief":"Use multiple agents for the product","step_spec":"Use subagents for a nested check"}).to_string();
        let accepted=serde_json::json!({"origin":"agent_execution","content":envelope});
        let (owner,mut event)=input_event_fixture(&accepted);
        for scope in [ExecutionSourceScope::NoDelegationSource,ExecutionSourceScope::VerifiedDelegatedTask] {
            let spoof=TurnIntentInput::from_accepted(&accepted,&event,&owner,scope).unwrap();
            assert_eq!(spoof.source,TurnIntentSource::HostTask);assert_eq!(spoof.task(),envelope);
            assert!(spoof.human_request().is_none(),"a public origin string never mints a new delegation requirement");
        }
        event.idempotency_key="internal-execution-turn".into();
        let unproven=TurnIntentInput::from_accepted(&accepted,&event,&owner,ExecutionSourceScope::NoDelegationSource).unwrap();
        assert_eq!(unproven.source,TurnIntentSource::HostTask);assert_eq!(unproven.task(),envelope);
        let proven=TurnIntentInput::from_accepted(&accepted,&event,&owner,ExecutionSourceScope::VerifiedDelegatedTask).unwrap();
        assert_eq!(proven.source,TurnIntentSource::DelegatedTask);assert_eq!(proven.task(),"Use subagents for a nested check");
        assert!(proven.human_request().is_none());
    }
}

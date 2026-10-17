//! One read-only result reducer shared by history and delivery consumers.
use std::collections::{BTreeMap, BTreeSet};
use nomifun_agent_contracts::{AgentTurnOutcome, AgentUserInputRequest, AgentWaitReason, OperationId, SessionEventRecord,
    canonical_assistant_step_message_id, digest_bytes};
use serde::Deserialize;
use serde_json::Value;
use crate::{ChatCausalityFacts, SessionStoreError, TurnReceipt, TurnReceiptStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalTurnPublicResult {
    pub turn_outcome: Option<AgentTurnOutcome>,
    pub result_text: Option<String>,
    pub user_input_request: Option<AgentUserInputRequest>,
}

fn invalid(detail: impl Into<String>) -> SessionStoreError {
    SessionStoreError::InvalidEvent(format!("canonical Turn public result: {}", detail.into()))
}

pub fn canonical_turn_public_result(facts: &ChatCausalityFacts, receipt: &TurnReceipt)
    -> Result<CanonicalTurnPublicResult, SessionStoreError> {
    if receipt.agent_session_id != facts.session.agent_session_id { return Err(invalid("Session differs from receipt")); }
    derive_turn_public_result(receipt, matches!(facts.head.status.as_str(), "paused" | "reconciliation")
        && facts.head.active_turn_id.as_deref() == Some(receipt.operation_id.as_ref()),
        &facts.events, &facts.event_payloads)
}

pub fn canonical_turn_outcome(facts: &ChatCausalityFacts, receipt: &TurnReceipt)
    -> Result<Option<AgentTurnOutcome>, SessionStoreError> {
    Ok(canonical_turn_public_result(facts, receipt)?.turn_outcome)
}

/// An exact active Turn's parked state at an observed canonical cursor. This
/// projection grants no process ownership, input or recovery authorization.
fn canonical_turn_waiting_outcome(events: &[SessionEventRecord], payloads: &BTreeMap<String, Value>,
    operation: &OperationId, through_seq: u64) -> Result<Option<AgentTurnOutcome>, SessionStoreError> {
    let mut pending_request = None;
    let mut waiting = None;
    for event in events.iter().filter(|event| event.seq <= through_seq && event.correlation_id.as_ref() == operation.as_ref()) {
        if matches!(event.kind.0.as_str(), "turn/completed" | "turn/failed" | "turn/cancelled") { waiting = None; continue; }
        if event.kind.0 != "runtime/progress-recorded" { continue; }
        let progress = payloads.get(event.event_id.as_ref()).and_then(|value| value.get("event"))
            .ok_or_else(|| invalid("waiting observation has no resolved Runtime payload"))?;
        match progress.get("event").and_then(Value::as_str) {
            Some("user_input_requested") => {
                let request: AgentUserInputRequest = serde_json::from_value(progress.get("request").cloned()
                    .ok_or_else(|| invalid("waiting question is missing"))?)?;
                request.validate().map_err(invalid)?;
                pending_request = Some(request.request_id.as_ref().to_owned());
            }
            Some("turn_waiting") => {
                let reason: AgentWaitReason = serde_json::from_value(progress.get("reason").cloned()
                    .ok_or_else(|| invalid("waiting reason is missing"))?)?;
                let request_id = progress.get("request_id").filter(|value| !value.is_null()).map(|value|
                    value.as_str().map(str::to_owned).ok_or_else(|| invalid("waiting request identity is invalid"))).transpose()?;
                if request_id.as_ref().is_some_and(|id| pending_request.as_ref() != Some(id))
                    || (reason == AgentWaitReason::UserInput && request_id.is_none()) {
                    return Err(invalid("waiting does not reference its exact source question"));
                }
                waiting = Some(AgentTurnOutcome::Waiting { wait_seq:event.seq, request_id, reason });
            }
            Some("steering_inputs") if progress.get("inputs").and_then(Value::as_array).is_some_and(|inputs| !inputs.is_empty()) => {
                waiting = None; pending_request = None;
            },
            Some("turn_wait_resolved" | "model_step_started" | "tool_started") => { waiting = None; pending_request = None; },
            _ => {},
        }
    }
    Ok(waiting)
}

#[derive(Deserialize)]
struct Part { content: String, turn_id: String }
#[derive(Deserialize)]
struct Seal { part_count: u64, content_digest: String }

fn owned_empty_seal(event: &SessionEventRecord, start: &SessionEventRecord, root: &str) -> bool {
    if event.causation_event_id.as_ref().is_some_and(|cause|cause==&start.event_id || cause.as_ref()==root) { return true; }
    let (Ok(root),Ok(id))=(uuid::Uuid::parse_str(root),uuid::Uuid::parse_str(event.correlation_id.as_ref())) else {return false};
    let (root_bytes,id_bytes)=(root.as_bytes(),id.as_bytes());
    root.get_version_num()==7 && root_bytes[..14]==id_bytes[..14]
        && root_bytes[14..]!=id_bytes[14..]
}

fn validate_public_message_id(id:&str)->Result<(),SessionStoreError> {
    if uuid::Uuid::parse_str(id).ok().is_none_or(|id|id.get_version_num()!=7) {
        return Err(invalid("public message identity is not a canonical UUIDv7"));
    }
    Ok(())
}

pub(crate) fn derive_turn_public_result(receipt: &TurnReceipt, paused: bool,
    events: &[SessionEventRecord], payloads: &BTreeMap<String, Value>)
    -> Result<CanonicalTurnPublicResult, SessionStoreError> {
    let result = |outcome| CanonicalTurnPublicResult { turn_outcome:outcome, result_text:None, user_input_request:None };
    if receipt.status == TurnReceiptStatus::NotFound { return Ok(result(None)); }
    let start = receipt.started_event.as_ref().ok_or_else(|| invalid("missing Turn start"))?;
    if start.agent_session_id != receipt.agent_session_id || start.kind.0 != "turn/started"
        || start.correlation_id.as_ref() != receipt.operation_id.as_ref() { return Err(invalid("invalid Turn start")); }
    let payload = |event: &SessionEventRecord| -> Result<&Value, SessionStoreError> {
        if event.agent_session_id != receipt.agent_session_id || event.kind_version != 1 { return Err(invalid("invalid event identity/version")); }
        payloads.get(event.event_id.as_ref()).ok_or_else(|| invalid("missing resolved payload"))
    };
    let root = payload(start)?.get("source_message_id").and_then(Value::as_str)
        .ok_or_else(|| invalid("start has no accepted source message"))?;
    if receipt.status == TurnReceiptStatus::Running && !paused {
        let outcome = canonical_turn_waiting_outcome(events, payloads, &receipt.operation_id, u64::MAX)?;
        let mut public = result(outcome);
        if let Some(AgentTurnOutcome::Waiting {request_id:Some(id),..}) = &public.turn_outcome {
            for event in events.iter().filter(|event| event.correlation_id.as_ref()==receipt.operation_id.as_ref() && event.kind.0=="runtime/progress-recorded") {
                let progress = payload(event)?.get("event").unwrap_or(&Value::Null);
                if progress.get("event").and_then(Value::as_str)==Some("user_input_requested")
                    && progress.pointer("/request/request_id").and_then(Value::as_str)==Some(id) {
                    public.user_input_request = Some(serde_json::from_value(progress["request"].clone())?);
                }
            }
        }
        return Ok(public);
    }
    let mut request=None;
    let mut terminal_model_step=None;
    let upper=if receipt.status==TurnReceiptStatus::Running {
        if receipt.terminal_event.is_some() {return Err(invalid("running Turn has a terminal"));}
        u64::MAX
    } else {
        let terminal=receipt.terminal_event.as_ref().ok_or_else(||invalid("missing terminal"))?;
        let expected=match receipt.status {TurnReceiptStatus::Completed=>"turn/completed",TurnReceiptStatus::Failed=>"turn/failed",
            TurnReceiptStatus::Cancelled=>"turn/cancelled",_=>unreachable!()};
        if terminal.seq<=start.seq || terminal.kind.0!=expected || terminal.correlation_id.as_ref()!=receipt.operation_id.as_ref() {
            return Err(invalid("invalid exact terminal"));
        }
        let terminal_payload=payload(terminal)?;
        terminal_model_step=terminal_payload.get("model_steps").and_then(Value::as_u64)
            .and_then(|step|u16::try_from(step).ok()).filter(|step|*step>0);
        if let Some(value)=terminal_payload.get("user_input_request") {
            let typed:AgentUserInputRequest=serde_json::from_value(value.clone())?;
            typed.validate().map_err(invalid)?;
            if receipt.status!=TurnReceiptStatus::Completed {return Err(invalid("input handoff requires normal Turn completion"));}
            request=Some(typed);
        }
        terminal.seq
    };
    let mut parts=BTreeMap::<String,(String,u64)>::new();
    let mut messages=BTreeMap::<String,String>::new();
    let mut sealed=BTreeSet::new();
    let mut latest_seal=None;
    let mut latest_part=None;
    let mut truncated=BTreeSet::<u16>::new();
    let mut nonpublic=BTreeSet::<u16>::new();
    let mut proven_model_steps=BTreeSet::<u16>::new();
    let mut delivered_steps=BTreeSet::<u16>::new();
    let mut previous_seq=start.seq;
    for event in events.iter().filter(|event|event.seq>start.seq && event.seq<upper) {
        if event.seq<=previous_seq {return Err(invalid("event order is not strictly increasing"));}
        previous_seq=event.seq;
        match event.kind.0.as_str() {
            "message/content-part"=>{
                let part:Part=serde_json::from_value(payload(event)?.clone())?;
                if part.turn_id!=root {continue;}
                let id=event.correlation_id.as_ref().to_owned();
                validate_public_message_id(&id)?;
                if sealed.contains(&id) {return Err(invalid("public content follows its seal"));}
                let current=parts.entry(id.clone()).or_default();
                if current.0.len().saturating_add(part.content.len())>8*1024*1024 {return Err(invalid("public content budget exceeded"));}
                current.0.push_str(&part.content);current.1+=1;
                messages.insert(id.clone(),current.0.clone());latest_part=Some(id);
            }
            "message/completed"=>{
                let seal:Seal=serde_json::from_value(payload(event)?.clone())?;
                let id=event.correlation_id.as_ref().to_owned();
                if sealed.contains(&id) {return Err(invalid("duplicate public content seal"));}
                let text=match parts.remove(&id) {
                    Some((text,count)) if count==seal.part_count && digest_bytes(text.as_bytes()).as_ref()==seal.content_digest=>text,
                    None if owned_empty_seal(event,start,root)=>{
                        validate_public_message_id(&id)?;
                        if seal.part_count!=0 || digest_bytes(b"").as_ref()!=seal.content_digest {return Err(invalid("public seal has no matching committed parts"));}
                        String::new()
                    },
                    None=>continue,
                    _=>return Err(invalid("public content seal differs from committed parts")),
                };
                if !sealed.insert(id.clone()) {return Err(invalid("duplicate public content seal"));}
                latest_seal=Some(id.clone());messages.insert(id,text);
            }
            "runtime/progress-recorded" if event.correlation_id.as_ref()==receipt.operation_id.as_ref()=>{
                let progress=payload(event)?.get("event").unwrap_or(&Value::Null);
                let Some(step)=progress.get("step").and_then(Value::as_u64).and_then(|step|u16::try_from(step).ok()) else {continue;};
                match progress.get("event").and_then(Value::as_str) {
                    Some("model_step_started"|"output_text_delta")=>{proven_model_steps.insert(step);}
                    Some("completion_delivered")=>{proven_model_steps.insert(step);delivered_steps.insert(step);}
                    Some("model_output_truncated") if progress.get("continuation").and_then(Value::as_bool)==Some(true)
                        && progress.get("discarded_tool_call_ids").and_then(Value::as_array).is_some_and(Vec::is_empty)=>{truncated.insert(step);}
                    Some("voice_model_step_superseded")=>{
                        let id=canonical_assistant_step_message_id(root,step).map_err(invalid)?;
                        // The typed withdrawal retains immutable audit parts,
                        // but they are no longer an unfinished public answer.
                        parts.remove(&id);messages.remove(&id);
                        if latest_part.as_ref()==Some(&id) {latest_part=None;}
                        if latest_seal.as_ref()==Some(&id) {latest_seal=None;}
                        nonpublic.insert(step);
                    }
                    Some("model_response_rejected"|"tool_call_completed")=>{nonpublic.insert(step);}
                    _=>{}
                }
            }
            _=>{}
        }
    }
    if receipt.status==TurnReceiptStatus::Completed && !parts.is_empty() {return Err(invalid("completed Turn has unfinished public content"));}
    if let Some(request)=request {
        let mut public=result(Some(AgentTurnOutcome::InputRequired {request_id:request.request_id.as_ref().into()}));
        public.user_input_request=Some(request);return Ok(public);
    }
    if receipt.status==TurnReceiptStatus::Completed {
        let Some(step)=terminal_model_step.filter(|step|proven_model_steps.contains(step)
            &&(!nonpublic.contains(step)||delivered_steps.contains(step))) else {
            return Ok(result(Some(AgentTurnOutcome::Unavailable)));
        };
        let expected=canonical_assistant_step_message_id(root,step).map_err(invalid)?;
        if latest_seal.as_ref()!=Some(&expected) {return Ok(result(Some(AgentTurnOutcome::Unavailable)));}
    }
    let final_id=if receipt.status==TurnReceiptStatus::Completed {latest_seal} else {latest_part};
    let mut ids=final_id.filter(|id|messages.get(id).is_some_and(|text|!text.trim().is_empty())).into_iter().collect::<Vec<_>>();
    let mut predecessor=BTreeMap::new();
    for step in truncated {
        let Some(next)=step.checked_add(1) else {continue;};
        if nonpublic.contains(&step)||nonpublic.contains(&next) {continue;}
        let (Ok(previous),Ok(current))=(canonical_assistant_step_message_id(root,step),canonical_assistant_step_message_id(root,next)) else {continue;};
        if sealed.contains(&previous) && messages.get(&previous).is_some_and(|text|!text.trim().is_empty())
            && messages.get(&current).is_some_and(|text|!text.trim().is_empty()) {predecessor.insert(current,previous);}
    }
    while let Some(previous)=ids.last().and_then(|id|predecessor.get(id)) {ids.push(previous.clone());}
    ids.reverse();
    let text=(!ids.is_empty()).then(||ids.iter().map(|id|messages[id].as_str()).collect::<String>());
    let outcome=match receipt.status {
        TurnReceiptStatus::Completed if ids.is_empty()=>AgentTurnOutcome::Unavailable,
        TurnReceiptStatus::Completed=>AgentTurnOutcome::Answered {message_ids:ids},
        TurnReceiptStatus::Failed=>AgentTurnOutcome::Failed {message_ids:ids},
        TurnReceiptStatus::Cancelled=>AgentTurnOutcome::Cancelled {message_ids:ids},
        TurnReceiptStatus::Running=>AgentTurnOutcome::Paused {message_ids:ids},
        TurnReceiptStatus::NotFound=>unreachable!(),
    };
    Ok(CanonicalTurnPublicResult {turn_outcome:Some(outcome),result_text:text,user_input_request:None})
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{SessionEventKind,SessionEventPayloadRef};
    use serde_json::json;
    const ROOT:&str="0190f5fe-7c00-7a00-8000-000000000001";

    #[test]
    fn waiting_keeps_the_exact_question_and_yields_to_input_or_terminal() {
        let mut f = Fixture::new();
        f.receipt.status = TurnReceiptStatus::Running;
        f.receipt.terminal_event = None;
        f.push("runtime/progress-recorded", "operation", json!({"event":{"event":"user_input_requested","step":1,
            "request":{"request_id":"authorize","question":"请完成浏览器授权","remaining_work":"验证登录并继续"}}}));
        f.push("runtime/progress-recorded", "operation", json!({"event":{"event":"turn_waiting","model_steps":1,
            "request_id":"authorize","reason":"user_input"}}));
        let before = f.events.last().unwrap().seq;
        let waiting = derive_turn_public_result(&f.receipt, false, &f.events, &f.payloads).unwrap();
        assert_eq!(waiting.turn_outcome, Some(AgentTurnOutcome::Waiting {wait_seq:before,request_id:Some("authorize".into()),reason:AgentWaitReason::UserInput}));
        assert_eq!(waiting.user_input_request.unwrap().question, "请完成浏览器授权");
        f.push("runtime/progress-recorded", "foreign", json!({"event":{"event":"turn_wait_resolved","model_steps":1}}));
        assert!(canonical_turn_waiting_outcome(&f.events,&f.payloads,&"operation".into(),u64::MAX).unwrap().is_some());
        f.push("runtime/progress-recorded", "operation", json!({"event":{"event":"steering_inputs","inputs":[{"receipt_operation_id":"owner-answer"}]}}));
        assert!(derive_turn_public_result(&f.receipt,false,&f.events,&f.payloads).unwrap().turn_outcome.is_none());
        assert!(canonical_turn_waiting_outcome(&f.events,&f.payloads,&"operation".into(),before).unwrap().is_some());
    }

    #[test]
    fn waiting_without_a_source_question_is_rejected_and_native_pause_wins() {
        let mut f = Fixture::new();f.receipt.status=TurnReceiptStatus::Running;f.receipt.terminal_event=None;
        f.push("runtime/progress-recorded","operation",json!({"event":{"event":"turn_waiting","model_steps":1,
            "request_id":"invented","reason":"user_input"}}));
        assert!(derive_turn_public_result(&f.receipt,false,&f.events,&f.payloads).is_err());
        assert!(matches!(derive_turn_public_result(&f.receipt,true,&f.events,&f.payloads).unwrap().turn_outcome,
            Some(AgentTurnOutcome::Paused {..})));
    }
    struct Fixture {receipt:TurnReceipt,events:Vec<SessionEventRecord>,payloads:BTreeMap<String,Value>,next:u64}
    impl Fixture {
        fn new()->Self {
            let start=SessionEventRecord {agent_session_id:"session".into(),seq:1,event_id:"start".into(),
                producer_id:"runtime_supervisor".into(),idempotency_key:"start".into(),kind:SessionEventKind("turn/started".into()),
                kind_version:1,correlation_id:"operation".into(),causation_event_id:None,payload:SessionEventPayloadRef::Empty};
            let terminal=SessionEventRecord {seq:100,event_id:"terminal".into(),kind:SessionEventKind("turn/completed".into()),..start.clone()};
            Self {receipt:TurnReceipt {agent_session_id:"session".into(),operation_id:"operation".into(),status:TurnReceiptStatus::Completed,
                    started_event:Some(start.clone()),terminal_event:Some(terminal)},events:vec![start],
                payloads:BTreeMap::from([("start".into(),json!({"source_message_id":ROOT})),("terminal".into(),json!({}))]),next:2}
        }
        fn push(&mut self,kind:&str,id:&str,value:Value) {
            let event=SessionEventRecord {seq:self.next,event_id:format!("event-{}",self.next).into(),
                kind:SessionEventKind(kind.into()),correlation_id:id.into(),causation_event_id:Some("start".into()),
                ..self.receipt.started_event.as_ref().unwrap().clone()};
            self.next+=1;self.payloads.insert(event.event_id.as_ref().into(),value);self.events.push(event);
        }
        fn answer(&mut self,step:u16,text:&str) ->String {
            let id=canonical_assistant_step_message_id(ROOT,step).unwrap();
            self.payloads.get_mut("terminal").unwrap()["model_steps"]=json!(step);
            self.push("runtime/progress-recorded","operation",json!({"event":{"event":"output_text_delta","step":step,"text":text}}));
            self.push("message/content-part",&id,json!({"turn_id":ROOT,"content":text}));
            self.push("message/completed",&id,json!({"part_count":1,"content_digest":digest_bytes(text.as_bytes())}));id
        }
        fn derive(&self,paused:bool)->Result<CanonicalTurnPublicResult,SessionStoreError> {
            derive_turn_public_result(&self.receipt,paused,&self.events,&self.payloads)
        }
    }
    #[test]
    fn typed_handoff_survives_the_empty_assistant_seal_without_copying_the_question() {
        let mut f=Fixture::new();
        f.push("message/completed",&canonical_assistant_step_message_id(ROOT,1).unwrap(),json!({"part_count":0,"content_digest":digest_bytes(b"")}));
        f.payloads.get_mut("terminal").unwrap()["user_input_request"]=json!({"request_id":"ask-name","question":"姓名？","remaining_work":"完成PPT"});
        let result=f.derive(false).unwrap();
        assert_eq!(result.turn_outcome,Some(AgentTurnOutcome::InputRequired {request_id:"ask-name".into()}));
        assert_eq!(result.result_text,None);
        f.receipt.status=TurnReceiptStatus::Failed;f.receipt.terminal_event.as_mut().unwrap().kind=SessionEventKind("turn/failed".into());
        assert!(f.derive(false).is_err(),"failure cannot be reinterpreted as a normal handoff");
    }
    #[test]
    fn last_empty_seal_is_unavailable_instead_of_reusing_an_earlier_progress_message() {
        let mut f=Fixture::new();f.answer(1,"I will start working.");
        f.push("message/completed",&canonical_assistant_step_message_id(ROOT,2).unwrap(),json!({"part_count":0,"content_digest":digest_bytes(b"")}));
        assert_eq!(f.derive(false).unwrap().turn_outcome,Some(AgentTurnOutcome::Unavailable));
        assert_eq!(f.derive(false).unwrap().result_text,None);
    }

    #[test]
    fn the_closing_model_step_must_prove_the_answer_in_old_history_without_a_fabricated_empty_seal() {
        let mut f=Fixture::new();f.answer(1,"I will start working.");
        f.push("runtime/progress-recorded","operation",json!({"event":{"event":"model_step_started","step":2,"operation_id":"operation:model:2"}}));
        f.payloads.get_mut("terminal").unwrap()["model_steps"]=json!(2);
        assert_eq!(f.derive(false).unwrap().turn_outcome,Some(AgentTurnOutcome::Unavailable));
        f.payloads.get_mut("terminal").unwrap().as_object_mut().unwrap().remove("model_steps");
        assert_eq!(f.derive(false).unwrap().turn_outcome,Some(AgentTurnOutcome::Unavailable),"unproven closed history cannot borrow an earlier preface");
    }
    #[test]
    fn final_answer_and_explicit_continuations_are_exact_and_exclude_future_or_foreign_messages() {
        let mut f=Fixture::new();f.answer(1,"Progress.");let prefix=f.answer(2,"Hello ");
        f.push("runtime/progress-recorded","operation",json!({"event":{"event":"model_output_truncated","step":2,"continuation":true,"discarded_tool_call_ids":[]}}));
        let tail=f.answer(3,"world.");
        f.push("message/content-part","foreign",json!({"turn_id":"another-root","content":"foreign"}));
        let foreign_part=f.events.last().unwrap().event_id.clone();
        f.push("message/completed","foreign",json!({"part_count":1,"content_digest":digest_bytes(b"foreign")}));
        f.events.last_mut().unwrap().causation_event_id=Some(foreign_part);
        f.next=101;f.answer(4,"Future.");
        f.payloads.get_mut("terminal").unwrap()["model_steps"]=json!(3);
        let result=f.derive(false).unwrap();
        assert_eq!(result.turn_outcome,Some(AgentTurnOutcome::Answered {message_ids:vec![prefix,tail]}));
        assert_eq!(result.result_text.as_deref(),Some("Hello world."));
    }
    #[test]
    fn digest_damage_unfinished_content_and_duplicate_seals_fail_closed() {
        let mut f=Fixture::new();let id=f.answer(1,"answer");
        let seal=f.events.last().unwrap().event_id.as_ref().to_owned();
        f.payloads.get_mut(&seal).unwrap()["content_digest"]=json!("bad");assert!(f.derive(false).is_err());
        f.payloads.get_mut(&seal).unwrap()["content_digest"]=json!(digest_bytes(b"answer"));
        f.push("message/completed",&id,json!({"part_count":0,"content_digest":digest_bytes(b"")}));assert!(f.derive(false).is_err());
        let mut f=Fixture::new();f.push("message/content-part",&canonical_assistant_step_message_id(ROOT,1).unwrap(),json!({"turn_id":ROOT,"content":"not sealed"}));
        assert!(f.derive(false).is_err());
    }
    #[test]
    fn paused_failed_and_cancelled_reference_only_committed_partial_public_content() {
        let mut f=Fixture::new();f.answer(1,"Earlier progress.");
        let partial=canonical_assistant_step_message_id(ROOT,2).unwrap();
        f.push("message/content-part",&partial,json!({"turn_id":ROOT,"content":"Partial answer."}));
        f.receipt.status=TurnReceiptStatus::Running;f.receipt.terminal_event=None;
        assert_eq!(f.derive(false).unwrap().turn_outcome,None);
        assert_eq!(f.derive(true).unwrap().turn_outcome,Some(AgentTurnOutcome::Paused {message_ids:vec![partial.clone()]}));
        let terminal=SessionEventRecord {seq:100,event_id:"terminal".into(),kind:SessionEventKind("turn/failed".into()),..f.receipt.started_event.as_ref().unwrap().clone()};
        f.receipt.terminal_event=Some(terminal);f.receipt.status=TurnReceiptStatus::Failed;
        assert_eq!(f.derive(false).unwrap().turn_outcome,Some(AgentTurnOutcome::Failed {message_ids:vec![partial.clone()]}));
        f.receipt.status=TurnReceiptStatus::Cancelled;f.receipt.terminal_event.as_mut().unwrap().kind=SessionEventKind("turn/cancelled".into());
        assert_eq!(f.derive(false).unwrap().turn_outcome,Some(AgentTurnOutcome::Cancelled {message_ids:vec![partial]}));
    }

    #[test]
    fn voice_superseded_public_parts_remain_audit_without_blocking_the_successor_or_becoming_cancelled_output() {
        let mut f=Fixture::new();let withdrawn=canonical_assistant_step_message_id(ROOT,1).unwrap();
        f.push("message/content-part",&withdrawn,json!({"turn_id":ROOT,"content":"Withdrawn draft."}));
        f.push("runtime/progress-recorded","operation",json!({"event":{"event":"voice_model_step_superseded","step":1}}));
        let audit=f.events.clone();
        f.receipt.status=TurnReceiptStatus::Cancelled;f.receipt.terminal_event.as_mut().unwrap().kind=SessionEventKind("turn/cancelled".into());
        let cancelled=f.derive(false).unwrap();
        assert_eq!(cancelled.turn_outcome,Some(AgentTurnOutcome::Cancelled {message_ids:vec![]}));
        assert!(cancelled.result_text.is_none());
        assert_eq!(f.events,audit,"read-only outcome derivation retains withdrawn audit facts");
        f.receipt.status=TurnReceiptStatus::Completed;f.receipt.terminal_event.as_mut().unwrap().kind=SessionEventKind("turn/completed".into());
        let successor=f.answer(2,"Current answer.");
        assert_eq!(f.derive(false).unwrap().turn_outcome,Some(AgentTurnOutcome::Answered {message_ids:vec![successor]}));
        assert_eq!(f.derive(false).unwrap().result_text.as_deref(),Some("Current answer."));
    }
}

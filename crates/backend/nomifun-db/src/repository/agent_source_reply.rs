//! Explicit question replies can continue only their original canonical work.
//! The same transaction-scoped proof serves readers and command admission.
use crate::DbError;
use nomifun_agent_contracts::{AgentBindingValue,AgentHandoffBindingRefV1,AgentUserInputRequest,ResolvedSnapshotRef,SessionPayloadBody};
use serde_json::Value;
use sqlx::{Sqlite,Transaction};

fn invalid(detail:impl std::fmt::Display)->DbError {DbError::Init(format!("canonical collaboration reply: {detail}"))}

/// Immutable original native command proof. Accepted work can read this after
/// its parent ends; admission separately checks the sender's current epoch.
pub async fn agent_message_command_tx(tx:&mut Transaction<'_,Sqlite>,source:&crate::AgentExecutionMessageSource)
    ->Result<(u64,nomifun_agent_contracts::StrictJsonValue),DbError> {
    use nomifun_agent_contracts::{SharedToolContentKind,SharedToolContentReference,SHARED_TOOL_CONTENT_KEY};
    let row:Option<(Option<String>,Option<String>,i64)>=sqlx::query_as("SELECT turn.started_event_id,turn.terminal_event_id,head.last_seq FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id WHERE turn.session_id=? AND turn.operation_id=?")
        .bind(&source.agent_session_id).bind(&source.turn_operation_id).fetch_optional(&mut **tx).await?;
    let (start,terminal,head)=row.ok_or_else(||invalid("message Turn is missing"))?;
    let started=event(tx,&source.agent_session_id,Some(&start.ok_or_else(||invalid("message Turn start is missing"))?),None).await?;
    let command=event(tx,&source.agent_session_id,Some(&source.command_event_id),None).await?;
    if started.kind!="turn/started"||started.kind_version!=1||started.producer_id!="session_api"
        ||started.correlation_id!=source.turn_operation_id||command.seq<=started.seq||command.seq>head
        ||command.kind!="runtime/progress-recorded"||command.kind_version!=1||command.producer_id!="runtime_supervisor"
        ||command.correlation_id!=source.turn_operation_id {
        return Err(invalid("message is not an exact native command in its Turn"));
    }
    if let Some(terminal)=terminal {
        let terminal=event(tx,&source.agent_session_id,Some(&terminal),None).await?;
        if terminal.correlation_id!=source.turn_operation_id||terminal.kind_version!=1||command.seq>=terminal.seq
            ||!matches!(terminal.kind.as_str(),"turn/completed"|"turn/failed"|"turn/cancelled") {
            return Err(invalid("message command is outside its native Turn"));
        }
    }
    let mut value=command.payload()?;
    if value.pointer("/event/event").and_then(Value::as_str)!=Some("tool_call_completed")
        ||value.pointer("/event/call/name").and_then(Value::as_str)!=Some("agent_message")
        ||value.pointer("/event/call/call_id").and_then(Value::as_str).is_none_or(str::is_empty) {
        return Err(invalid("message command is not an original agent_message proposal"));
    }
    if let Some(metadata)=value.get(SHARED_TOOL_CONTENT_KEY).cloned() {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct References {version:u8,references:Vec<SharedToolContentReference>,#[serde(default)] input_sources:Vec<Value>}
        let metadata:References=serde_json::from_value(metadata).map_err(invalid)?;
        let [reference]=metadata.references.as_slice() else {return Err(invalid("message command requires one exact shared argument slot"));};
        if metadata.version!=1||!metadata.input_sources.is_empty()
            ||!reference.is_for_slot(&source.agent_session_id.clone().into(),"/event/call/arguments",SharedToolContentKind::ToolArguments)
            ||!value.pointer("/event/call/arguments").is_some_and(Value::is_null) {
            return Err(invalid("shared message arguments differ from their Session or typed slot"));
        }
        let row:Option<(String,Vec<u8>,i64,String)>=sqlx::query_as("SELECT media_type,body,byte_len,digest FROM agent_payloads WHERE session_id=? AND payload_id=?")
            .bind(&source.agent_session_id).bind(reference.payload_id.as_ref()).fetch_optional(&mut **tx).await?;
        let (media,body,len,digest)=row.ok_or_else(||invalid("shared message arguments are missing"))?;
        let SessionPayloadBody::Json(body)=serde_json::from_slice(&body).map_err(invalid)? else {return Err(invalid("shared message arguments are not typed JSON"));};
        let bytes=nomifun_agent_contracts::canonical_json_bytes(&body.0).map_err(invalid)?;
        if media!=reference.kind.media_type()||u64::try_from(len).ok()!=Some(reference.byte_len)
            ||digest!=reference.digest.as_ref()||bytes.len() as u64!=reference.byte_len
            ||nomifun_agent_contracts::digest_bytes(&bytes)!=reference.digest||!reference.kind.accepts(&body.0) {
            return Err(invalid("shared message arguments differ from their exact bytes"));
        }
        *value.pointer_mut("/event/call/arguments").ok_or_else(||invalid("shared message slot is missing"))?=body.0;
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Proposal {
        #[serde(rename="event")] _event:String,
        #[serde(rename="step")] _step:u16,
        call:nomifun_agent_contracts::chat_model::ChatToolCall,
    }
    let proposal:Proposal=serde_json::from_value(value.get("event").cloned().ok_or_else(||invalid("message proposal is missing"))?).map_err(invalid)?;
    if !proposal.call.arguments.0.is_object() {return Err(invalid("message command arguments are not an object"));}
    Ok((u64::try_from(command.seq).map_err(invalid)?,proposal.call.arguments))
}

pub(crate) async fn verify_message_command_tx(tx:&mut Transaction<'_,Sqlite>,owner:&str,
    source:&crate::AgentExecutionMessageSource,execution:&str,step:&str,operation:&str,content_digest:&str)->Result<(),DbError> {
    verify_message_source_tx(tx,owner,source).await?;
    agent_message_command_bound_tx(tx,source,execution,step,operation,content_digest).await?;
    Ok(())
}

/// The original accepted command identity, independent of its sender's current
/// lease. Delivery readers and admission writers share the same argument and
/// stable-key binding; only a new command writer validates the current epoch.
pub async fn agent_message_command_bound_tx(tx:&mut Transaction<'_,Sqlite>,source:&crate::AgentExecutionMessageSource,
    execution:&str,step:&str,operation:&str,content_digest:&str)
    ->Result<(u64,nomifun_agent_contracts::StrictJsonValue),DbError> {
    let (sequence,arguments)=agent_message_command_tx(tx,source).await?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Command {execution_id:String,step_id:String,content:String,idempotency_key:String}
    let command:Command=serde_json::from_value(arguments.0.clone()).map_err(invalid)?;
    let identity=serde_json::to_vec(&(&source.agent_session_id,&source.turn_operation_id,execution,step,&command.idempotency_key)).map_err(invalid)?;
    let expected=format!("agent-message:{}",nomifun_agent_contracts::digest_bytes(&identity).as_ref());
    if command.execution_id!=execution||command.step_id!=step||command.idempotency_key.is_empty()||command.idempotency_key.len()>128
        ||command.content.trim().is_empty()||command.content.len()>64*1024||expected!=operation
        ||nomifun_agent_contracts::digest_bytes(command.content.as_bytes()).as_ref()!=content_digest {
        return Err(DbError::Conflict("Agent message command differs from its exact native proposal".into()));
    }
    Ok((sequence,arguments))
}

pub(crate) async fn verify_message_source_tx(tx:&mut Transaction<'_,Sqlite>,owner:&str,source:&crate::AgentExecutionMessageSource)
    ->Result<(),DbError> {
    let fence=i64::try_from(source.native_fence).map_err(invalid)?;
    let generation=i64::try_from(source.native_generation).map_err(invalid)?;
    let admitted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_turns turn JOIN agent_session_heads head ON head.session_id=turn.session_id JOIN agent_sessions session ON session.agent_session_id=turn.session_id WHERE turn.session_id=? AND turn.operation_id=? AND turn.state='running' AND head.active_turn_id=turn.operation_id AND head.status='running' AND turn.execution_owner IS NOT NULL AND turn.execution_owner<>'' AND turn.execution_fence=? AND turn.execution_generation=? AND turn.execution_generation>0 AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=?)")
        .bind(&source.agent_session_id).bind(&source.turn_operation_id).bind(fence).bind(generation).bind(owner).fetch_one(&mut **tx).await?;
    if !admitted||!crate::native_resume_target_eligible_tx(tx,owner,&source.agent_session_id,&source.turn_operation_id).await? {
        return Err(DbError::Conflict("Agent message source native epoch or domain owner is fenced".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Event {
    session_id:String,seq:i64,event_id:String,kind:String,kind_version:i64,producer_id:String,
    correlation_id:String,causation_event_id:Option<String>,inline_json:Option<String>,payload_id:Option<String>,
    payload_session:Option<String>,body:Option<Vec<u8>>,byte_len:Option<i64>,digest:Option<String>,
}

impl Event {
    fn payload(&self)->Result<Value,DbError> {
        match (&self.inline_json,&self.payload_id) {
            (Some(raw),None)=>serde_json::from_str(raw).map_err(invalid),
            (None,Some(_))=>{
                if self.payload_session.as_deref()!=Some(self.session_id.as_str()) {return Err(invalid("stored payload owner differs"));}
                let raw=self.body.as_ref().ok_or_else(||invalid("stored payload is missing"))?;
                let SessionPayloadBody::Json(value)=serde_json::from_slice::<SessionPayloadBody>(raw).map_err(invalid)?
                    else {return Err(invalid("reply proof requires typed JSON"));};
                let bytes=nomifun_agent_contracts::canonical_json_bytes(&value.0).map_err(invalid)?;
                if self.byte_len!=i64::try_from(bytes.len()).ok()||self.digest.as_deref()!=Some(nomifun_agent_contracts::digest_bytes(&bytes).as_ref()) {
                    return Err(invalid("stored payload length or digest differs"));
                }
                Ok(value.0)
            },
            _=>Err(invalid("reply proof has no unique payload")),
        }
    }
}

async fn event(tx:&mut Transaction<'_,Sqlite>,session:&str,id:Option<&str>,seq:Option<i64>)->Result<Event,DbError> {
    sqlx::query_as("SELECT e.session_id,e.seq,e.event_id,e.kind,e.kind_version,e.producer_id,e.correlation_id,e.causation_event_id,e.inline_json,e.payload_id,p.session_id AS payload_session,p.body,p.byte_len,p.digest FROM agent_events e LEFT JOIN agent_payloads p ON p.session_id=e.session_id AND p.payload_id=e.payload_id WHERE e.session_id=? AND ((? IS NOT NULL AND e.event_id=?) OR (? IS NOT NULL AND e.seq=?))")
        .bind(session).bind(id).bind(id).bind(seq).bind(seq).fetch_optional(&mut **tx).await?
        .ok_or_else(||invalid("referenced canonical event is missing"))
}

async fn input(tx:&mut Transaction<'_,Sqlite>,session:&str,operation:&str)->Result<(Event,Value,Event,Value),DbError> {
    let row:Option<(String,Option<String>,Option<String>)>=sqlx::query_as("SELECT turn_id,source_message_id,started_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(session).bind(operation).fetch_optional(&mut **tx).await?;
    let (turn,source,started)=row.ok_or_else(||invalid("reply Turn is missing"))?;
    let source=source.ok_or_else(||invalid("Turn source is missing"))?;
    let started=event(tx,session,Some(&started.ok_or_else(||invalid("Turn start is missing"))?),None).await?;
    let start=started.payload()?;
    if turn!=operation||started.kind!="turn/started"||started.kind_version!=1||started.producer_id!="session_api"
        ||started.correlation_id!=operation||start.get("operation_id").and_then(Value::as_str)!=Some(operation)
        ||start.get("source_message_id").and_then(Value::as_str)!=Some(source.as_str())
        ||started.causation_event_id.as_deref()!=Some(source.as_str()) {
        return Err(invalid("Turn start differs from its exact accepted input"));
    }
    let accepted=event(tx,session,Some(&source),None).await?;
    if accepted.kind!="message/user-accepted"||accepted.kind_version!=1||accepted.producer_id!="session_api"
        ||accepted.correlation_id!=accepted.event_id||accepted.seq>=started.seq {
        return Err(invalid("accepted reply identity differs"));
    }
    let body=accepted.payload()?;
    Ok((accepted,body,started,start))
}

pub(crate) fn binding(started:&Value)->Result<Option<AgentHandoffBindingRefV1>,DbError> {
    let admission=started.get("admission").and_then(Value::as_object).ok_or_else(||invalid("claim admission is missing"))?;
    let snapshot:ResolvedSnapshotRef=serde_json::from_value(admission.get("resolved_snapshot_ref").cloned()
        .ok_or_else(||invalid("admitted Snapshot is missing"))?).map_err(invalid)?;
    let Some(value)=admission.get("binding_ref") else {return Ok(None);};
    let binding:AgentHandoffBindingRefV1=serde_json::from_value(value.clone()).map_err(invalid)?;
    if binding.binding_version==0||binding.resolved_snapshot_ref!=snapshot {return Err(invalid("claim binding differs from its Snapshot"));}
    Ok(Some(binding))
}

pub(crate) async fn event_payload_tx(tx:&mut Transaction<'_,Sqlite>,session:&str,event_id:&str)->Result<Value,DbError> {
    event(tx,session,Some(event_id),None).await?.payload()
}

/// Read decreasing explicit reply ancestry. This never selects latest work,
/// loads a Runtime, changes a binding or authorizes replay of an old Turn.
pub async fn agent_reply_source_turns_tx(tx:&mut Transaction<'_,Sqlite>,owner:&str,session:&str,current:&str)
    ->Result<Vec<String>,DbError> {
    let raw:Option<String>=sqlx::query_scalar("SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id=? AND state='live' AND json_extract(owner_ref_json,'$.principal_kind')='user' AND json_extract(owner_ref_json,'$.principal_id')=?")
        .bind(session).bind(owner).fetch_optional(&mut **tx).await?;
    let live:AgentBindingValue=serde_json::from_str(&raw.ok_or_else(||invalid("live reply Session owner differs"))?).map_err(invalid)?;
    let live=AgentHandoffBindingRefV1::from(&live);
    let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session).fetch_one(&mut **tx).await?;
    let mut cursor=current.to_owned();let mut result=Vec::new();let mut bytes=0usize;
    loop {
        let (accepted,body,_,start)=input(tx,session,&cursor).await?;
        let Some(reply)=body.get("reply_to") else {break;};
        let object=reply.as_object().ok_or_else(||invalid("reply target is not an object"))?;
        if object.len()!=2 {return Err(invalid("reply target contains unsupported fields"));}
        let source=reply.get("turn_id").and_then(Value::as_str).ok_or_else(||invalid("reply source input is missing"))?;
        if uuid::Uuid::parse_str(source).map_err(invalid)?.get_version_num()!=7 {return Err(invalid("reply source input is not UUIDv7"));}
        let sequence=reply.get("question_seq").and_then(Value::as_u64).and_then(|value|i64::try_from(value).ok())
            .ok_or_else(||invalid("reply question sequence is invalid"))?;
        if sequence>=accepted.seq {return Err(invalid("question does not precede its reply"));}
        if sequence<=floor {break;}
        let rows:Vec<(String,String,Option<String>)>=sqlx::query_as("SELECT operation_id,state,terminal_event_id FROM agent_turns WHERE session_id=? AND source_message_id=? LIMIT 2")
            .bind(session).bind(source).fetch_all(&mut **tx).await?;
        let [(previous,state,terminal)]=rows.as_slice() else {return Err(invalid("question has no unique original Turn"));};
        if previous==&cursor {return Err(invalid("question reply cannot continue itself"));}
        if state!="completed" {break;}
        let (previous_input,_,previous_start,previous_started)=input(tx,session,previous).await?;
        if previous_input.seq>=accepted.seq {return Err(invalid("reply ancestry is not decreasing"));}
        if binding(&previous_started)?.as_ref()!=Some(&live)||binding(&start)?.as_ref()!=Some(&live) {break;}
        let question=event(tx,session,None,Some(sequence)).await?;
        let value=question.payload()?;
        if question.kind!="runtime/progress-recorded"||question.kind_version!=1||question.producer_id!="runtime_supervisor"
            ||question.correlation_id!=*previous||value.pointer("/event/event").and_then(Value::as_str)!=Some("user_input_requested") {
            return Err(invalid("reply does not reference its exact committed question"));
        }
        let request:AgentUserInputRequest=serde_json::from_value(value.pointer("/event/request").cloned()
            .ok_or_else(||invalid("question request is missing"))?).map_err(invalid)?;
        request.validate().map_err(invalid)?;
        let terminal=event(tx,session,Some(terminal.as_deref().ok_or_else(||invalid("question terminal is missing"))?),None).await?;
        if terminal.kind!="turn/completed"||terminal.kind_version!=1||terminal.correlation_id!=*previous
            ||previous_start.seq>=question.seq||question.seq>=terminal.seq {return Err(invalid("question terminal identity differs"));}
        let terminal=terminal.payload()?;
        let Some(value)=terminal.get("user_input_request") else {break;};
        let terminal:AgentUserInputRequest=serde_json::from_value(value.clone()).map_err(invalid)?;
        terminal.validate().map_err(invalid)?;
        if terminal!=request {break;}
        bytes=bytes.saturating_add(previous.len());if bytes>96*1024 {break;}
        result.push(previous.clone());cursor=previous.clone();
    }
    Ok(result)
}

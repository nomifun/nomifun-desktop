//! Runtime input delivery references its original accepted fact. This codec
//! stores no second body and never imports an unclaimed input into context.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InputReference {
    index: usize,
    claim_event_id: EventId,
    input_event_id: EventId,
    fields: Vec<String>,
}

async fn original_input(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,claim_id:&EventId,
    operation:&CorrelationId,before_seq:Option<u64>)->Result<(EventId,Value),SessionStoreError> {
    let claim=event_by_event_id_tx(tx,claim_id.as_ref()).await?.map(event_from_row).transpose()?
        .ok_or_else(||invalid("input delivery claim is missing"))?;
    if claim.agent_session_id!=*session || claim.kind.0!="turn/steer-accepted" || claim.kind_version!=2
        || claim.producer_id.as_ref()!="session_api" || claim.correlation_id!=*operation
        || before_seq.is_some_and(|seq|claim.seq>=seq) {
        return Err(invalid("input delivery claim differs from its exact Session and Turn"));
    }
    let payload=Box::pin(payload_value_for_event_tx(tx,&claim)).await?;
    if payload.get("target_operation_id").and_then(Value::as_str)!=Some(operation.as_ref()) || payload.get("input").is_some() {
        return Err(invalid("input delivery claim has an invalid target or body mirror"));
    }
    let id=payload.get("input_event_id").and_then(Value::as_str).ok_or_else(||invalid("input delivery source is missing"))?;
    let source=event_by_event_id_tx(tx,id).await?.map(event_from_row).transpose()?
        .ok_or_else(||invalid("input delivery source is missing"))?;
    if source.agent_session_id!=*session || source.kind.0!="message/user-accepted" || source.kind_version!=1
        || source.seq>=claim.seq || source.producer_id.as_ref()!="session_api" || source.correlation_id.as_ref()!=id
        || claim.causation_event_id.as_ref()!=Some(&source.event_id) {
        return Err(invalid("input delivery source has invalid accepted provenance"));
    }
    let session_row=session_row_by_id_tx(tx,session.as_ref()).await?;
    let owner:PrincipalRef=serde_json::from_str(&session_row.owner_ref_json)?;
    if !super::super::session_input::is_session_input_event(&source,&owner) {return Err(invalid("input source is not an accepted unified Session message"));}
    let input=Box::pin(payload_value_for_event_tx(tx,&source)).await?;
    Ok((source.event_id,input))
}

fn field(input:&Value,name:&str)->Result<Value,SessionStoreError> {
    match name {
        "text"=>input.get("content").filter(|v|v.is_string()).cloned().ok_or_else(||invalid("accepted input has no text")),
        "files"|"inject_skills"=> {
            let value=input.get(name).cloned().unwrap_or_else(||json!([]));
            let _:Vec<String>=serde_json::from_value(value.clone()).map_err(|_|invalid("accepted input references are invalid"))?;
            Ok(value)
        },
        _=>Err(invalid("input delivery reference selects an unsupported field")),
    }
}

pub(super) async fn prepare(tx:&mut Transaction<'_,Sqlite>,append:&SessionEventAppend,value:&mut Value)
    ->Result<Vec<InputReference>,SessionStoreError> {
    if append.semantic_event.kind.0!="runtime/progress-recorded" || !matches!(
        value.pointer("/event/event").and_then(Value::as_str),Some("steering_inputs"|"steering_deferred")) {return Ok(Vec::new());}
    let inputs=value.pointer_mut("/event/inputs").and_then(Value::as_array_mut)
        .ok_or_else(||invalid("input delivery has no typed inputs"))?;
    if inputs.len()>16 {return Err(invalid("input delivery exceeds its existing input bound"));}
    let mut references=Vec::new();
    for (index,delivered) in inputs.iter_mut().enumerate() {
        let receipt=delivered.get("receipt_operation_id").and_then(Value::as_str)
            .ok_or_else(||invalid("input delivery has no receipt identity"))?;
        let claim=event_by_event_id_tx(tx,receipt).await?;
        // Exact voice/domain v1 receipts retain their own original content.
        // Only the new unified input contract is represented by a source ref.
        if claim.as_ref().is_none_or(|row|row.kind!="turn/steer-accepted" || row.kind_version!=2) {continue;}
        let claim_id=EventId::from(receipt);
        let (input_id,original)=original_input(tx,&append.agent_session_id,&claim_id,&append.semantic_event.correlation_id,None).await?;
        if delivered.get("message_id").and_then(Value::as_str)!=Some(input_id.as_ref()) {
            return Err(invalid("input delivery message differs from its accepted source"));
        }
        let object=delivered.as_object_mut().ok_or_else(||invalid("input delivery is not an object"))?;
        let mut fields=Vec::new();
        for name in ["text","files","inject_skills"] {
            if let Some(value)=object.get(name) {
                if value!=&field(&original,name)? {return Err(invalid("input delivery content differs from its original accepted fact"));}
                fields.push(name.to_owned());
            }
        }
        if !fields.iter().any(|name|name=="text") {return Err(invalid("input delivery has no original text"));}
        for name in &fields {object.remove(name);}
        references.push(InputReference {index,claim_event_id:claim_id,input_event_id:input_id,fields});
    }
    Ok(references)
}

pub(super) async fn hydrate(tx:&mut Transaction<'_,Sqlite>,event:&SessionEventRecord,value:&mut Value,references:Vec<InputReference>)
    ->Result<(),SessionStoreError> {
    if references.is_empty() {return Ok(());}
    if event.kind.0!="runtime/progress-recorded" || references.len()>16 || !matches!(
        value.pointer("/event/event").and_then(Value::as_str),Some("steering_inputs"|"steering_deferred")) {
        return Err(invalid("input source references require an exact Runtime delivery"));
    }
    let inputs=value.pointer_mut("/event/inputs").and_then(Value::as_array_mut).ok_or_else(||invalid("input reference slots are missing"))?;
    let mut seen=BTreeSet::new();
    for reference in references {
        if !seen.insert(reference.index) || reference.fields.is_empty() || reference.fields.len()>3
            || !reference.fields.iter().any(|name|name=="text") {
            return Err(invalid("input source reference has invalid slots"));
        }
        let delivered=inputs.get_mut(reference.index).and_then(Value::as_object_mut).ok_or_else(||invalid("input source slot is missing"))?;
        if delivered.get("receipt_operation_id").and_then(Value::as_str)!=Some(reference.claim_event_id.as_ref())
            || delivered.get("message_id").and_then(Value::as_str)!=Some(reference.input_event_id.as_ref()) {
            return Err(invalid("input source reference differs from its delivery identity"));
        }
        let (input_id,original)=original_input(tx,&event.agent_session_id,&reference.claim_event_id,&event.correlation_id,Some(event.seq)).await?;
        if input_id!=reference.input_event_id {return Err(invalid("input source reference changed"));}
        let mut fields=BTreeSet::new();
        for name in reference.fields {
            if !fields.insert(name.clone()) || delivered.contains_key(&name) {return Err(invalid("input source slot contains a duplicated body"));}
            delivered.insert(name.clone(),field(&original,&name)?);
        }
    }
    Ok(())
}

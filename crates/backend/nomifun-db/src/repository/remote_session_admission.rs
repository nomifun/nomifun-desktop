//! Admission consumes the original Remote cancellation facts. No second
//! cancellation state or invocation ledger is maintained here.
use sqlx::{Sqlite, Transaction};
use crate::DbError;

pub async fn remote_session_admitted_tx(tx:&mut Transaction<'_,Sqlite>, session_id:&str)->Result<bool,DbError> {
    let binding:Option<(Option<String>,Option<i64>)>=sqlx::query_as("SELECT remote_binding_id,remote_binding_version FROM agent_sessions WHERE agent_session_id=?")
        .bind(session_id).fetch_optional(&mut **tx).await?;
    let Some((binding,version))=binding else {return Err(DbError::NotFound("Remote admission Session".into()));};
    let Some(binding)=binding else {return Ok(true);};
    let projection:Option<(String,String,i64)>=sqlx::query_as("SELECT state,remote_binding_id,binding_version FROM nomi_remote_sessions WHERE agent_session_id=?")
        .bind(session_id).fetch_optional(&mut **tx).await?;
    let Some((state,remote_binding,binding_version))=projection else {return Err(DbError::Init("Remote Session has no original projection".into()));};
    if remote_binding!=binding || version!=Some(binding_version) {return Err(DbError::Init("Remote Session binding differs from its original projection".into()));}
    match state.as_str() {
        "cancelled"|"failed"=>return Ok(false),
        "opening"|"ready"=>{},
        _=>return Err(DbError::Init("Remote Session has an invalid original state".into())),
    }
    // Read the latest fact for each original command, not just the most recent
    // cancellation globally: rejecting one command cannot erase another fence.
    let events:Vec<(String,String)>=sqlx::query_as("SELECT event.event_type,event.payload_json FROM nomi_remote_events event \
        WHERE event.agent_session_id=? AND event.event_type IN ('session/cancel-requested','session/cancel-unknown','session/cancel-rejected','session/cancelled') \
        AND NOT EXISTS(SELECT 1 FROM nomi_remote_events later WHERE later.agent_session_id=event.agent_session_id AND later.seq>event.seq \
            AND later.event_type IN ('session/cancel-requested','session/cancel-unknown','session/cancel-rejected','session/cancelled') \
            AND json_extract(later.payload_json,'$.operation_key_digest') IS json_extract(event.payload_json,'$.operation_key_digest'))")
        .bind(session_id).fetch_all(&mut **tx).await?;
    let mut admitted=true;
    for (kind,raw) in events {
        let payload:serde_json::Value=serde_json::from_str(&raw).map_err(|error|DbError::Init(format!("invalid Remote cancellation payload: {error}")))?;
        if payload.get("operation_key_digest").and_then(serde_json::Value::as_str).is_none_or(str::is_empty) {
            return Err(DbError::Init("Remote cancellation has no original command identity".into()));
        }
        if matches!(kind.as_str(),"session/cancel-requested"|"session/cancel-unknown"|"session/cancelled") {admitted=false;}
    }
    Ok(admitted)
}

pub async fn ensure_remote_session_admitted_tx(tx:&mut Transaction<'_,Sqlite>,session_id:&str)->Result<(),DbError> {
    if !remote_session_admitted_tx(tx,session_id).await? {return Err(DbError::Conflict("Remote Session has an original cancellation fence".into()));}
    Ok(())
}

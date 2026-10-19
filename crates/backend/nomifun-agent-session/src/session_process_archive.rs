//! Original native output in the existing Session Payload chain. The exact
//! reap event is the only root; these bytes cannot admit work or model input.
use super::*;
use nomifun_agent_contracts::{SessionProcessOutputArchiveV1,SessionProcessReapedV1};
use nomifun_engine_core::{EngineProcessPoll,EngineProcessTerminalMetadata};

pub(super) const OUTPUT_MEDIA:&str="application/vnd.nomifun.native-process-output+json-fragment";
pub(super) const RECEIPT_MEDIA:&str="application/vnd.nomifun.native-process-reaped+json";

pub(super) fn is_archive_media(media:&str)->bool {matches!(media,OUTPUT_MEDIA|RECEIPT_MEDIA)}

pub(super) fn validate_direct_reference(kind:&str,media:&str)->Result<(),SessionStoreError> {
    if is_archive_media(media)&&(kind!=nomifun_agent_contracts::SESSION_PROCESS_REAPED||media!=RECEIPT_MEDIA) {
        return Err(SessionStoreError::InvalidPayload("native archive Payload is only referenced by its exact process reap root".into()));
    }
    Ok(())
}

pub(super) async fn model_payload_bytes_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId)->Result<i64,SessionStoreError> {
    model_payload_bytes_inner_tx(tx,session,false).await
}

pub(super) async fn native_model_payload_bytes_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId)->Result<i64,SessionStoreError> {
    model_payload_bytes_inner_tx(tx,session,true).await
}

async fn model_payload_bytes_inner_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,exclude_file_observation:bool)->Result<i64,SessionStoreError> {
    // The media label alone is not classification authority: only the actual
    // canonical reap root and its same-Session typed references exclude bytes.
    // Extract each root's references once, then use SQLite's materialized set
    // index. A long Session must not scan every old archive for every Payload.
    Ok(sqlx::query_scalar(MODEL_PAYLOAD_BYTES_SQL)
        .bind(session.as_ref()).bind(RECEIPT_MEDIA).bind(RECEIPT_MEDIA).bind(OUTPUT_MEDIA)
        .bind(session.as_ref()).bind(exclude_file_observation).bind(session.as_ref()).fetch_one(&mut **tx).await?)
}

const MODEL_PAYLOAD_BYTES_SQL:&str="WITH native_roots AS MATERIALIZED ( \
    SELECT root.payload_id,root.body FROM agent_events e JOIN agent_payloads root \
      ON root.payload_id=e.payload_id AND root.session_id=e.session_id \
      WHERE e.session_id=? AND e.kind='session/process-reaped' AND e.producer_id='session_api' AND root.media_type=? \
    ), native_archive_payloads(payload_id,media_type) AS MATERIALIZED ( \
      SELECT payload_id,? FROM native_roots \
      UNION SELECT leaf.value,? FROM native_roots,json_each(json_extract(CAST(native_roots.body AS TEXT),'$.value.output_archive.chunks')) leaf \
        WHERE leaf.type='text' \
    ), file_observations AS MATERIALIZED ( \
      SELECT DISTINCT payload_id FROM agent_events WHERE session_id=? AND ? \
        AND kind='turn/workspace-files-observed' AND payload_id IS NOT NULL \
    ) SELECT COALESCE(SUM(p.byte_len),0) FROM agent_payloads p \
      LEFT JOIN native_archive_payloads archive ON archive.payload_id=p.payload_id AND archive.media_type=p.media_type \
      LEFT JOIN file_observations files ON files.payload_id=p.payload_id \
      WHERE p.session_id=? AND archive.payload_id IS NULL AND files.payload_id IS NULL";

#[cfg(test)]
#[path="session_process_archive_query_tests.rs"]
mod query_tests;

fn chunk_id(key:&str,index:usize)->SessionPayloadId {format!("{key}:output:{index}").into()}

pub(super) fn prepare(receipt:&mut SessionProcessReapedV1,poll:&EngineProcessPoll,key:&str,bytes:Option<&[u8]>)->Result<Vec<SessionPayloadRecord>,SessionStoreError> {
    let Some(bytes)=bytes else {return Ok(vec![]);};
    let terminal=poll.terminal_metadata().ok_or_else(||SessionStoreError::InvalidPayload("process archive needs its real confirmed native terminal".into()))?;
    let chunks:Vec<_>=bytes.chunks(MAX_SINGLE_PAYLOAD_BYTES).enumerate().map(|(index,bytes)|SessionPayloadRecord {
        payload_id:chunk_id(key,index),agent_session_id:receipt.retention.agent_session_id.clone(),media_type:OUTPUT_MEDIA.into(),
        byte_len:bytes.len() as u64,digest:digest_bytes(bytes),body:SessionPayloadBody::Base64(BASE64.encode(bytes)),
    }).collect();
    receipt.output_archive=Some(SessionProcessOutputArchiveV1 {terminal:StrictJsonValue(serde_json::to_value(terminal)?),
        byte_len:bytes.len() as u64,digest:digest_bytes(bytes),chunks:chunks.iter().map(|chunk|chunk.payload_id.clone()).collect()});
    receipt.terminal=None;
    receipt.output_replay=None;
    Ok(chunks)
}

/// Portable physical proof validates the typed root, never a local decoder or
/// raw archive availability. Missing/corrupt bytes cannot resurrect a process;
/// an explicit output read below still fails closed without any text fallback.
pub(super) fn terminal(archive:&SessionProcessOutputArchiveV1,key:&str)->Result<EngineProcessTerminalMetadata,SessionStoreError> {
    let count=archive.byte_len.div_ceil(MAX_SINGLE_PAYLOAD_BYTES as u64);
    if count==0||u64::try_from(archive.chunks.len()).ok()!=Some(count)
        ||archive.chunks.iter().enumerate().any(|(index,id)|*id!=chunk_id(key,index))
        ||archive.digest.as_ref().len()!=64||!archive.digest.as_ref().bytes().all(|byte|byte.is_ascii_hexdigit()&&!byte.is_ascii_uppercase()) {
        return Err(SessionStoreError::InvalidPayload("process archive differs from its exact original Payload manifest".into()));
    }
    let metadata:EngineProcessTerminalMetadata=serde_json::from_value(archive.terminal.0.clone())?;
    if !metadata.is_confirmed_reaped() {return Err(SessionStoreError::InvalidPayload("process archive has no confirmed native reap".into()));}
    Ok(metadata)
}

pub(super) async fn read_tx(tx:&mut Transaction<'_,Sqlite>,receipt:&SessionProcessReapedV1,key:&str,cursor:u64)->Result<EngineProcessPoll,SessionStoreError> {
    let archive=receipt.output_archive.as_ref().ok_or(SessionStoreError::ExecutionFenced)?;
    let metadata=terminal(archive,key)?;
    let mut bytes=Vec::new();
    for (index,id) in archive.chunks.iter().enumerate() {
        let payload=payload_by_id_tx(tx,id.as_ref()).await?.map(payload_from_row).transpose()?
            .ok_or_else(||SessionStoreError::InvalidPayload("original native output Payload is missing".into()))?;
        if payload.agent_session_id!=receipt.retention.agent_session_id||payload.media_type!=OUTPUT_MEDIA
            ||!matches!(&payload.body,SessionPayloadBody::Base64(_)) {
            return Err(SessionStoreError::InvalidPayload("native output Payload belongs to another source".into()));
        }
        validate_payload_contents(&payload)?;
        let chunk=logical_payload_bytes(&payload.body)?;
        let expected=if index+1==archive.chunks.len() {archive.byte_len-(index as u64*MAX_SINGLE_PAYLOAD_BYTES as u64)}else{MAX_SINGLE_PAYLOAD_BYTES as u64};
        if chunk.len() as u64!=expected {return Err(SessionStoreError::InvalidPayload("native output Payload chunk has a different original extent".into()));}
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() as u64!=archive.byte_len||digest_bytes(&bytes)!=archive.digest {
        return Err(SessionStoreError::InvalidPayload("original native output archive digest differs".into()));
    }
    let replay:nomifun_engine_core::EngineProcessOutputReplay=serde_json::from_slice(&bytes)?;
    metadata.with_output(&replay,cursor).map_err(|error|SessionStoreError::InvalidPayload(error.to_string()))
}

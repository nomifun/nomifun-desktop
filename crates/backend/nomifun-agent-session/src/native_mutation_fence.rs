//! Optional native target proof for opt-in consumers of the existing writer.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeTurnMutationFence {
    pub binding_version: u64,
    pub execution_generation: u64,
}

#[derive(Clone,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeInputContextFence {
    pub binding_version:u64,pub context_floor:u64,
    #[serde(default,skip_serializing_if="is_false")]
    pub supersede_model_step:bool,
}
fn is_false(value:&bool)->bool{!*value}

pub(super) async fn validate_input_tx(tx:&mut Transaction<'_,Sqlite>,session:&AgentSessionId,fence:&NativeInputContextFence)->Result<(),SessionStoreError> {
    let current=live_session_by_id_tx(tx,session.as_ref()).await?;
    let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
        .bind(session.as_ref()).fetch_one(&mut **tx).await?;
    if current.agent_binding.binding_version!=fence.binding_version||as_u64(floor,"input context floor")?!=fence.context_floor{return Err(SessionStoreError::ExecutionFenced);}Ok(())
}

pub(super) async fn validate_tx(tx:&mut Transaction<'_,Sqlite>,session_id:&AgentSessionId,operation:&OperationId,fence:&NativeTurnMutationFence)
    ->Result<(),SessionStoreError> {
    if fence.binding_version==0 {return Err(SessionStoreError::ExecutionFenced);}
    let session=live_session_by_id_tx(tx,session_id.as_ref()).await?;
    let generation:Option<i64>=sqlx::query_scalar("SELECT execution_generation FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(session_id.as_ref()).bind(operation.as_ref()).fetch_optional(&mut **tx).await?;
    if session.agent_binding.binding_version!=fence.binding_version
        ||generation.map(|value|as_u64(value,"native mutation generation")).transpose()?!=Some(fence.execution_generation) {
        return Err(SessionStoreError::ExecutionFenced);
    }
    Ok(())
}

pub(super) fn validate_replay(payload:&SessionEventPayloadRef,fence:&NativeTurnMutationFence)->Result<(),SessionStoreError> {
    let SessionEventPayloadRef::InlineJson(value)=payload else {return Err(SessionStoreError::IdempotencyConflict("native mutation replay has no original proof".into()));};
    if value.0.get("native_target_fence")!=Some(&serde_json::to_value(fence)?) {
        return Err(SessionStoreError::IdempotencyConflict("native mutation replay differs from its original binding or generation".into()));
    }
    Ok(())
}

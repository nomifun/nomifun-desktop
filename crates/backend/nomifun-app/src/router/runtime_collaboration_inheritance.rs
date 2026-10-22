//! An explicit reply to a canonical question may continue that task's exact
//! delegated work. Ordinary historical references grant no execution scope.
use super::*;
use nomifun_db::sqlx;

async fn retire_deleted_consumption(pool:&nomifun_db::SqlitePool,owner:&str,progress:&mut AgentCollaborationProgress)
    ->Result<(),AppError> {
    for id in progress.results.keys().filter(|id|!progress.pending_child_executions.contains(*id)).cloned().collect::<Vec<_>>() {
        let deleted:Option<Option<i64>>=sqlx::query_scalar(
            "SELECT deleted_at FROM agent_executions WHERE execution_id=? AND user_id=?")
            .bind(&id).bind(owner).fetch_optional(pool).await.map_err(error)?;
        if matches!(deleted,Some(Some(_))) {
            // The original results were already consumed. Deleting that
            // completed domain record cannot make it a required live scope
            // for a later human answer. Pending work is never pruned here.
            progress.results.remove(&id);
            progress.workspace_observations.remove(&id);
        }
    }
    Ok(())
}

pub(super) async fn load(host:&ConversationRuntimeHost, admitted:&super::super::engine_session_host::EngineTurnReceipt)
    ->Result<Option<AgentCollaborationProgress>,AppError> {
    let owner=host.session_host.session_owner()?;
    let session:AgentSessionId=host.options.conversation_id.clone().into();
    let current:OperationId=admitted.operation_id().to_owned().into();
    let Some(source)=owner.collaboration_reply_source_turn(&host.principal.principal_id,&session,&current).await? else {
        return Ok(None);
    };
    let mut window=host.session_host.read_history_exact(admitted,source.as_ref()).await?;
    if window.turns.len()!=1 {return Err(error("question continuation has no unique canonical Runtime history"));}
    let turn=window.turns.remove(0);
    let events=super::super::unified_runtime_history::decode_turn_events(turn.records,&turn.receipt_status,turn.unstarted_terminal)?
        .ok_or_else(||error("question continuation has no canonical Runtime events"))?;
    let mut progress=collaboration_progress_from_events(&events).map_err(error)?;
    retire_deleted_consumption(owner.collaboration_database(),&host.principal.principal_id,&mut progress).await?;
    if progress.is_empty() {return Ok(None);}
    let scopes=owner.collaboration_source_turns(&host.principal.principal_id,&session,&current).await?;
    for id in progress.pending_child_executions.iter().chain(progress.results.keys()).collect::<BTreeSet<_>>() {
        let mut candidates:Vec<String>=sqlx::query_scalar(
            "SELECT json_extract(initial_plan_input,'$.source') FROM agent_executions WHERE execution_id=? AND user_id=? AND json_type(initial_plan_input,'$.source')='object'")
            .bind(id).bind(&host.principal.principal_id).fetch_all(owner.collaboration_database()).await.map_err(error)?;
        candidates.extend(sqlx::query_scalar::<_,String>(
            "SELECT json_extract(event.payload,'$.delegation_source') FROM agent_execution_events event JOIN agent_executions execution ON execution.execution_id=event.execution_id WHERE event.execution_id=? AND execution.user_id=? AND json_type(event.payload,'$.delegation_source')='object'")
            .bind(id).bind(&host.principal.principal_id).fetch_all(owner.collaboration_database()).await.map_err(error)?);
        let mut proven=false;
        for candidate in candidates {
            let source:nomifun_agent_execution::AgentExecutionSource=serde_json::from_str(&candidate).map_err(error)?;
            if source.agent_session_id!=session.as_ref() || !scopes.iter().any(|scope|scope.as_ref()==source.turn_operation_id) {continue;}
            let effect=owner.canonical().store().read_effect(&session,&source.effect_id).await.map_err(error)?
                .ok_or_else(||error("continued delegation has no original Effect"))?;
            if effect.state!=nomifun_agent_session::AgentEffectState::Returned || effect.owner_domain!="wave5.product"
                || effect.capability_module.as_ref()!="agent.collaboration" || !matches!(effect.action_id.as_ref(),"agent/delegate"|"agent/fork")
                || effect.turn_id.as_ref()!=source.turn_operation_id || effect.operation_id.as_ref()!=source.tool_operation_id {
                return Err(error("continued delegation differs from its admitted Effect"));
            }
            owner.canonical().store().committed_tool_call_causation(&session,&effect.turn_id,&effect.operation_id,
                &effect.capability_module,&effect.action_id).await.map_err(error)?;
            proven=true;break;
        }
        if !proven {return Err(error("question reply cannot inherit unrelated delegated work"));}
    }
    Ok(Some(progress))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn deleting_consumed_history_does_not_block_a_question_reply_or_remove_pending_authority_checks() {
        let database=nomifun_db::init_database_memory().await.unwrap();let pool=database.pool();
        let owner=uuid::Uuid::now_v7().to_string();let other=uuid::Uuid::now_v7().to_string();
        let consumed=uuid::Uuid::now_v7().to_string();let pending=uuid::Uuid::now_v7().to_string();
        let live=uuid::Uuid::now_v7().to_string();let foreign=uuid::Uuid::now_v7().to_string();
        let missing=uuid::Uuid::now_v7().to_string();
        for (id,user,deleted) in [(&consumed,&owner,Some(2_i64)),(&pending,&owner,Some(2)),
            (&live,&owner,None),(&foreign,&other,Some(2))] {
            sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,deleted_at,created_at,updated_at) VALUES(?,?,'canonical question continuation','completed','fixed','automatic','automatic','{}',?,1,2)")
                .bind(id).bind(user).bind(deleted).execute(pool).await.unwrap();
        }
        let mut progress=AgentCollaborationProgress::default();
        for id in [&consumed,&pending,&live,&foreign,&missing] {
            progress.results.insert(id.clone(),nomifun_agent_runtime::AgentChildResultConsumption {
                sequence:7,source_digests:BTreeMap::from([("child".into(),"a".repeat(64))]),
                all_terminal_observed:true,..Default::default()
            });
            progress.workspace_observations.insert(id.clone(),BTreeMap::from([("step:child".into(),"b".repeat(64))]));
        }
        progress.pending_child_executions.insert(pending.clone());
        let pending_before=progress.pending_child_executions.clone();
        retire_deleted_consumption(pool,&owner,&mut progress).await.unwrap();
        assert!(!progress.results.contains_key(&consumed),"consumed deleted history is not a new live execution obligation");
        assert!(!progress.workspace_observations.contains_key(&consumed));
        for id in [&pending,&live,&foreign,&missing] {assert!(progress.results.contains_key(id));}
        assert_eq!(progress.pending_child_executions,pending_before,"pending work must still pass its original source proof");
        progress.validate().unwrap();
        let deleted:Option<i64>=sqlx::query_scalar("SELECT deleted_at FROM agent_executions WHERE execution_id=?")
            .bind(&pending).fetch_one(pool).await.unwrap();
        assert_eq!(deleted,Some(2),"the reader cannot reopen a deleted Execution");
    }
}

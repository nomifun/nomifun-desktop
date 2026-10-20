use super::*;

async fn insert_task(services: &crate::services::AppServices, owner: &PrincipalRef, execution: &str, step: &str, old: &str, current: &str) {
    sqlx::query("INSERT INTO agent_executions(execution_id,user_id,goal,status,plan_gate,adaptation_policy,decision_policy,delegation_policy,initial_plan_input,created_at,updated_at) \
        VALUES(?,?,'exact task inspection','running','automatic','fixed','automatic','automatic','{}',1,1)")
        .bind(execution).bind(&owner.principal_id).execute(services.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,introduced_in_revision,created_at,updated_at) \
        VALUES(?,?,'Original task','inspect original resource owner','agent','running',0,1,1)")
        .bind(step).bind(execution).execute(services.database.pool()).await.unwrap();
    for (id, number, status) in [(old, 0, "cancelled"), (current, 1, "running")] {
        sqlx::query("INSERT INTO agent_execution_attempts(attempt_id,execution_id,step_id,attempt_no,status,trigger_reason,created_at,updated_at) VALUES(?,?,?,?,?,'initial',1,1)")
            .bind(id).bind(execution).bind(step).bind(number).bind(status).execute(services.database.pool()).await.unwrap();
    }
}

#[tokio::test]
async fn exact_task_activity_reads_historical_attempt_and_never_selects_its_successor() {
    let (_root,services,router,state,session,principal)=fixture().await;
    let execution=uuid::Uuid::now_v7().to_string();let step=uuid::Uuid::now_v7().to_string();
    let old=uuid::Uuid::now_v7().to_string();let current=uuid::Uuid::now_v7().to_string();
    insert_task(&services,&principal,&execution,&step,&old,&current).await;
    // Even an AutoWork root is inspected as this exact historical Attempt.
    // Reading a task detail cannot silently widen its Stop target to the run.
    let requirement=uuid::Uuid::now_v7().to_string();
    sqlx::query("UPDATE agent_executions SET initial_plan_input=? WHERE execution_id=?")
        .bind(json!({"mode":"automation","source":{"requirement_id":requirement,"claim_generation":1,"operation_id":"original-automation-run"}}).to_string())
        .bind(&execution).execute(services.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
        .bind(session.as_ref()).bind(&execution).execute(services.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'automation',?,?,0,1,1)")
        .bind(session.as_ref()).bind(&execution).bind(&step).bind(&old).execute(services.database.pool()).await.unwrap();
    let before=state.session_owner.canonical.store().read_events(&session,None,500).await.unwrap();
    let path=format!("/api/agent-activities?execution_id={execution}&step_id={step}&attempt_id={old}");
    let original=request(&router,"GET",&path,Value::Null).await;
    let items=original["items"].as_array().unwrap();assert_eq!(items.len(),1);assert!(original["next_cursor"].is_null());
    assert_eq!(items[0]["target"]["kind"],"execution_task");assert_eq!(items[0]["target"]["attempt_id"],old);
    assert_eq!(items[0]["kind"],"delegated_task");assert!(items[0]["delegation_target"].is_null());
    assert_eq!(items[0]["target"]["step_id"],step);assert_eq!(items[0]["target"]["execution_id"],execution);
    assert_eq!(items[0]["stop_accepted"],true);assert_eq!(items[0]["execution_stopped"],false);
    assert_eq!(items[0]["can_retry_cleanup"],true);assert_eq!(items[0]["can_stop"],false);
    assert!(items[0]["resources"].as_array().unwrap().iter().all(|resource| resource["state"]!="released"));
    let next=request(&router,"GET",&path.replace(&old,&current),Value::Null).await;
    assert_eq!(next["items"][0]["target"]["attempt_id"],current);assert_eq!(next["items"][0]["stop_accepted"],false);
    sqlx::query("UPDATE agent_execution_attempts SET status='interrupted' WHERE attempt_id=?").bind(&old).execute(services.database.pool()).await.unwrap();
    let interrupted=request(&router,"GET",&path,Value::Null).await;
    assert_eq!(interrupted["items"][0]["target"]["attempt_id"],old);
    assert_eq!(interrupted["items"][0]["can_stop"],false,"a historical Attempt must not offer a business Stop that would hit its successor Step");
    assert_eq!(interrupted["items"][0]["can_retry_cleanup"],true,"the exact original cleanup responsibility remains available");
    assert_eq!(state.session_owner.canonical.store().read_events(&session,None,500).await.unwrap(),before);
    let status:String=sqlx::query_scalar("SELECT status FROM agent_execution_attempts WHERE attempt_id=?").bind(&current).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(status,"running");
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn exact_task_activity_requires_the_complete_owned_identity_without_global_paging() {
    let (_root,services,router,_state,_session,principal)=fixture().await;
    let execution=uuid::Uuid::now_v7().to_string();let step=uuid::Uuid::now_v7().to_string();
    let old=uuid::Uuid::now_v7().to_string();let current=uuid::Uuid::now_v7().to_string();
    let mut foreign=principal.clone();foreign.principal_id=uuid::Uuid::now_v7().to_string();
    insert_task(&services,&foreign,&execution,&step,&old,&current).await;
    for (path,expected) in [
        (format!("/api/agent-activities?execution_id={execution}"),StatusCode::BAD_REQUEST),
        (format!("/api/agent-activities?execution_id=invalid&step_id={step}&attempt_id={old}"),StatusCode::BAD_REQUEST),
        (format!("/api/agent-activities?execution_id={execution}&step_id={step}&attempt_id={old}&after_cursor=ignored"),StatusCode::BAD_REQUEST),
        (format!("/api/agent-activities?execution_id={execution}&step_id={step}&attempt_id={old}"),StatusCode::NOT_FOUND),
    ] {
        let response=router.clone().oneshot(axum::http::Request::builder().method("GET").uri(path)
            .header("x-nomi-local-trust",TRUST).body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),expected);
    }
    services.shutdown_nomi_core_host().await.unwrap();
    services.database.close().await;
}

#[tokio::test]
async fn exact_task_activity_rejects_a_mixed_owned_triple_instead_of_falling_back_to_latest() {
    let (_root,services,router,_state,_session,principal)=fixture().await;
    let execution=uuid::Uuid::now_v7().to_string();let step=uuid::Uuid::now_v7().to_string();
    let old=uuid::Uuid::now_v7().to_string();let current=uuid::Uuid::now_v7().to_string();
    insert_task(&services,&principal,&execution,&step,&old,&current).await;
    let other_step=uuid::Uuid::now_v7().to_string();
    sqlx::query("INSERT INTO agent_execution_steps(step_id,execution_id,title,spec,kind,status,introduced_in_revision,created_at,updated_at) VALUES(?,?,'Other task','independent task','agent','pending',0,1,1)")
        .bind(&other_step).bind(&execution).execute(services.database.pool()).await.unwrap();
    let path=format!("/api/agent-activities?execution_id={execution}&step_id={other_step}&attempt_id={old}");
    let response=router.clone().oneshot(axum::http::Request::builder().method("GET").uri(path)
        .header("x-nomi-local-trust",TRUST).body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::NOT_FOUND);
    let exact=request(&router,"GET",&format!("/api/agent-activities?execution_id={execution}&step_id={step}&attempt_id={old}&limit=1"),Value::Null).await;
    assert_eq!(exact["items"].as_array().unwrap().len(),1);assert!(exact["next_cursor"].is_null());
    assert_eq!(exact["items"][0]["target"]["attempt_id"],old);
    services.shutdown_nomi_core_host().await.unwrap();services.database.close().await;
}

#[tokio::test]
async fn exact_task_activity_uses_the_committed_final_continuation_after_runtime_queue_retirement() {
    let (_root,services,router,state,session,principal)=fixture().await;
    let execution=uuid::Uuid::now_v7().to_string();let step=uuid::Uuid::now_v7().to_string();
    let old=uuid::Uuid::now_v7().to_string();let current=uuid::Uuid::now_v7().to_string();
    insert_task(&services,&principal,&execution,&step,&old,&current).await;
    sqlx::query("UPDATE agent_execution_attempts SET status='completed',runtime_state=NULL WHERE attempt_id=?")
        .bind(&old).execute(services.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) VALUES(?,?,'attempt',?,?,0,1,1)")
        .bind(session.as_ref()).bind(&execution).bind(&step).bind(&old).execute(services.database.pool()).await.unwrap();
    let seq=i64::try_from(state.session_owner.canonical.store().current_cursor(&session).await.unwrap().seq).unwrap();
    let key=format!("{old}:decision:final");
    let operation=format!("turn:user:{}:{}:{key}",principal.principal_id,session.as_ref());
    let started=uuid::Uuid::now_v7().to_string();let terminal=uuid::Uuid::now_v7().to_string();
    for (number,id,kind) in [(seq+1,&started,"turn/started"),(seq+2,&terminal,"turn/completed")] {
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'exact-task-fixture',?,?,1,?,'{}')")
            .bind(session.as_ref()).bind(number).bind(id).bind(id).bind(kind).bind(&operation).execute(services.database.pool()).await.unwrap();
    }
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,?,?,?)")
        .bind(session.as_ref()).bind(&operation).bind(&operation).bind(&operation).bind(&started).bind(&terminal)
        .bind(seq+1).bind(seq+1).bind(seq+2).execute(services.database.pool()).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET last_seq=?,status='ready',active_turn_id=NULL WHERE session_id=?")
        .bind(seq+2).bind(session.as_ref()).execute(services.database.pool()).await.unwrap();
    sqlx::query("UPDATE agent_sessions SET next_seq=? WHERE agent_session_id=?")
        .bind(seq+3).bind(session.as_ref()).execute(services.database.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_execution_events(execution_id,sequence,event_type,step_id,attempt_id,actor_type,on_behalf_of_user_id,payload,created_at) VALUES(?,1,'attempt_changed',?,?,'system',?,?,1)")
        .bind(&execution).bind(&step).bind(&old).bind(&principal.principal_id)
        .bind(json!({"attempt_status":"completed","step_status":"completed","turn_operation_key":key,"canonical_operation_id":operation,"terminal_event_id":terminal}).to_string())
        .execute(services.database.pool()).await.unwrap();
    sqlx::query("UPDATE agent_executions SET event_sequence=1 WHERE execution_id=?").bind(&execution).execute(services.database.pool()).await.unwrap();
    let before=state.session_owner.canonical.store().read_events(&session,None,500).await.unwrap();
    let path=format!("/api/agent-activities?execution_id={execution}&step_id={step}&attempt_id={old}");
    let activity=request(&router,"GET",&path,Value::Null).await;
    let resource=activity["items"][0]["resources"].as_array().unwrap().iter().find(|resource|resource["kind"]=="effects").unwrap();
    assert_eq!(resource["state"],"released","a retired live queue must not send a continued task back to an absent initial Turn");
    assert_eq!(activity["items"][0]["target"]["attempt_id"],old);
    assert_eq!(state.session_owner.canonical.store().read_events(&session,None,500).await.unwrap(),before);
    services.shutdown_nomi_core_host().await.unwrap();services.database.close().await;
}

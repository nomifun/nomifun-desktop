use super::*;
use super::scoped_stop::Fixture;
use nomifun_db::ConversationCleanupTurnTarget;

async fn start_on(fixture:&Fixture,title:&str,session:&str)->nomifun_db::AgentExecutionStepDetailRow {
    let task=fixture.task(title).await;
    let queued=fixture.repository.create_attempt(&fixture.owner,&fixture.execution_id,&task.step.step_id,task.step.version,None,
        &CreateAgentExecutionAttemptParams {participant_id:task.step.assigned_participant_id,start_immediately:false,
            trigger_reason:"same-session-exact-owner".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},
        &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let attempt=&queued.current_attempt.as_ref().unwrap().attempt;
    fixture.repository.start_attempt(&fixture.owner,&fixture.execution_id,&queued.step.step_id,queued.step.version,
        &attempt.attempt_id,attempt.version,session,AgentExecutionAttemptSessionKind::ChildAttempt,None,
        &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap()
}

#[tokio::test]
async fn cleanup_acknowledges_only_the_exact_attempt_while_a_successor_shares_the_session() {
    let fixture=Fixture::memory(&["first","second","successor"],&[]).await;
    let first=fixture.start("first").await;
    let session=first.current_attempt.as_ref().unwrap().conversation_id.clone().unwrap();
    fixture.complete("first").await;
    let second=start_on(&fixture,"second",&session).await;
    fixture.complete("second").await;
    let successor=start_on(&fixture,"successor",&session).await;
    // Same Attempt duplicate links may share one proven Stop. A distinct
    // Attempt in this Session must retain its own external cleanup receipt.
    nomifun_db::sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,step_id,attempt_id,active,created_at,updated_at) SELECT conversation_id,execution_id,relation,step_id,attempt_id,0,1,1 FROM conversation_execution_links WHERE attempt_id=? LIMIT 1")
        .bind(&first.current_attempt.as_ref().unwrap().attempt.attempt_id).execute(fixture.db.pool()).await.unwrap();
    let page=fixture.repository.list_pending_conversation_cleanups(Some(&fixture.execution_id),None,100).await.unwrap();
    assert_eq!(page.cleanups.len(),2,"an unrelated active successor cannot hide exact older cleanup owners");
    let selected=page.cleanups.iter().find(|cleanup|cleanup.step_id==first.step.step_id).unwrap();
    assert_eq!(selected.target,ConversationCleanupTurnTarget::Exact {operation_id:format!("turn:user:{}:{session}:{}:initial-turn",fixture.owner,selected.attempt_id)});
    assert!(fixture.repository.validate_conversation_cleanup(selected).await.unwrap());
    assert!(fixture.repository.mark_conversation_cleanup_completed_exact(selected,700).await.unwrap());
    let rows:Vec<(String,bool,Option<i64>)>=nomifun_db::sqlx::query_as("SELECT attempt_id,active,cleanup_completed_at FROM conversation_execution_links WHERE conversation_id=? ORDER BY id")
        .bind(&session).fetch_all(fixture.db.pool()).await.unwrap();
    let first_attempt=&first.current_attempt.as_ref().unwrap().attempt.attempt_id;
    assert!(rows.iter().filter(|row|&row.0==first_attempt).all(|row|row.2==Some(700)));
    let second_attempt=&second.current_attempt.as_ref().unwrap().attempt.attempt_id;
    assert!(rows.iter().filter(|row|&row.0==second_attempt).all(|row|row.2.is_none()));
    let successor_attempt=&successor.current_attempt.as_ref().unwrap().attempt.attempt_id;
    assert!(rows.iter().any(|row|&row.0==successor_attempt && row.1 && row.2.is_none()));
    assert_eq!(fixture.task("successor").await.step.status,"running");
    let remaining=fixture.repository.list_pending_conversation_cleanups(Some(&fixture.execution_id),None,100).await.unwrap();
    assert_eq!(remaining.cleanups.len(),1);assert_eq!(&remaining.cleanups[0].attempt_id,second_attempt);
}

#[tokio::test]
async fn soft_deleted_execution_cleanup_retains_the_original_continuation_target_and_owner() {
    let fixture=Fixture::memory(&["old-task","successor"],&[]).await;
    let old=fixture.start("old-task").await;
    let session=old.current_attempt.as_ref().unwrap().conversation_id.clone().unwrap();
    let old_attempt=&old.current_attempt.as_ref().unwrap().attempt.attempt_id;
    fixture.complete("old-task").await;
    let detail=fixture.detail().await;
    let key="the-original-authorized-continuation";
    fixture.repository.append_event(&fixture.owner,&fixture.execution_id,detail.execution.version,
        &NewAgentExecutionEvent {step_id:Some(old.step.step_id.clone()),attempt_id:Some(old_attempt.clone()),
            payload:serde_json::json!({"turn_operation_key":key}).to_string(),..event(AgentExecutionEventKind::AttemptChanged)}).await.unwrap();
    let detail=fixture.detail().await;
    fixture.repository.delete_execution(&fixture.owner,&fixture.execution_id,detail.execution.version,&event(AgentExecutionEventKind::Deleted)).await.unwrap();
    assert!(fixture.repository.get_attempt(&fixture.owner,&fixture.execution_id,&old.step.step_id,old_attempt).await.unwrap().is_none());
    let page=fixture.repository.list_pending_conversation_cleanups(Some(&fixture.execution_id),None,100).await.unwrap();
    assert_eq!(page.cleanups.len(),1);
    assert_eq!(page.cleanups[0].target,ConversationCleanupTurnTarget::Exact {operation_id:format!("turn:user:{}:{session}:{key}",fixture.owner)});
    assert!(fixture.repository.validate_conversation_cleanup(&page.cleanups[0]).await.unwrap());
    assert!(fixture.repository.mark_conversation_cleanup_completed_exact(&page.cleanups[0],800).await.unwrap());
    assert!(fixture.repository.list_pending_conversation_cleanups(Some(&fixture.execution_id),None,100).await.unwrap().cleanups.is_empty());
}

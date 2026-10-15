use super::*;
use serde_json::Value;
use crate::{KnowledgeWritebackStartRequest, MessageProjection};
use nomifun_agent_contracts::{KnowledgeWritebackNoticeStatus as Status, KnowledgeWritebackNoticeFile, KnowledgeWritebackSettledV1, TURN_KNOWLEDGE_WRITEBACK_STARTED, TURN_KNOWLEDGE_WRITEBACK_SETTLED};

struct Completed { session: AgentSessionLiveRecord, operation: OperationId, root: EventId, terminal: EventId }

async fn completed(store: &AgentSessionStore, key: &str) -> Completed {
    let (session,_) = create_ready(store,key).await;
    complete_in(store, session, key, None).await
}

async fn complete_in(store: &AgentSessionStore, session: AgentSessionLiveRecord, key: &str, legacy: Option<Value>) -> Completed {
    let operation = OperationId::from(format!("operation-{key}"));
    let (input,started) = store.start_turn(&session.agent_session_id,"session_api".into(),format!("input-{key}").into(),operation.clone(),
        StrictJsonValue(json!({"content":"save this learning", "admission":{"resolved_snapshot_ref":session.agent_binding.resolved_snapshot_ref}}))).await.unwrap();
    let root = input.ack.unwrap().event_id;
    let mut payload = json!({"finished_at_ms":1_990_000_000_000_i64});
    if let Some(legacy) = legacy { payload["knowledge_writeback"] = legacy; }
    let terminal = append(&session.agent_session_id, &format!("terminal-{key}"), "runtime_supervisor", &format!("terminal-{key}"),
        "turn/completed", operation.as_ref(), Some(started.ack.unwrap().event_id),payload);
    let terminal = store.append_event(&terminal).await.unwrap().ack.unwrap().event_id;
    Completed { session, operation, root, terminal }
}

fn request(f: &Completed, instance: &str) -> KnowledgeWritebackStartRequest {
    KnowledgeWritebackStartRequest { owner: owner(), agent_session_id:f.session.agent_session_id.clone(), operation_id:f.operation.clone(),
        resolved_snapshot_ref:f.session.agent_binding.resolved_snapshot_ref.clone(),binding_version:f.session.agent_binding.binding_version, execution_instance_id:instance.into() }
}

fn outcome(f: &Completed, status: Status) -> KnowledgeWritebackSettledV1 {
    let written = if matches!(status, Status::Written|Status::Partial) {
        vec![KnowledgeWritebackNoticeFile{ kb_id:Uuid::now_v7().to_string().into(), rel_path:"notes/reusable.md".into() }]
    } else {vec![]};
    KnowledgeWritebackSettledV1 {operation_id:f.operation.clone(),status,written,detail:None,diagnostic:None}
}

async fn summary(store: &AgentSessionStore, f: &Completed) -> MessageProjection {
    store.turn_history_for_operation(&owner(),&f.session.agent_session_id,&f.operation).await.unwrap().unwrap()
}

#[tokio::test]
async fn knowledge_writeback_settles_original_turn_without_blocking_or_mutating_the_next_turn() {
    let store=AgentSessionStore::open_in_memory().await.unwrap(); let f=completed(&store,"decoupled").await;
    let before=store.read_turn_receipt(&f.session.agent_session_id,&f.operation).await.unwrap();
    let terminal_before:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?").bind(f.terminal.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    let summary_before=summary(&store,&f).await;
    let start=store.start_knowledge_writeback(request(&f,"process-a")).await.unwrap(); assert!(start.first);
    assert!(!store.start_knowledge_writeback(request(&f,"process-a")).await.unwrap().first);
    assert_eq!(summary(&store,&f).await.projection["knowledge_writeback"]["status"],"running");
    assert!(!store.has_unsettled_effects(&f.session.agent_session_id).await.unwrap());
    store.start_turn(&f.session.agent_session_id,"session_api".into(),"next-input".into(),"next-turn".into(),StrictJsonValue(json!({"content":"keep chatting"}))).await.unwrap();
    let head_before=store.head(&f.session.agent_session_id).await.unwrap();
    let done=store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,outcome(&f,Status::Written)).await.unwrap();
    let head=store.head(&f.session.agent_session_id).await.unwrap(); assert_eq!(head.status,head_before.status); assert_eq!(head.active_turn_id,Some("next-turn".into()));
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&f.operation).await.unwrap(),before);
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT inline_json FROM agent_events WHERE event_id=?").bind(f.terminal.as_ref()).fetch_one(store.test_pool()).await.unwrap(),terminal_before);
    let turn_summary=summary(&store,&f).await;
    assert_eq!((turn_summary.first_seq,turn_summary.last_seq),(summary_before.first_seq,summary_before.last_seq));
    assert_eq!(turn_summary.projection["finished_seq"],summary_before.projection["finished_seq"]);
    assert_eq!(turn_summary.projection["finished_at_ms"],summary_before.projection["finished_at_ms"]);
    assert_eq!(turn_summary.projection["state"],"completed");assert_eq!(turn_summary.projection["knowledge_writeback"]["status"],"written");
    assert_eq!(turn_summary.projection["knowledge_writeback"]["event_seq"],done.ack.unwrap().seq);
    assert!(turn_summary.projection["finished_at_ms"].as_i64().is_some());
    assert_eq!(store.turn_history_for_sources(&f.session.agent_session_id,&[f.root.as_ref().into()]).await.unwrap()[0],turn_summary);
    let (page,_,_)=store.message_history_before(&f.session.agent_session_id,None,50).await.unwrap();assert!(page.contains(&turn_summary));
    let (old_page,_,_)=store.message_history_before(&f.session.agent_session_id,Some(summary_before.last_seq+1),50).await.unwrap();
    assert!(old_page.contains(&turn_summary),"late observation must hydrate the original page without moving its Turn");
    assert!(!old_page.iter().any(|item|item.projection.get("turn_operation_id").and_then(Value::as_str)==Some("next-turn")));
    let facts=store.chat_causality_facts(&f.session.agent_session_id,&f.operation).await.unwrap();
    assert!(!facts.events.iter().any(|event|matches!(event.kind.0.as_str(),TURN_KNOWLEDGE_WRITEBACK_STARTED|TURN_KNOWLEDGE_WRITEBACK_SETTLED)));
    store.rebuild_projections(&f.session.agent_session_id).await.unwrap(); assert_eq!(summary(&store,&f).await,turn_summary);
    assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().active_turn_id,Some("next-turn".into()));
}

#[tokio::test]
async fn knowledge_writeback_start_requires_exact_owner_completed_turn_snapshot_binding_and_context() {
    let store=AgentSessionStore::open_in_memory().await.unwrap(); let f=completed(&store,"fences").await;
    let mut wrong=request(&f,"process-a");wrong.owner.principal_id="foreign".into();assert!(store.start_knowledge_writeback(wrong).await.is_err());
    let mut wrong=request(&f,"process-a");wrong.resolved_snapshot_ref.snapshot_digest=digest('c');assert!(store.start_knowledge_writeback(wrong).await.is_err());
    let mut wrong=request(&f,"process-a");wrong.binding_version+=1;assert!(store.start_knowledge_writeback(wrong).await.is_err());
    let mut wrong=request(&f,"process-a");wrong.operation_id="missing-turn".into();assert!(store.start_knowledge_writeback(wrong).await.is_err());
    let current=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    clear_context(&store,&f.session.agent_session_id,"clear-before-writeback").await;assert!(store.start_knowledge_writeback(request(&f,"process-a")).await.is_err());
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap().seq,current.seq+1);
    let f=completed(&store,"binding-changed").await;let mut replacement=f.session.agent_binding.clone();replacement.binding_version+=1;replacement.resolved_snapshot_ref.snapshot_id="snapshot-replacement".into();replacement.resolved_snapshot_ref.snapshot_digest=digest('c');
    store.replace_session_agent_binding(&owner(),&f.session.agent_session_id,agent_switch(f.session.agent_binding.clone(),replacement.clone(),"knowledge-fence")).await.unwrap();
    assert!(store.start_knowledge_writeback(request(&f,"process-a")).await.is_err());
    let mut latest=request(&f,"process-a");latest.binding_version=replacement.binding_version;latest.resolved_snapshot_ref=replacement.resolved_snapshot_ref;
    assert!(store.start_knowledge_writeback(latest).await.is_err(),"current Snapshot cannot replace the original Turn admission");
    let f=completed(&store,"deleting-start").await;
    store.fence_delete(&DeleteAgentSessionCommand{operation_id:"delete-start".into(),agent_session_id:f.session.agent_session_id.clone(),owner_ref:owner(),requested_at:1_900_000_000_000}).await.unwrap();
    assert!(store.start_knowledge_writeback(request(&f,"process-a")).await.is_err());
}

#[tokio::test]
async fn knowledge_writeback_settlement_is_exact_single_terminal_and_idempotent() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=completed(&store,"settlement").await;
    assert!(store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&f.terminal,outcome(&f,Status::Failed)).await.is_err());
    let start=store.start_knowledge_writeback(request(&f,"process-a")).await.unwrap();
    let wrong=PrincipalRef{principal_kind:"user".into(),principal_id:"foreign".into()};
    assert!(store.settle_knowledge_writeback(&wrong,&f.session.agent_session_id,&start.start_event_id,outcome(&f,Status::Failed)).await.is_err());
    let mut wrong=outcome(&f,Status::Failed);wrong.operation_id="other-operation".into();assert!(store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,wrong).await.is_err());
    assert!(store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,outcome(&f,Status::Running)).await.is_err());
    let append=append(&f.session.agent_session_id,"generic-knowledge","runtime_supervisor","generic-knowledge",TURN_KNOWLEDGE_WRITEBACK_SETTLED,f.operation.as_ref(),Some(start.start_event_id.clone()),serde_json::to_value(outcome(&f,Status::Failed)).unwrap());
    assert!(store.append_event(&append).await.is_err(),"generic writer cannot inject observations");
    let mut out=outcome(&f,Status::Failed);
    let mut diagnostic=nomifun_agent_contracts::KnowledgeWritebackDiagnostic::new(nomifun_agent_contracts::KnowledgeWritebackStage::Stream);
    diagnostic.code=Some("USER_LLM_PROVIDER_TIMEOUT".into());diagnostic.elapsed_ms=Some(90_000);diagnostic.first_frame_ms=Some(900);
    diagnostic.received_frames=Some(120);diagnostic.received_bytes=Some(640_000);diagnostic.output_bytes=Some(9000);diagnostic.input_tokens_estimate=Some(5000);diagnostic.output_token_limit=Some(16_384);
    out.diagnostic=Some(diagnostic.clone());
    let result=store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,out.clone()).await.unwrap();
    assert!(!result.duplicate);let repeated=store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,out).await.unwrap();assert!(repeated.duplicate);assert_eq!(result.record,repeated.record);
    assert!(store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,outcome(&f,Status::Written)).await.is_err());
    assert_eq!(summary(&store,&f).await.projection["knowledge_writeback"]["status"],"failed");
    assert_eq!(summary(&store,&f).await.projection["knowledge_writeback"]["diagnostic"],serde_json::to_value(diagnostic).unwrap());
    assert!(summary(&store,&f).await.projection["knowledge_writeback"].get("detail").is_none());
    assert!(!store.chat_causality_facts(&f.session.agent_session_id,&f.operation).await.unwrap().events.iter().any(|event|event.kind.0==TURN_KNOWLEDGE_WRITEBACK_SETTLED));
}

#[tokio::test]
async fn knowledge_writeback_restart_interrupts_only_old_pending_process_observations_and_never_replays() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let old=completed(&store,"old-process").await;let current=completed(&store,"current-process").await;
    store.start_knowledge_writeback(request(&old,"old-instance")).await.unwrap();store.start_knowledge_writeback(request(&current,"current-instance")).await.unwrap();
    let deleting=completed(&store,"deleting-restart").await;store.start_knowledge_writeback(request(&deleting,"old-instance")).await.unwrap();
    store.fence_delete(&DeleteAgentSessionCommand{operation_id:"delete-restart".into(),agent_session_id:deleting.session.agent_session_id.clone(),owner_ref:owner(),requested_at:1_900_000_000_000}).await.unwrap();
    assert_eq!(store.interrupt_stale_knowledge_writebacks("current-instance").await.unwrap(),2);
    assert_eq!(store.interrupt_stale_knowledge_writebacks("current-instance").await.unwrap(),0);
    assert_eq!(summary(&store,&old).await.projection["knowledge_writeback"]["status"],"interrupted");
    assert_eq!(summary(&store,&current).await.projection["knowledge_writeback"]["status"],"running");
    assert!(!store.start_knowledge_writeback(request(&old,"old-instance")).await.unwrap().first);
    assert!(store.start_knowledge_writeback(request(&old,"current-instance")).await.is_err());
    assert!(!store.has_unsettled_effects(&old.session.agent_session_id).await.unwrap());
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE kind='turn/knowledge-writeback-settled' AND session_id=?").bind(deleting.session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(count,1);
}

#[tokio::test]
async fn knowledge_writeback_settlement_survives_reordered_results_and_delete_fence() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let first=completed(&store,"first-result").await;let start1=store.start_knowledge_writeback(request(&first,"instance")).await.unwrap();
    let second=complete_in(&store,first.session.clone(),"second-result",None).await;let start2=store.start_knowledge_writeback(request(&second,"instance")).await.unwrap();
    store.settle_knowledge_writeback(&owner(),&second.session.agent_session_id,&start2.start_event_id,outcome(&second,Status::NoCandidate)).await.unwrap();
    store.settle_knowledge_writeback(&owner(),&first.session.agent_session_id,&start1.start_event_id,outcome(&first,Status::Partial)).await.unwrap();
    assert_eq!(summary(&store,&first).await.projection["knowledge_writeback"]["status"],"partial");assert_eq!(summary(&store,&second).await.projection["knowledge_writeback"]["status"],"no_candidate");
    let third=complete_in(&store,first.session.clone(),"delete-result",None).await;let start3=store.start_knowledge_writeback(request(&third,"instance")).await.unwrap();
    store.fence_delete(&DeleteAgentSessionCommand{operation_id:"delete-result".into(),agent_session_id:third.session.agent_session_id.clone(),owner_ref:owner(),requested_at:1_900_000_000_000}).await.unwrap();
    store.settle_knowledge_writeback(&owner(),&third.session.agent_session_id,&start3.start_event_id,outcome(&third,Status::Cancelled)).await.unwrap();
    let status:String=sqlx::query_scalar("SELECT json_extract(inline_json,'$.status') FROM agent_events WHERE session_id=? AND kind='turn/knowledge-writeback-settled' AND correlation_id=?")
        .bind(third.session.agent_session_id.as_ref()).bind(third.operation.as_ref()).fetch_one(store.test_pool()).await.unwrap();assert_eq!(status,"cancelled");
}

#[tokio::test]
async fn knowledge_writeback_legacy_terminal_success_is_read_only_and_never_invents_pending() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"legacy-notice").await;
    let f=complete_in(&store,session,"legacy-notice",Some(json!({"status":"written","written":[{"kb_id":Uuid::now_v7().to_string(),"rel_path":"old/actual.md"}]}))).await;
    let before=store.current_cursor(&f.session.agent_session_id).await.unwrap();let summary=summary(&store,&f).await;
    assert_eq!(summary.projection["knowledge_writeback"]["status"],"written");assert!(summary.projection["knowledge_writeback"].get("event_seq").is_none());
    assert_eq!(store.interrupt_stale_knowledge_writebacks("current-instance").await.unwrap(),0);assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),before);
    let(session,_)=create_ready(&store,"legacy-invalid").await;let f=complete_in(&store,session,"legacy-invalid",Some(json!({"status":"running","written":[]}))).await;
    assert!(store.turn_history_for_operation(&owner(),&f.session.agent_session_id,&f.operation).await.is_err());
}

#[tokio::test]
async fn knowledge_writeback_observations_do_not_extend_the_successor_native_checkpoint_or_history() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=completed(&store,"checkpoint-original").await;
    let start=store.start_knowledge_writeback(request(&f,"process-a")).await.unwrap();
    let next:OperationId="checkpoint-next-turn".into();
    let route=ChatRouteIdentity::new("coding.codex@1",nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,"next-route".into(),4);
    let (_,turn)=store.start_turn(&f.session.agent_session_id,"session_api".into(),"checkpoint-next".into(),next.clone(),
        StrictJsonValue(json!({"content":"next answer","admission":{"resolved_snapshot_ref":snapshot_ref(),"route_identity":route}}))).await.unwrap();
    let lease=store.claim_native_execution(crate::NativeExecutionClaim{owner:owner(),agent_session_id:f.session.agent_session_id.clone(),operation_id:next.clone(),
        snapshot:snapshot_ref(),active_set_generation:0,holder:"checkpoint-current-process".into(),expected_fence:0,checkpoint:None}).await.unwrap();
    let state=StrictJsonValue(json!({"version":1,"model_steps":0}));let digest=nomifun_agent_contracts::digest_bytes(&canonical_json_bytes(&state.0).unwrap());
    let progress=append(&f.session.agent_session_id,"knowledge-successor-checkpoint","runtime_supervisor","knowledge-successor-checkpoint","runtime/progress-recorded",next.as_ref(),Some(turn.ack.unwrap().event_id),
        json!({"producer_seq":1,"event":{"event":"execution_checkpoint_saved","step":0,"revision":1,"digest":digest}}));
    let checkpoint=store.save_native_checkpoint(&progress,crate::NativeCheckpointWrite{owner:owner(),operation_id:next.clone(),snapshot:snapshot_ref(),active_set_generation:0,
        expected_revision:0,execution_fence:lease.fence(),lease:Some(lease.clone()),state}).await.unwrap();
    let bytes=store.native_payload_bytes(&lease).await.unwrap();
    store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,outcome(&f,Status::Written)).await.unwrap();
    assert_eq!(store.native_payload_bytes(&lease).await.unwrap(),bytes);
    assert_eq!(store.load_native_checkpoint(&owner(),&f.session.agent_session_id,&next).await.unwrap(),Some(checkpoint));
    assert!(!store.native_recovery_facts(&f.session.agent_session_id,&next).await.unwrap().events.iter()
        .any(|event|matches!(event.kind.0.as_str(),TURN_KNOWLEDGE_WRITEBACK_STARTED|TURN_KNOWLEDGE_WRITEBACK_SETTLED)));
    store.verify_native_execution(&lease).await.unwrap();
}

#[tokio::test]
async fn knowledge_writeback_cannot_begin_for_running_cancelled_or_non_human_roots() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let(session,_)=create_ready(&store,"non-human").await;
    let operation:OperationId="non-human-turn".into();
    let(input,started)=store.start_turn(&session.agent_session_id,"session_api".into(),"non-human-input".into(),operation.clone(),
        StrictJsonValue(json!({"content":"scheduled input","origin":"cron","admission":{"resolved_snapshot_ref":snapshot_ref()}}))).await.unwrap();
    let mut f=Completed{session,operation,root:input.ack.unwrap().event_id,terminal:started.ack.unwrap().event_id};
    assert!(store.start_knowledge_writeback(request(&f,"process-a")).await.is_err(),"a running Turn has no completed-answer permission");
    let terminal=append(&f.session.agent_session_id,"non-human-completed","runtime_supervisor","non-human-completed","turn/completed",f.operation.as_ref(),Some(f.terminal.clone()),json!({}));
    f.terminal=store.append_event(&terminal).await.unwrap().ack.unwrap().event_id;
    assert!(store.start_knowledge_writeback(request(&f,"process-a")).await.is_err(),"scheduled input cannot become owner Knowledge through this policy");
    let(session,_)=create_ready(&store,"cancelled-root").await;
    let(input,started)=store.start_turn(&session.agent_session_id,"session_api".into(),"cancelled-root-input".into(),"cancelled-root-turn".into(),
        StrictJsonValue(json!({"content":"stop this answer","admission":{"resolved_snapshot_ref":snapshot_ref()}}))).await.unwrap();
    let f=Completed{session,operation:"cancelled-root-turn".into(),root:input.ack.unwrap().event_id,terminal:started.ack.unwrap().event_id};
    store.cancel_active_turn(&f.session.agent_session_id,"cancelled-root-request".into(),"session_api".into()).await.unwrap();
    assert!(store.start_knowledge_writeback(request(&f,"process-a")).await.is_err());
}

#[tokio::test]
async fn knowledge_writeback_cancelled_publication_settles_actual_files_without_leaving_pending_cleanup() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=completed(&store,"cancelled-publication").await;
    let start=store.start_knowledge_writeback(request(&f,"process-a")).await.unwrap();
    let mut stopped=outcome(&f,Status::Written);stopped.status=Status::Cancelled;
    stopped.detail=Some("Owner publication finished before cancellation cleanup completed".into());
    let files=stopped.written.clone();
    let result=store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,stopped.clone()).await.unwrap();
    assert!(!result.duplicate);
    let settled=summary(&store,&f).await;
    assert_eq!(settled.projection["knowledge_writeback"]["status"],"cancelled");
    assert_eq!(settled.projection["knowledge_writeback"]["written"],serde_json::to_value(files).unwrap());
    assert!(store.settle_knowledge_writeback(&owner(),&f.session.agent_session_id,&start.start_event_id,stopped).await.unwrap().duplicate,
        "cleanup retries the same owner result without publishing again");
    assert_eq!(store.interrupt_stale_knowledge_writebacks("next-process").await.unwrap(),0,"completed cancellation must not remain pending");
    assert!(!store.has_unsettled_effects(&f.session.agent_session_id).await.unwrap());
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&f.operation).await.unwrap().status,TurnReceiptStatus::Completed);
    let g=completed(&store,"interrupted-publication").await;let start=store.start_knowledge_writeback(request(&g,"process-a")).await.unwrap();
    let mut interrupted=outcome(&g,Status::Written);interrupted.status=Status::Interrupted;
    store.settle_knowledge_writeback(&owner(),&g.session.agent_session_id,&start.start_event_id,interrupted).await.unwrap();
    assert_eq!(summary(&store,&g).await.projection["knowledge_writeback"]["written"].as_array().unwrap().len(),1);
}

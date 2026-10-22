use super::*;
use nomifun_db::{AgentExecutionSourceAppend, AppendAgentExecutionStepsParams};

async fn source_turn(db:&nomifun_db::Database,session:&str,operation:&str) {
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running',?,0,0,0) ON CONFLICT(session_id) DO UPDATE SET status='running',active_turn_id=excluded.active_turn_id")
        .bind(session).bind(operation).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,accepted_at,started_at,execution_owner,execution_generation) VALUES(?,?,?,?,'running',1,1,'fixture-native',1)")
        .bind(session).bind(operation).bind(operation).bind(operation).execute(db.pool()).await.unwrap();
}

pub(super) async fn fixture(titles:&[&str])->(scoped_stop::Fixture,String,CreateAgentExecutionParams) {
    let db=database().await;
    let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    let session=nomifun_common::generate_id();
    insert_canonical_session(&db,&session,OWNER_ID,"live").await;
    source_turn(&db,&session,"source-turn").await;
    let command=super::delegation_creation::delegated_params(&session,"source-turn","initial");
    let participant_id=nomifun_common::generate_id();
    let mut selected=participant(participant_id.clone());selected.constraints=Some("{\"max_concurrency\":3}".into());
    let created=repository.create_execution_with_participants(OWNER_ID,&command,&[selected],
        &event(AgentExecutionEventKind::Created)).await.unwrap();
    repository.reconcile_plan(OWNER_ID,&created.execution_id,created.version,&ReconcileAgentExecutionPlanParams {
        goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,keep_step_ids:vec![],
        new_participants:vec![],retire_participant_ids:vec![],new_steps:titles.iter().map(|title|step(nomifun_common::generate_id(),Some(participant_id.clone()),title)).collect(),
        new_dependencies:vec![],execution_status:AgentExecutionStatus::Running,
    },&event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
    (scoped_stop::Fixture {db,repository,owner:OWNER_ID.into(),execution_id:created.execution_id},session,command)
}

fn append_event(session:&str,attempt:Option<String>)->NewAgentExecutionEvent {
    NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(session,attempt),..event(AgentExecutionEventKind::PlanChanged)}
}

pub(super) async fn message_source(fixture:&scoped_stop::Fixture,sender:&str,turn:&str,original:&str,key:&str,content:&str)
    ->(nomifun_db::AgentExecutionMessageSource,String) {
    let (mut start,fence,generation):(Option<String>,i64,i64)=sqlx::query_as("SELECT started_event_id,execution_fence,execution_generation FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(sender).bind(turn).fetch_one(fixture.db.pool()).await.unwrap();
    let mut seq:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=?").bind(sender).fetch_one(fixture.db.pool()).await.unwrap();
    if start.is_none() {
        seq+=1;let id=nomifun_common::generate_id();
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'session_api',?,'turn/started',1,?,'{}')")
            .bind(sender).bind(seq).bind(&id).bind(&id).bind(turn).execute(fixture.db.pool()).await.unwrap();
        sqlx::query("UPDATE agent_turns SET started_event_id=? WHERE session_id=? AND operation_id=?").bind(&id).bind(sender).bind(turn).execute(fixture.db.pool()).await.unwrap();start=Some(id);
    }
    seq+=1;let id=nomifun_common::generate_id();
    let value=serde_json::json!({"producer_seq":seq,"event":{"event":"tool_call_completed","step":1,
        "call":{"call_id":format!("message-{key}"),"name":"agent_message","arguments":{
            "execution_id":fixture.execution_id,"step_id":original,"content":content,"idempotency_key":key}}}});
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,?,?,'runtime_supervisor',?,'runtime/progress-recorded',1,?,?,?)")
        .bind(sender).bind(seq).bind(&id).bind(&id).bind(turn).bind(start).bind(value.to_string()).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET last_seq=? WHERE session_id=?").bind(seq).bind(sender).execute(fixture.db.pool()).await.unwrap();
    let identity=serde_json::to_vec(&(sender,turn,&fixture.execution_id,original,key)).unwrap();
    (nomifun_db::AgentExecutionMessageSource {command_event_id:id,agent_session_id:sender.into(),turn_operation_id:turn.into(),native_fence:fence as u64,native_generation:generation as u64},
        format!("agent-message:{}",nomifun_agent_contracts::digest_bytes(&identity).as_ref()))
}

async fn followup(fixture:&scoped_stop::Fixture,sender:&str,turn:&str,original:&str,key:&str,content:&str)->AgentExecutionSourceAppend {
    let (message_source,operation_id)=message_source(fixture,sender,turn,original,key,content).await;
    AgentExecutionSourceAppend::ChildFollowup {message_source,original_step_id:original.into(),operation_id,
        content_digest:nomifun_agent_contracts::digest_bytes(content.as_bytes()).0}
}

#[tokio::test]
async fn source_batches_append_once_and_keep_each_steps_exact_original_source() {
    let (fixture,session,original)=fixture(&["first batch"]).await;
    let first=fixture.task("first batch").await;
    let initial=fixture.repository.step_source(&fixture.owner,&fixture.execution_id,&first.step.step_id).await.unwrap().unwrap();
    assert_eq!(initial.tool_operation_id,"tool-initial");
    let mut command=super::delegation_creation::delegated_params(&session,"source-turn","second");
    command.goal="another independent batch".into();
    command.max_parallel=8;
    let source=AgentExecutionSourceAppend::Delegation {command:command.clone()};
    let version=fixture.detail().await.execution.version;
    let params=AppendAgentExecutionStepsParams {new_steps:vec![step(nomifun_common::generate_id(),first.step.assigned_participant_id.clone(),"second batch")],new_dependencies:vec![]};
    let other=AppendAgentExecutionStepsParams {new_steps:vec![step(nomifun_common::generate_id(),first.step.assigned_participant_id.clone(),"second batch")],new_dependencies:vec![]};
    let event=append_event(&session,None);
    let (one,two)=tokio::join!(fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,version,&source,&params,&event),
        fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,version,&source,&other,&event));
    let one=one.unwrap();let two=two.unwrap();
    assert_eq!(one.added_step_ids,two.added_step_ids);
    assert_eq!(one.detail.steps.len(),2);
    assert_eq!(one.detail.execution.goal,original.goal);
    assert_eq!(one.detail.execution.max_parallel,original.max_parallel,"new work cannot rewrite the user's concurrency settings");
    let source=fixture.repository.step_source(&fixture.owner,&fixture.execution_id,&one.added_step_ids[0]).await.unwrap().unwrap();
    assert_eq!(source.tool_operation_id,"tool-second","a later batch never borrows the first tool's context cutoff");
    assert_eq!(fixture.repository.find_source_command(&fixture.owner,&command).await.unwrap().unwrap().execution_id,fixture.execution_id);
    let version=fixture.detail().await.execution.version;
    let manual=nomifun_common::generate_id();
    fixture.repository.append_steps(&fixture.owner,&fixture.execution_id,version,&AppendAgentExecutionStepsParams {
        new_steps:vec![step(manual.clone(),first.step.assigned_participant_id.clone(),"manual work")],new_dependencies:vec![],
    },&event).await.unwrap();
    assert!(fixture.repository.step_source(&fixture.owner,&fixture.execution_id,&manual).await.unwrap().is_none(),
        "unrelated later work cannot inherit the initial delegation's accepted input");
    let before=fixture.detail().await;
    let terminal=nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'runtime_supervisor',?,'turn/cancelled',1,'source-turn','{}')")
        .bind(&session).bind(&terminal).bind(&terminal).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_turns SET state='cancelled',terminal_event_id=?,finished_at=2 WHERE session_id=?")
        .bind(terminal).bind(&session).execute(fixture.db.pool()).await.unwrap();
    command.goal="stopped source cannot append".into();
    assert!(fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,
        &AgentExecutionSourceAppend::Delegation {command},&other,&event).await.is_err());
    assert_eq!(fixture.detail().await.execution.version,before.execution.version);
}

#[tokio::test]
async fn completed_child_followups_keep_its_session_prior_result_fifo_and_stop_ownership() {
    let (fixture,session,_)=fixture(&["original child"]).await;
    let original=fixture.start("original child").await;
    let original_session=original.current_attempt.as_ref().unwrap().conversation_id.clone().unwrap();
    fixture.complete("original child").await;
    let completed=fixture.task("original child").await;
    let content="\ncontinue with a distinct accepted input\n";
    let params=AppendAgentExecutionStepsParams {new_steps:vec![{
        let mut next=step(nomifun_common::generate_id(),completed.step.assigned_participant_id.clone(),"followup");
        next.spec=content.into();next
    }],new_dependencies:vec![]};
    let command=followup(&fixture,&session,"source-turn",&completed.step.step_id,"stable-message",content).await;
    let event=append_event(&session,None);
    let before=fixture.detail().await;
    let appended=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,&command,&params,&event).await.unwrap();
    let next_id=&appended.added_step_ids[0];
    assert_eq!(appended.detail.steps.iter().find(|step|step.step_id==*next_id).unwrap().spec,content);
    assert_eq!(fixture.repository.continuation_target(&fixture.owner,&fixture.execution_id,next_id).await.unwrap().unwrap().conversation_id,original_session);
    assert_eq!(fixture.task("original child").await.step.version,completed.step.version);
    assert_eq!(fixture.task("original child").await.current_attempt.as_ref().unwrap().attempt.output_summary,
        completed.current_attempt.as_ref().unwrap().attempt.output_summary);
    let replay=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,&command,&params,&event).await.unwrap();
    assert_eq!(replay.added_step_ids,appended.added_step_ids);
    assert_eq!(replay.detail.execution.version,appended.detail.execution.version);
    let mut queued=params.clone();queued.new_steps[0].step_id=nomifun_common::generate_id();
    let second=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,appended.detail.execution.version,
        &followup(&fixture,&session,"source-turn",&completed.step.step_id,"next-message",content).await,&queued,&event).await.unwrap();
    assert!(second.detail.dependencies.is_empty(),"Session input order is not an artifact dependency");
    assert!(fixture.repository.step_session_available(&fixture.owner,&fixture.execution_id,next_id).await.unwrap());
    assert!(!fixture.repository.step_session_available(&fixture.owner,&fixture.execution_id,&second.added_step_ids[0]).await.unwrap());
    let second_step=second.detail.steps.iter().find(|step|step.step_id==second.added_step_ids[0]).unwrap();
    let attempt=CreateAgentExecutionAttemptParams {participant_id:second_step.assigned_participant_id.clone(),start_immediately:false,
        trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None};
    assert!(fixture.repository.create_attempt(&fixture.owner,&fixture.execution_id,&second_step.step_id,second_step.version,None,
        &attempt,&super::event(AgentExecutionEventKind::AttemptChanged)).await.is_err(),"the writer enforces the same exact Session FIFO");
    let next=second.detail.steps.iter().find(|step|step.step_id==*next_id).unwrap();
    fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,second.detail.execution.version,
        next_id,next.version,nomifun_common::AgentExecutionCancellationScope::Task,
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&fixture.owner),..super::event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    assert!(fixture.repository.step_session_available(&fixture.owner,&fixture.execution_id,&second.added_step_ids[0]).await.unwrap(),
        "stopping one input preserves the later independent input in the same child Session");
    fixture.repository.create_attempt(&fixture.owner,&fixture.execution_id,&second_step.step_id,second_step.version,None,
        &attempt,&super::event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let stopped=fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,second.detail.execution.version,
        &completed.step.step_id,completed.step.version,nomifun_common::AgentExecutionCancellationScope::DelegatedWork,
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&fixture.owner),..super::event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    assert_eq!(stopped.steps.iter().find(|step|step.step_id==completed.step.step_id).unwrap().status,"completed");
    assert!(stopped.steps.iter().filter(|step|step.step_id!=completed.step.step_id).all(|step|step.status=="cancelled"));
    queued.new_steps[0].step_id=nomifun_common::generate_id();
    assert!(fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,stopped.execution.version,
        &followup(&fixture,&session,"source-turn",&completed.step.step_id,"after-stop",content).await,&queued,&event).await.is_err(),
        "an explicit ownership stop fences future followup derivation without destroying prior results");
}

#[tokio::test]
async fn nested_parent_followup_uses_its_own_committed_source_and_ancestor_stop_scope() {
    let (fixture,_,_)=fixture(&["parent"]).await;
    let parent=fixture.start("parent").await;
    let parent_attempt=parent.current_attempt.as_ref().unwrap();
    let sender=parent_attempt.conversation_id.as_ref().unwrap();
    let sender_turn=format!("turn:user:{}:{sender}:{}:initial-turn",fixture.owner,parent_attempt.attempt.attempt_id);
    source_turn(&fixture.db,sender,&sender_turn).await;
    let child=step(nomifun_common::generate_id(),parent.step.assigned_participant_id.clone(),"nested child");
    let child_id=child.step_id.clone();
    let source=nomifun_common::AgentExecutionSource {agent_session_id:sender.clone(),turn_operation_id:sender_turn.clone(),tool_operation_id:"nested-tool".into(),effect_id:"nested-effect".into()};
    let actor=append_event(sender,Some(parent_attempt.attempt.attempt_id.clone()));
    fixture.repository.append_steps_from_attempt(&fixture.owner,&fixture.execution_id,
        &nomifun_db::AppendAgentExecutionStepsFromAttemptParams {operation_id:"nested-delegation".into(),caller_conversation_id:sender.clone(),
            caller_step_id:parent.step.step_id.clone(),caller_attempt_id:parent_attempt.attempt.attempt_id.clone(),
            expected_caller_step_version:parent.step.version,expected_caller_attempt_version:parent_attempt.attempt.version,
            new_steps:vec![child],new_dependencies:vec![]},
        &NewAgentExecutionEvent {payload:serde_json::json!({"delegation_source":source}).to_string(),..actor.clone()}).await.unwrap();
    fixture.start("nested child").await;fixture.complete("nested child").await;
    let content="continue the nested child";
    let mut next=step(nomifun_common::generate_id(),parent.step.assigned_participant_id.clone(),"nested followup");next.spec=content.into();
    let before=fixture.detail().await;
    let appended=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,
        &followup(&fixture,sender,&sender_turn,&child_id,"nested-message",content).await,
        &AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},&actor).await.unwrap();
    let followup_id=&appended.added_step_ids[0];
    let events=fixture.repository.list_events(&fixture.owner,&fixture.execution_id,0,100).await.unwrap();
    let committed=events.last().unwrap();
    assert_eq!(committed.step_id.as_deref(),Some(child_id.as_str()),"the command's target remains the child");
    assert_eq!(committed.actor_attempt_id.as_deref(),Some(parent_attempt.attempt.attempt_id.as_str()),"the actor remains the exact parent Attempt");
    assert_ne!(committed.attempt_id,committed.actor_attempt_id,"target Attempt and caller Attempt are independent facts");
    assert_eq!(fixture.repository.step_source(&fixture.owner,&fixture.execution_id,followup_id).await.unwrap(),Some(source));
    let row=fixture.repository.get_step(&fixture.owner,&fixture.execution_id,followup_id).await.unwrap().unwrap();
    assert_eq!(row.delegation_depth,1,"continuation cannot reset a nested child's recursion budget");
    let current=fixture.task("parent").await;
    let stopped=fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,appended.detail.execution.version,
        &current.step.step_id,current.step.version,nomifun_common::AgentExecutionCancellationScope::DelegatedWork,
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&fixture.owner),..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    assert_eq!(stopped.steps.iter().find(|step|step.step_id==parent.step.step_id).unwrap().status,"running");
    assert_eq!(stopped.steps.iter().find(|step|step.step_id==child_id).unwrap().status,"completed");
    assert_eq!(stopped.steps.iter().find(|step|step.step_id==*followup_id).unwrap().status,"cancelled");
}

#[tokio::test]
async fn followup_requires_the_exact_native_message_arguments_and_allows_its_original_ref_after_resume() {
    for mutation in ["valid","resumed","execution_id","step_id","content","idempotency_key","producer","name","proposal_shape","reference"] {
        let (fixture,session,_)=fixture(&["original child"]).await;
        let original=fixture.start("original child").await;fixture.complete("original child").await;
        let content="\nthis exact input must remain byte-for-byte\n";
        let mut command=followup(&fixture,&session,"source-turn",&original.step.step_id,"native-identity",content).await;
        let AgentExecutionSourceAppend::ChildFollowup {message_source,..}=&mut command else {unreachable!()};
        match mutation {
            "resumed"=>{
                sqlx::query("UPDATE agent_turns SET execution_fence=1,execution_generation=3 WHERE session_id=? AND operation_id='source-turn'")
                    .bind(&session).execute(fixture.db.pool()).await.unwrap();
                message_source.native_fence=1;message_source.native_generation=3;
            },
            "execution_id"|"step_id"|"content"|"idempotency_key"=>{
                let path=format!("$.event.call.arguments.{mutation}");
                sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,?,'tampered') WHERE session_id=? AND event_id=?")
                    .bind(path).bind(&session).bind(&message_source.command_event_id).execute(fixture.db.pool()).await.unwrap();
            },
            "producer"=>{sqlx::query("UPDATE agent_events SET producer_id='session_api' WHERE session_id=? AND event_id=?")
                .bind(&session).bind(&message_source.command_event_id).execute(fixture.db.pool()).await.unwrap();},
            "name"=>{sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.event.call.name','write_file') WHERE session_id=? AND event_id=?")
                .bind(&session).bind(&message_source.command_event_id).execute(fixture.db.pool()).await.unwrap();},
            "proposal_shape"=>{sqlx::query("UPDATE agent_events SET inline_json=json_remove(inline_json,'$.event.step') WHERE session_id=? AND event_id=?")
                .bind(&session).bind(&message_source.command_event_id).execute(fixture.db.pool()).await.unwrap();},
            "reference"=>{message_source.command_event_id=nomifun_common::generate_id();},
            _=>{},
        }
        let mut next=step(nomifun_common::generate_id(),original.step.assigned_participant_id.clone(),"followup");next.spec=content.into();
        let before=fixture.detail().await;
        let result=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,&command,
            &AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},&append_event(&session,None)).await;
        if matches!(mutation,"valid"|"resumed") {assert!(result.is_ok(),"{mutation}: {result:?}");}
        else {assert!(result.is_err(),"{mutation} cannot grant a new child input");assert_eq!(fixture.detail().await.execution.version,before.execution.version);}
    }
}

#[tokio::test]
async fn ordinary_and_autowork_containers_continue_only_formally_delegated_children() {
    for automation in [false,true] {
        for delegated in [false,true] {
            let (fixture,lead)=if automation {
                let db=database().await;let lead=nomifun_common::generate_id();
                insert_canonical_session(&db,&lead,OWNER_ID,"live").await;
                sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_set_generation,last_seq,unread_count) VALUES(?,'ready',0,0,0)")
                    .bind(&lead).execute(db.pool()).await.unwrap();
                let claim=super::claimed_autowork_requirement(&db,&lead).await;
                let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
                let participant_id=nomifun_common::generate_id();let mut selected=participant(participant_id.clone());selected.constraints=Some("{\"max_concurrency\":3}".into());
                let mut params=execution_params();params.lead_conversation_id=Some(lead.clone());
                params.initial_plan_input=serde_json::json!({"mode":"automation","source":{"requirement_id":claim.requirement_id,
                    "claim_generation":claim.claim_generation,"operation_id":"autowork:container"},"plan":{"steps":[]}}).to_string();
                let created=repository.create_execution_with_participants(OWNER_ID,&params,&[selected],&event(AgentExecutionEventKind::Created)).await.unwrap();
                repository.reconcile_plan(OWNER_ID,&created.execution_id,created.version,&ReconcileAgentExecutionPlanParams {
                    goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,keep_step_ids:vec![],new_participants:vec![],retire_participant_ids:vec![],
                    new_steps:vec![step(nomifun_common::generate_id(),Some(participant_id),"parent")],new_dependencies:vec![],execution_status:AgentExecutionStatus::Running,
                },&event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
                (scoped_stop::Fixture {db,repository,owner:OWNER_ID.into(),execution_id:created.execution_id},Some(lead))
            } else {(scoped_stop::Fixture::memory(&["parent"],&[]).await,None)};
            let parent=if let Some(lead)=lead.as_ref() {
                let task=fixture.task("parent").await;
                let queued=fixture.repository.create_attempt(&fixture.owner,&fixture.execution_id,&task.step.step_id,task.step.version,None,
                    &CreateAgentExecutionAttemptParams {participant_id:task.step.assigned_participant_id.clone(),start_immediately:false,trigger_reason:"initial".into(),
                        effective_config:"{\"session_kind\":\"automation\"}".into(),retry_after:None,runtime_state:None},&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
                let attempt=queued.current_attempt.as_ref().unwrap();
                fixture.repository.start_attempt(&fixture.owner,&fixture.execution_id,&queued.step.step_id,queued.step.version,
                    &attempt.attempt.attempt_id,attempt.attempt.version,lead,AgentExecutionAttemptSessionKind::AutomationLead,None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap()
            } else {fixture.start("parent").await};
            let caller=parent.current_attempt.as_ref().unwrap();let sender=caller.conversation_id.as_ref().unwrap();
            let sender_turn=format!("turn:user:{}:{sender}:{}:initial-turn",fixture.owner,caller.attempt.attempt_id);
            source_turn(&fixture.db,sender,&sender_turn).await;
            let child_policy=parent.step.tool_policy.parse().unwrap();
            let mut child=step(nomifun_common::generate_id(),parent.step.assigned_participant_id.clone(),"child");child.tool_policy=child_policy;let child_id=child.step_id.clone();
            let source=nomifun_common::AgentExecutionSource {agent_session_id:sender.clone(),turn_operation_id:sender_turn.clone(),tool_operation_id:"container-tool".into(),effect_id:"container-effect".into()};
            let actor=append_event(sender,Some(caller.attempt.attempt_id.clone()));
            fixture.repository.append_steps_from_attempt(&fixture.owner,&fixture.execution_id,&nomifun_db::AppendAgentExecutionStepsFromAttemptParams {
                operation_id:"container-delegation".into(),caller_conversation_id:sender.clone(),caller_step_id:parent.step.step_id.clone(),caller_attempt_id:caller.attempt.attempt_id.clone(),
                expected_caller_step_version:parent.step.version,expected_caller_attempt_version:caller.attempt.version,new_steps:vec![child],new_dependencies:vec![],
            },&NewAgentExecutionEvent {payload:if delegated {serde_json::json!({"delegation_source":source}).to_string()} else {"{}".into()},..actor.clone()}).await.unwrap();
            fixture.start("child").await;fixture.complete("child").await;
            let content="continue this exact delegated child";let mut next=step(nomifun_common::generate_id(),parent.step.assigned_participant_id.clone(),"followup");next.spec=content.into();next.tool_policy=child_policy;
            let command=followup(&fixture,sender,&sender_turn,&child_id,"container-message",content).await;
            let before=fixture.detail().await;
            let result=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,&command,
                &AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},&actor).await;
            if delegated {
                let result=result.unwrap();assert_eq!(fixture.repository.step_source(&fixture.owner,&fixture.execution_id,&result.added_step_ids[0]).await.unwrap(),Some(source));
                if automation {
                    sqlx::query("UPDATE requirements SET status='cancelled' WHERE owner_conversation_id=?").bind(sender).execute(fixture.db.pool()).await.unwrap();
                    let stopped=fixture.detail().await;
                    let mut next=step(nomifun_common::generate_id(),parent.step.assigned_participant_id.clone(),"after claim Stop");next.spec=content.into();next.tool_policy=child_policy;
                    let command=followup(&fixture,sender,&sender_turn,&child_id,"after-claim-stop",content).await;
                    assert!(fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,stopped.execution.version,&command,
                        &AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},&actor).await.is_err(),
                        "Requirement Stop fences a late native MESSAGE even before the head/epoch closes");
                    assert_eq!(fixture.detail().await.execution.version,stopped.execution.version);
                }
            }
            else {assert!(result.is_err(),"container {automation} cannot manufacture a missing Step source");assert_eq!(fixture.detail().await.execution.version,before.execution.version);}
        }
    }
}

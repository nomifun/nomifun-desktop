//! Real Step Plan through the production Session/Runtime/Execution graph.
//! Only this run's disposable root, provider and files are used.
use super::*;
use nomifun_db::sqlx;
use std::path::PathBuf;
use nomifun_agent_runtime::AgentEngineEvent;

const MARKER:&str="NOMIFUN_SUBAGENT_INTEGRATION_OK";
const INTEGRATED:&str="LEFT=17;RIGHT=29;SUM=46";

fn failed(code:&'static str)->SmokeFailure { SmokeFailure::new("subagent.integration",code,422) }

pub(super) fn final_result_evidence(children:usize,text:Option<&str>,marker:&str)->(bool,bool) {
    let nonempty=text.is_some_and(|text|!text.trim().is_empty());
    let marked=text.is_some_and(|text|text.contains(marker));
    // Only bounded structural evidence leaves the fixture: never model prose,
    // inputs, tool arguments, credential material, or arbitrary error text.
    eprintln!("NOMIFUN_SUBAGENT_FINAL_RESULT children={children} text_present={} text_nonempty={nonempty} text_bytes={} marker_present={marked}",text.is_some(),text.map(str::len).unwrap_or(0));
    (nonempty,marked)
}

#[derive(Debug)]
pub(super) struct WaitEvidence {pub wait_calls:usize,pub status_calls:usize,pub waits:usize,pub wakes:usize,pub parent_model_steps:usize}

/// Measure real canonical model dispatch boundaries during an outstanding
/// child wait. This does not infer provider calls from UI state or elapsed time.
pub(super) fn wait_evidence(events:&[AgentEngineEvent])->Result<WaitEvidence,SmokeFailure> {
    let mut proof=WaitEvidence {wait_calls:0,status_calls:0,waits:0,wakes:0,parent_model_steps:0};let mut parked=false;
    for event in events {
        match event {
            AgentEngineEvent::ToolCallCompleted {call,..} if call.name=="agent_wait"=>proof.wait_calls+=1,
            AgentEngineEvent::ToolCallCompleted {call,..} if call.name=="agent_status"=>proof.status_calls+=1,
            AgentEngineEvent::AwaitingChildren {..}=>{if parked {return Err(failed("SUBAGENT_NESTED_WAIT"));}parked=true;proof.waits+=1;},
            AgentEngineEvent::ChildrenWaitResolved {..}=>{if !parked {return Err(failed("SUBAGENT_WAIT_WITHOUT_OWNER"));}parked=false;proof.wakes+=1;},
            AgentEngineEvent::ModelStepStarted {..}=>{if parked {return Err(failed("SUBAGENT_MODEL_POLLED_DURING_WAIT"));}proof.parent_model_steps+=1;},
            _=>{},
        }
    }
    if parked||proof.waits==0||proof.waits!=proof.wakes||proof.wait_calls==0||proof.status_calls==0 {
        return Err(failed("SUBAGENT_WAIT_NOT_FULLY_JOINED"));
    }
    Ok(proof)
}

fn verification_command()->(String,Value) {
    if cfg!(windows) {
        (PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe").to_string_lossy().into_owned(),
            json!(["-NoProfile","-NonInteractive","-Command",format!("if ((Get-Content -LiteralPath 'integrated.txt' -Raw) -ceq '{INTEGRATED}') {{ exit 0 }} else {{ exit 1 }}")]))
    } else {
        ("/bin/sh".into(),json!(["-c",format!("test \"$(cat integrated.txt)\" = \"{INTEGRATED}\"")]))
    }
}

pub(super) fn successful_calls(facts:&nomifun_agent_session::ChatCausalityFacts,operation:&nomifun_agent_contracts::OperationId,
    actions:&[&str])->Vec<(Value,Value)> {
    let Some(turn)=facts.events.iter().find(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==operation.as_ref())
        else {return vec![];};
    facts.events.iter().filter(|event|event.kind.0=="tool/call-started"&&event.causation_event_id.as_ref()==Some(&turn.event_id))
        .filter_map(|start|Some((start,facts.event_payloads.get(start.event_id.as_ref())?)))
        .filter(|(_,input)|input.get("action_id").and_then(Value::as_str).is_some_and(|action|actions.contains(&action)))
        .filter_map(|(start,input)|{
            let call=input.get("call_id")?.as_str()?;
            let arguments=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.correlation_id.as_ref()==operation.as_ref())
                .filter_map(|event|facts.event_payloads.get(event.event_id.as_ref()))
                .find(|payload|payload.pointer("/event/event").and_then(Value::as_str)==Some("tool_call_completed")
                    &&payload.pointer("/event/call/call_id").and_then(Value::as_str)==Some(call))?
                .pointer("/event/call/arguments")?.clone();
            let output=facts.events.iter().filter(|event|event.kind.0=="tool/result-recorded"&&event.causation_event_id.as_ref()==Some(&start.event_id))
                .filter_map(|event|facts.event_payloads.get(event.event_id.as_ref()))
                .find(|payload|payload.get("operation_id")==input.get("operation_id")&&payload["error"].is_null())?;
            let result=serde_json::from_value::<nomifun_agent_runtime::AgentToolResult>(output["output"].clone()).ok()?;
            if result.is_error||result.call_id.as_ref()!=call {return None;}
            Some((arguments,serde_json::from_str::<Value>(&result.output_text()).ok()?))
        }).collect()
}

pub(super) async fn run(router:&Router,key:&str,model:&str,root:&Path,work:&Path)->Result<(),SmokeFailure> {
    if model!="step-5-preview" {return Err(failed("SUBAGENT_MODEL_INVALID"));}
    let provider=configure_stepfun(router,key,STEPFUN_PLAN_BASE_URL,model).await?;
    let (preset,_)=create_agent_preset(router,&provider,model,&[
        ("agent.collaboration",&["agent/delegate"],""),
        ("workspace.files",&["workspace.files/read","workspace.files/write"],"workspace"),
        ("workspace.process",&["workspace.process/start","workspace.process/poll","workspace.process/cancel"],"process_session"),
    ]).await?;
    let (session,_)=create_session_in_workspace(router,&preset,&provider,model,json!([
        {"resource_kind":"workspace","resource_id":"default-workspace"},
        {"resource_kind":"process_session","resource_id":"managed-process-session"}
    ]),Some(work)).await?;
    let (verify_program,verify_arguments)=verification_command();
    let command=format!("Use start_process command={}, args={}, wait_ms=30000.",json!(verify_program),verify_arguments);
    let prompt=format!("Use two independent subagents to execute this development acceptance task in one agent/delegate strategy=parallel call. Both tasks MUST use tool_policy=full. Task name left: begin its prompt with the exact role label 你是 SubAgent-01，负责设计、实现和运行左侧模块。 Then use write_file to save left.txt with exactly LEFT=17, read it back, then report the actual saved file. Task name right: begin its prompt with the exact role label 你是 SubAgent-02，负责设计、实现和运行右侧模块。 Then use write_file to save right.txt with exactly RIGHT=29, read it back, then report the actual saved file. Each child may only modify its own file and must not delegate. Do not write those two files yourself. After the receipt, continue in this SAME parent Turn: use agent_wait then agent_status to consume both exact results. Read both files, compute their sum, and write integrated.txt with exactly {INTEGRATED}, without a trailing newline. Read integrated.txt and run the following real verification command in the admitted workspace. {command} If the process is still running use its exact process_id to poll and prove exit_code=0 and cleanup.reaped=true. Finish your normal plan/completion report only after both child deliveries and integration checks. End the final parent answer with {MARKER}. Do not claim success on acceptance and do not request another user input.");
    let pool=sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(
        sqlx::sqlite::SqliteConnectOptions::new().filename(root.join("data/nomifun-backend.db")).read_only(true))
        .await.map_err(|_|failed("SUBAGENT_FACT_READER_FAILED"))?;
    start_session_turn(router,"subagent.turn",&session,&uuid::Uuid::now_v7().to_string(),prompt).await?;
    let deadline=tokio::time::Instant::now()+Duration::from_secs(480);
    let operation=loop {
        let turn:Option<(String,String)>=sqlx::query_as("SELECT operation_id,state FROM agent_turns WHERE session_id=? ORDER BY rowid DESC LIMIT 1")
            .bind(&session).fetch_optional(&pool).await.map_err(|_|failed("SUBAGENT_PARENT_READ_FAILED"))?;
        if let Some((operation,state))=turn {
            if state=="completed" {break operation;}
            if matches!(state.as_str(),"failed"|"cancelled"|"interrupted") {return Err(failed("SUBAGENT_PARENT_DID_NOT_COMPLETE"));}
        }
        if tokio::time::Instant::now()>=deadline {return Err(failed("SUBAGENT_PARENT_DEADLINE"));}
        tokio::time::sleep(POLL_INTERVAL).await;
    };
    let store=nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await.map_err(|_|failed("SUBAGENT_FACT_READER_FAILED"))?;
    let sid=session.clone().into();let operation=operation.into();
    let facts=store.chat_causality_facts(&sid,&operation).await.map_err(|_|failed("SUBAGENT_PARENT_RESULT_INVALID"))?;
    let build_digest=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"
        && event.correlation_id.as_ref()==operation.as_ref()).filter_map(|event|facts.event_payloads.get(event.event_id.as_ref()))
        .find_map(|payload|payload.pointer("/event/binding/build_digest").and_then(Value::as_str))
        .filter(|digest|digest.len()==64&&digest.bytes().all(|byte|byte.is_ascii_hexdigit()))
        .ok_or_else(||failed("SUBAGENT_NATIVE_BUILD_IDENTITY_MISSING"))?;
    let receipt=store.read_turn_receipt(&sid,&operation).await.map_err(|_|failed("SUBAGENT_PARENT_RESULT_INVALID"))?;
    let result=nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).map_err(|_|failed("SUBAGENT_PARENT_RESULT_INVALID"))?;
    let (final_nonempty,final_marked)=final_result_evidence(2,result.result_text.as_deref(),MARKER);
    for (file,expected) in [("left.txt","LEFT=17"),("right.txt","RIGHT=29"),("integrated.txt",INTEGRATED)] {
        if tokio::fs::read(work.join(file)).await.ok().as_deref()!=Some(expected.as_bytes()) {
            return Err(failed("SUBAGENT_FILE_CONTENT_INVALID"));
        }
    }
    let executions:Vec<String>=sqlx::query_scalar("SELECT execution_id FROM agent_executions WHERE json_extract(initial_plan_input,'$.source.agent_session_id')=?")
        .bind(&session).fetch_all(&pool).await.map_err(|_|failed("SUBAGENT_EXECUTION_READ_FAILED"))?;
    if executions.len()!=1 {return Err(failed("SUBAGENT_DUPLICATED_EXECUTION"));}
    let specifications:Vec<(String,String)>=sqlx::query_as("SELECT title,spec FROM agent_execution_steps WHERE execution_id=? AND superseded_in_revision IS NULL")
        .bind(&executions[0]).fetch_all(&pool).await.map_err(|_|failed("SUBAGENT_SPEC_READ_FAILED"))?;
    if specifications.len()!=2 || specifications.iter().any(|(title,spec)|match title.as_str() {
        "left"=>!spec.contains("你是 SubAgent-01，负责设计、实现和运行左侧模块。"),
        "right"=>!spec.contains("你是 SubAgent-02，负责设计、实现和运行右侧模块。"),
        _=>true,
    }) {return Err(failed("SUBAGENT_ROLE_REGRESSION_NOT_EXERCISED"));}
    let rows:Vec<(String,String,i64,i64,i64)>=sqlx::query_as("SELECT step.tool_policy,attempt.status,attempt.attempt_no,attempt.started_at,attempt.finished_at FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.step_id=attempt.step_id WHERE attempt.execution_id=? ORDER BY attempt.started_at")
        .bind(&executions[0]).fetch_all(&pool).await.map_err(|_|failed("SUBAGENT_ATTEMPTS_READ_FAILED"))?;
    if rows.len()!=2 || rows.iter().any(|row|row.0!="full"||row.1!="completed"||row.2!=0) {
        return Err(failed("SUBAGENT_CHILD_DELIVERY_INVALID"));
    }
    let overlap=rows[0].4.min(rows[1].4)-rows[0].3.max(rows[1].3);
    if overlap<=0 {return Err(failed("SUBAGENT_EXECUTION_DID_NOT_OVERLAP"));}
    let events=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.correlation_id.as_ref()==operation.as_ref())
        .filter_map(|event|facts.event_payloads.get(event.event_id.as_ref())?.get("event"))
        .filter_map(|value|serde_json::from_value::<AgentEngineEvent>(value.clone()).ok()).collect::<Vec<_>>();
    let wait=wait_evidence(&events)?;
    let progress=nomifun_agent_runtime::collaboration_progress_from_events(&events).map_err(|_|failed("SUBAGENT_CONSUMPTION_REPLAY_INVALID"))?;
    let consumed=progress.results.get(&executions[0]).ok_or_else(||failed("SUBAGENT_RESULT_CONSUMPTION_MISSING"))?;
    let expected_steps:Vec<String>=sqlx::query_scalar("SELECT step_id FROM agent_execution_steps WHERE execution_id=? AND superseded_in_revision IS NULL")
        .bind(&executions[0]).fetch_all(&pool).await.map_err(|_|failed("SUBAGENT_STEP_READ_FAILED"))?;
    if !progress.pending_child_executions.is_empty()||!consumed.all_terminal_observed
        ||consumed.source_digests.keys().cloned().collect::<std::collections::BTreeSet<_>>()!=expected_steps.into_iter().collect()
        ||consumed.result_pages.values().any(|page|page.contiguous_bytes!=page.total_bytes) {return Err(failed("SUBAGENT_RESULTS_NOT_FULLY_CONSUMED"));}
    let delegations:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE action_id='agent/delegate'")
        .fetch_one(&pool).await.map_err(|_|failed("SUBAGENT_DELEGATION_READ_FAILED"))?;
    if delegations!=1 {return Err(failed("SUBAGENT_REPEATED_DELEGATION"));}
    let parent_turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(&session).fetch_one(&pool).await.map_err(|_|failed("SUBAGENT_PARENT_READ_FAILED"))?;
    let projections:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='message/assistant-projected'")
        .bind(&session).fetch_one(&pool).await.map_err(|_|failed("SUBAGENT_PARENT_READ_FAILED"))?;
    let writes:Vec<String>=sqlx::query_scalar("SELECT session_id FROM agent_effects WHERE action_id='workspace.files/write' AND state='returned'")
        .fetch_all(&pool).await.map_err(|_|failed("SUBAGENT_EFFECT_READ_FAILED"))?;
    let children:Vec<(String,String)>=sqlx::query_as("SELECT link.conversation_id,step.title FROM conversation_execution_links link JOIN agent_execution_steps step ON step.execution_id=link.execution_id AND step.step_id=link.step_id WHERE link.execution_id=? AND link.relation='attempt'")
        .bind(&executions[0]).fetch_all(&pool).await.map_err(|_|failed("SUBAGENT_CHILD_READ_FAILED"))?;
    if parent_turns!=1 || projections!=0 || children.len()!=2
        || children.iter().any(|(child,_)|!writes.contains(child)) || !writes.contains(&session) {
        return Err(failed("SUBAGENT_OWNERSHIP_OR_PARENT_CONTINUATION_INVALID"));
    }
    let expected_write_sessions=children.iter().map(|(child,_)|child.as_str()).chain(std::iter::once(session.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    if expected_write_sessions.len()!=3 || writes.iter().map(String::as_str).collect::<std::collections::BTreeSet<_>>()!=expected_write_sessions {
        return Err(failed("SUBAGENT_NATIVE_WRITE_OWNERS_INVALID"));
    }
    for (child,title) in &children {
        let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(child).fetch_one(&pool).await.map_err(|_|failed("SUBAGENT_CHILD_TURN_INVALID"))?;
        if turns!=1 {return Err(failed("SUBAGENT_CHILD_DUPLICATE_TURN"));}
        let (file,expected)=match title.as_str() {"left"=>("left.txt","LEFT=17"),"right"=>("right.txt","RIGHT=29"),
            _=>return Err(failed("SUBAGENT_CHILD_TASK_IDENTITY_INVALID"))};
        let child_operation:String=sqlx::query_scalar("SELECT operation_id FROM agent_turns WHERE session_id=? AND state='completed' ORDER BY rowid DESC LIMIT 1")
            .bind(child).fetch_one(&pool).await.map_err(|_|failed("SUBAGENT_CHILD_TURN_INVALID"))?;
        let child_operation=nomifun_agent_contracts::OperationId::from(child_operation);
        let child_facts=store.chat_causality_facts(&child.as_str().into(),&child_operation).await
            .map_err(|_|failed("SUBAGENT_CHILD_FACTS_INVALID"))?;
        if child_facts.event_payloads.values().any(|payload|payload.pointer("/event/rejection/kind").and_then(Value::as_str)==Some("required_delegation")) {
            return Err(failed("SUBAGENT_ROLE_FORCED_REDELEGATION"));
        }
        let calls=successful_calls(&child_facts,&child_operation,&["workspace.files/write"])
            .into_iter().filter(|(_,output)|output["written"]==true).collect::<Vec<_>>();
        if calls.is_empty()||calls.iter().any(|(arguments,_)|{
            let Some(path)=arguments["path"].as_str() else {return true;};
            let path=if Path::new(path).is_absolute(){PathBuf::from(path)}else{work.join(path)};
            nomifun_common::paths::canonicalize_simplified(&path).ok()!=nomifun_common::paths::canonicalize_simplified(&work.join(file)).ok()
        })||calls.last().is_none_or(|(arguments,_)|arguments["content"]!=expected) {
            return Err(failed("SUBAGENT_CHILD_DID_NOT_AUTHOR_ITS_MODULE"));
        }
    }
    let parent_writes=successful_calls(&facts,&operation,&["workspace.files/write"]);
    if parent_writes.iter().any(|(arguments,_)|arguments["path"].as_str().is_some_and(|path|
        matches!(Path::new(path).file_name().and_then(|name|name.to_str()),Some("left.txt"|"right.txt")))) {
        return Err(failed("SUBAGENT_PARENT_REPLACED_CHILD_WORK"));
    }
    let verified:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND action_id='workspace.process/start' AND state='returned'")
        .bind(&session).fetch_one(&pool).await.map_err(|_|failed("SUBAGENT_PROCESS_READ_FAILED"))?;
    if verified==0 {return Err(failed("SUBAGENT_PARENT_DID_NOT_RUN_VERIFICATION"));}
    let verification_processes=successful_calls(&facts,&operation,&["workspace.process/start"]).into_iter()
        .filter(|(arguments,_)|arguments["command"]==verify_program&&arguments["args"]==verify_arguments)
        .filter_map(|(_,output)|output["process_id"].as_str().map(str::to_owned)).collect::<std::collections::BTreeSet<_>>();
    let verified_and_reaped=successful_calls(&facts,&operation,&["workspace.process/start","workspace.process/poll"])
        .iter().any(|(_,output)|output["process_id"].as_str().is_some_and(|id|verification_processes.contains(id))
            &&output["state"]=="exited"&&output["exit_code"]==0&&output.pointer("/cleanup/reaped")==Some(&json!(true)));
    if !verified_and_reaped {return Err(failed("SUBAGENT_VERIFICATION_EXIT_OR_CLEANUP_INVALID"));}
    pool.close().await;
    eprintln!("NOMIFUN_SUBAGENT_FACTS model={model} build_digest={build_digest} full_children=2 role_labeled_children=2 child_required_delegation_rejections=0 overlap_ms={overlap} parent_turns=1 native_write_sessions=3 final_projection=0 integrated=true verification_process=true attempts=2 delegations=1 exact_results_consumed=2 wait_calls={} status_calls={} waits={} wakes={} parent_model_steps={} wait_model_calls=0",wait.wait_calls,wait.status_calls,wait.waits,wait.wakes,wait.parent_model_steps);
    if !final_nonempty {return Err(failed("SUBAGENT_PARENT_FINAL_EMPTY"));}
    if !final_marked {return Err(failed("SUBAGENT_PARENT_FINAL_MARKER_MISSING"));}
    eprintln!("NOMIFUN_SUBAGENT_LIVE_OK model={model} build_digest={build_digest} full_children=2 role_labeled_children=2 child_required_delegation_rejections=0 overlap_ms={overlap} parent_turns=1 native_write_sessions=3 final_projection=0 integrated=true verification_process=true attempts=2 delegations=1 exact_results_consumed=2 wait_calls={} status_calls={} waits={} wakes={} parent_model_steps={} wait_model_calls=0",wait.wait_calls,wait.status_calls,wait.waits,wait.wakes,wait.parent_model_steps);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(value:Value)->AgentEngineEvent {serde_json::from_value(value).unwrap()}
    #[test]
    fn final_result_diagnostics_distinguish_empty_text_from_missing_marker() {
        assert_eq!(final_result_evidence(8,None,MARKER),(false,false));
        assert_eq!(final_result_evidence(8,Some(" \n"),MARKER),(false,false));
        assert_eq!(final_result_evidence(8,Some("A real final delivery without the fixture marker"),MARKER),(true,false));
        assert_eq!(final_result_evidence(8,Some(MARKER),MARKER),(true,true));
    }
    #[test]
    fn child_wait_evidence_rejects_model_polling_and_unfinished_waits() {
        let events=vec![event(json!({"event":"tool_call_completed","step":1,"call":{"call_id":"wait","name":"agent_wait","arguments":{}}})),
            event(json!({"event":"awaiting_children","model_steps":1,"request":{"execution_ids":["original"],"mode":"all"}})),
            event(json!({"event":"children_wait_resolved","model_steps":1,"cursor":"original-result"})),
            event(json!({"event":"tool_call_completed","step":2,"call":{"call_id":"read","name":"agent_status","arguments":{}}})),
            event(json!({"event":"model_step_started","step":3,"operation_id":"model-3"}))];
        assert_eq!(wait_evidence(&events).unwrap().parent_model_steps,1);
        let mut polling=events.clone();polling.insert(2,event(json!({"event":"model_step_started","step":2,"operation_id":"unexpected-model"})));
        assert_eq!(wait_evidence(&polling).unwrap_err().code,"SUBAGENT_MODEL_POLLED_DURING_WAIT");
        assert!(wait_evidence(&events[..2]).is_err());
    }
}

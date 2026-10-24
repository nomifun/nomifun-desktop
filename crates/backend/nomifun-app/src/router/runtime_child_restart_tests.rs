//! A real native parent stays subscribed through its original child's exact
//! technical pause and explicit user restart. Provider responses are local;
//! Session, admission, control, checkpoint and result ownership are production.
use super::*;

const ROOT:&str="RESTART_ROOT: Use two subagents to author independent text modules. Keep managing a paused child while the user restarts that same task, then read both full results and deliver restarted_result+healthy_result.";
const PAUSED:&str="RESTART_CHILD_PAUSE: Author restart-original.txt containing exactly restarted_result. Write this assigned file once and read it back through the native workspace owner. After an explicit native restart, refresh only the original task plan once before completing; never repeat the settled file actions. Do not delegate, change other files or launch a process. Complete the original assignment using the native completion contract.";
const HEALTHY:&str="RESTART_CHILD_HEALTHY: Return exactly healthy_result. Do not delegate or invoke a tool.";

#[derive(Default)]
struct State {
    requests:Mutex<Vec<Value>>,parent_calls:AtomicUsize,child_calls:AtomicUsize,healthy_calls:AtomicUsize,
    setup_calls:AtomicUsize,rejected_reports:AtomicUsize,resumed_calls:AtomicUsize,
    resumed_plans:AtomicUsize,resumed_reports:AtomicUsize,
    resumed:AtomicBool,release_resumed:AtomicBool,wake_resumed:tokio::sync::Notify,
}

fn parent(body:&Value)->(Value,&'static str) {
    if returned(body,"restart-delegate").is_none() {return selected(body,"delegate_task","restart-delegate","agent/delegate",json!({"strategy":"parallel","tasks":[
        {"name":"restart-child","prompt":PAUSED,"tool_policy":"full"},{"name":"healthy-child","prompt":HEALTHY,"tool_policy":"full"}]}));}
    let execution=returned(body,"restart-delegate").unwrap()["execution_id"].as_str().unwrap().to_owned();
    if returned(body,"restart-wait-initial").is_none() {return call("agent_wait","restart-wait-initial",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"restart-status-initial").is_none() {return call("agent_status","restart-status-initial",json!({"execution_ids":[execution]}));}
    let original=returned(body,"restart-status-initial").unwrap();let row=&original["result"]["executions"][0];
    let paused=row["children"].as_array().unwrap().iter().find(|child|child["title"]=="restart-child").unwrap();
    assert!(paused["native_intervention"].is_object(),"the original technical pause must be explicit canonical task data: {paused}");
    assert_eq!(paused["native_intervention"]["boundary"]["reason"],"EXECUTION_INTERVENTION_REQUIRED");
    if returned(body,"restart-wait-again").is_none() {return call("agent_wait","restart-wait-again",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"restart-status-final").is_none() {return call("agent_status","restart-status-final",json!({"execution_ids":[execution]}));}
    let final_status=returned(body,"restart-status-final").unwrap();assert_eq!(final_status["result"]["all_ready"],true);
    let children=final_status["result"]["executions"][0]["children"].as_array().unwrap();assert_eq!(children.len(),2);
    assert!(children.iter().all(|child|child["status"]=="completed"&&child["result_ok"]==true),
        "both original assignments must deliver verified native completion, not only terminal failure");
    assert!(children.iter().any(|child|child["title"]=="restart-child"&&child["result"].as_str().is_some_and(|text|text.contains("restarted_result"))));
    assert!(children.iter().any(|child|child["title"]=="healthy-child"&&child["result"]=="healthy_result"));
    (json!({"role":"assistant","content":"restarted_result+healthy_result"}),"stop")
}

fn child_report(body:&Value,valid:bool,ordinal:usize)->(Value,&'static str) {
    let definition=body["tools"].as_array().unwrap().iter().find(|tool|tool["function"]["name"]=="report_completion")
        .expect("the test rejects actual completion arguments, never an unknown-tool proposal");
    let mut report=json!({"summary":if valid {"restarted_result\n此前 4 次完成报告参数被拒绝。"} else {""},
        "criteria":[{"disposition":"unverified","rationale":"This is the requested authored text module; no external verification was requested."}]});
    for field in ["observed_tool_error_count","observed_command_failure_count"] {
        let schema=&definition["function"]["parameters"]["properties"][field];
        if !schema["const"].is_null() {report[field]=schema["const"].clone();}
        else if let Some(value)=schema["enum"].as_array().and_then(|values|values.first()) {report[field]=value.clone();}
    }
    call("report_completion",&format!("restart-child-report-{ordinal}"),report)
}

async fn server()->(String,Arc<State>,tokio::task::JoinHandle<()>) {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}/v1",listener.local_addr().unwrap());
    let state=Arc::new(State::default());
    let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(|axum::extract::State(state):axum::extract::State<Arc<State>>,axum::Json(body):axum::Json<Value>|async move {
        state.requests.lock().unwrap().push(body.clone());let users=user_text(&body);
        let (delta,reason)=if users.contains("RESTART_CHILD_PAUSE") {
            let ordinal=state.child_calls.fetch_add(1,Ordering::SeqCst);
            if returned_with(&body,"restart-module-write-",|result|result["written"]==true).is_none() {
                assert_eq!(state.rejected_reports.load(Ordering::SeqCst),0,"setup precedes the exact completion-rejection loop");
                state.setup_calls.fetch_add(1,Ordering::SeqCst);
                selected(&body,"write_file",&format!("restart-module-write-{ordinal}"),"workspace.files/write",
                    json!({"path":"restart-original.txt","content":"restarted_result"}))
            } else if returned_with(&body,"restart-module-read-",|result|result["content"]=="restarted_result").is_none() {
                state.setup_calls.fetch_add(1,Ordering::SeqCst);
                selected(&body,"read_file",&format!("restart-module-read-{ordinal}"),"workspace.files/read",json!({"path":"restart-original.txt"}))
            } else if returned(&body,"restart-child-plan").is_none() {
                assert!(available(&body,"update_plan"),"the native task ledger must actually be advertised after the original write and read");
                state.setup_calls.fetch_add(1,Ordering::SeqCst);
                own_plan("restart-child-plan","in_progress","RESTART_CHILD_PAUSE")
            } else if state.rejected_reports.load(Ordering::SeqCst)<4 {
                assert!(available(&body,"report_completion"),"four real argument rejections require the advertised completion contract");
                state.rejected_reports.fetch_add(1,Ordering::SeqCst);child_report(&body,false,ordinal)
            } else {
                assert!(state.resumed_calls.fetch_add(1,Ordering::SeqCst)<2,"resume permits one required plan refresh and one valid report, never model polling");
                state.resumed.store(true,Ordering::SeqCst);
                loop {let wake=state.wake_resumed.notified();tokio::pin!(wake);wake.as_mut().enable();if state.release_resumed.load(Ordering::SeqCst){break;}wake.await;}
                if returned(&body,"restart-child-resume-plan").is_none() {
                    assert!(available(&body,"update_plan"),"fresh scoped instructions after native attachment retain the required plan control");
                    assert_eq!(state.resumed_plans.fetch_add(1,Ordering::SeqCst),0);
                    own_plan("restart-child-resume-plan","in_progress","RESTART_CHILD_PAUSE")
                } else {
                    assert_eq!(state.resumed_reports.fetch_add(1,Ordering::SeqCst),0);
                    child_report(&body,true,ordinal)
                }
            }
        } else if users.contains("RESTART_CHILD_HEALTHY") {state.healthy_calls.fetch_add(1,Ordering::SeqCst);(json!({"role":"assistant","content":"healthy_result"}),"stop")}
        else {state.parent_calls.fetch_add(1,Ordering::SeqCst);parent(&body)};
        let chunk=json!({"id":"restart-fixture","object":"chat.completion.chunk","created":1,"model":body["model"],"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let end=json!({"id":"restart-fixture","object":"chat.completion.chunk","created":1,"model":body["model"],"choices":[{"index":0,"delta":{},"finish_reason":reason}]});
        ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
    })).with_state(state.clone());
    let task=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});(url,state,task)
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn paused_child_user_restart_keeps_original_parent_wait_and_delivers_both_original_tasks() {
    let (url,state,server)=server().await;let fixture=Fixture::with_input_model("native-original-child-user-restart",&url).await;
    sqlx::query("CREATE TRIGGER fixture_restart_parallelism AFTER INSERT ON agent_executions BEGIN UPDATE agent_executions SET max_parallel=4,adaptation_policy='fixed' WHERE execution_id=NEW.execution_id; END").execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER fixture_restart_capacity AFTER INSERT ON agent_execution_participants BEGIN UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":4}' WHERE participant_id=NEW.participant_id; END").execute(fixture.pool()).await.unwrap();
    let (session,workspace)=fanout_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({"idempotency_key":"original-parent-one-turn","input":{"content":ROOT}})).await;
    let (execution,step,attempt,child,child_operation)=tokio::time::timeout(Duration::from_secs(60),async {loop {
        let paused:Option<(String,String,String,String,String)>=sqlx::query_as("SELECT attempt.execution_id,attempt.step_id,attempt.attempt_id,link.conversation_id,turn.operation_id FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.execution_id=attempt.execution_id AND step.step_id=attempt.step_id JOIN conversation_execution_links link ON link.attempt_id=attempt.attempt_id AND link.active=1 AND link.relation='attempt' JOIN agent_turns turn ON turn.session_id=link.conversation_id WHERE step.title='restart-child' AND step.status='waiting_input' AND json_extract(attempt.runtime_state,'$.review_blocked.kind')='native_pause' AND turn.state='running' AND json_extract(turn.native_pause_json,'$.cleanup_proven')=1 AND json_extract(turn.native_pause_json,'$.checkpoint_revision')>0")
            .fetch_optional(fixture.pool()).await.unwrap();
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.call.call_id')='restart-wait-again') AND COALESCE(MAX(CASE WHEN json_extract(inline_json,'$.event.event')='awaiting_children' THEN seq END),0)>COALESCE(MAX(CASE WHEN json_extract(inline_json,'$.event.event')='children_wait_resolved' THEN seq END),0) FROM agent_events WHERE session_id=?")
            .bind(session.as_ref()).bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
        if let Some(paused)=paused && waiting&&state.healthy_calls.load(Ordering::SeqCst)==1 {break paused;}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.expect("a real technical child pause becomes visible while the original parent remains subscribed");
    let calls=state.parent_calls.load(Ordering::SeqCst);tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(state.parent_calls.load(Ordering::SeqCst),calls,"the same acknowledged pause cannot poll the model");
    let parent_head:(String,i64)=sqlx::query_as("SELECT status,(SELECT COUNT(*) FROM agent_turns WHERE session_id=?) FROM agent_session_heads WHERE session_id=?")
        .bind(session.as_ref()).bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(parent_head,("running".into(),1));
    let versions:(i64,i64)=sqlx::query_as("SELECT execution.version,step.version FROM agent_executions execution JOIN agent_execution_steps step ON step.execution_id=execution.execution_id WHERE execution.execution_id=? AND step.step_id=?")
        .bind(&execution).bind(&step).fetch_one(fixture.pool()).await.unwrap();
    let restart=json!({"idempotency_key":"one-exact-user-restart","expected_execution_version":versions.0,"expected_step_version":versions.1,"expected_attempt_id":attempt});
    let path=format!("/api/agent-executions/{execution}/steps/{step}/restart");
    let first=api(&fixture._router,&path,restart.clone()).await;let replay=api(&fixture._router,&path,restart).await;
    assert!(first["operation_id"].as_str().is_some_and(|id|!id.is_empty()));
    assert_eq!(first["execution_id"],execution);assert_eq!(first["step_ids"],json!([step]));
    assert_eq!(first["replayed"],false);assert_eq!(replay["replayed"],true);
    assert_eq!(first["operation_id"],replay["operation_id"],"lost acknowledgement replay must name one accepted restart");
    tokio::time::timeout(Duration::from_secs(45),async {while !state.resumed.load(Ordering::SeqCst) {tokio::time::sleep(Duration::from_millis(20)).await;}}).await
        .expect("explicit user restart resumes the exact child checkpoint");
    assert_eq!(state.rejected_reports.load(Ordering::SeqCst),4);assert_eq!(state.resumed_calls.load(Ordering::SeqCst),1);
    state.release_resumed.store(true,Ordering::SeqCst);state.wake_resumed.notify_waiters();
    let operation=await_completion(&fixture,&session,&accepted).await;
    let child_identity:(i64,i64,String)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_execution_attempts WHERE step_id=?),(SELECT COUNT(*) FROM agent_turns WHERE session_id=?),(SELECT operation_id FROM agent_turns WHERE session_id=?)")
        .bind(&step).bind(&child).bind(&child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(child_identity,(1,1,child_operation));
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_execution_steps),(SELECT COUNT(*) FROM agent_execution_attempts),(SELECT COUNT(*) FROM agent_turns WHERE session_id=?)")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(2,2,1));
    assert_eq!(state.healthy_calls.load(Ordering::SeqCst),1);
    assert_eq!(state.child_calls.load(Ordering::SeqCst),state.setup_calls.load(Ordering::SeqCst)+6);
    assert_eq!(state.resumed_calls.load(Ordering::SeqCst),2);
    assert_eq!(state.resumed_plans.load(Ordering::SeqCst),1);
    assert_eq!(state.resumed_reports.load(Ordering::SeqCst),1);
    let writes:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE session_id=? AND action_id='workspace.files/write'")
        .bind(&child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(writes,1,"restart must not replay the original file mutation");
    assert_eq!(std::fs::read_to_string(workspace.join("restart-original.txt")).unwrap(),"restarted_result");
    let parent_pauses:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/paused'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(parent_pauses,0,"child intervention cannot leave its parent unable to consume a resumed result");
    let resumes:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/resume-authorized'")
        .bind(&child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(resumes,1,"replayed Restart has one native authorization");
    let delegates:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_call_completed' AND json_extract(inline_json,'$.event.call.name')='delegate_task'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(delegates,1);
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();
    let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    assert_eq!(nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap().result_text.as_deref(),Some("restarted_result+healthy_result"));
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.abort();let _=server.await;
}

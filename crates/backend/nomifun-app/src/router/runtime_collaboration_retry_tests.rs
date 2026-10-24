//! Full native parent/child delivery through provider failure and exact input
//! handoff failure. No production Session or external model is used.
use super::*;

const ROOT:&str="RETRY_ROOT: Use two subagents for two independent text modules. If one fails, retry that same task once, keep managing it, read both exact results and integrate them in your final answer.";
const FAILED:&str="RETRY_CHILD_FAIL: Return exactly retry_result as the complete assigned module. Do not delegate or invoke a tool.";
const HEALTHY:&str="RETRY_CHILD_HEALTHY: Return exactly healthy_result as the complete assigned module. Do not delegate or invoke a tool.";
const RESULT:&str="retry_result+healthy_result";

#[derive(Default)]
struct State {
    requests:Mutex<Vec<Value>>,provider_failure:bool,child_calls:AtomicUsize,parent_calls:AtomicUsize,
    failure_entered:AtomicBool,retry_entered:AtomicBool,release_failure:AtomicBool,release_retry:AtomicBool,
    wake_failure:tokio::sync::Notify,wake_retry:tokio::sync::Notify,
}

fn parent(body:&Value,provider_failure:bool)->(Value,&'static str) {
    if returned(body,"retry-delegate").is_none() {return selected(body,"delegate_task","retry-delegate","agent/delegate",json!({"strategy":"parallel","tasks":[
        {"name":"retry-child","prompt":FAILED,"tool_policy":"full"},{"name":"healthy-child","prompt":HEALTHY,"tool_policy":"full"}]}));}
    let execution=returned(body,"retry-delegate").unwrap()["execution_id"].as_str().unwrap().to_owned();
    if returned(body,"retry-wait-initial").is_none() {return call("agent_wait","retry-wait-initial",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"retry-status-initial").is_none() {return call("agent_status","retry-status-initial",json!({"execution_ids":[execution]}));}
    if provider_failure {
        let status=returned(body,"retry-status-initial").unwrap();let row=&status["result"]["executions"][0];
        let child=row["children"].as_array().unwrap().iter().find(|child|child["title"]=="retry-child").unwrap();
        assert_eq!(child["status"],"failed","the parent decides on an actual known failure");
        for id in ["retry-original-command","retry-replayed-command"] {
            if returned(body,id).is_none() {return call("agent_message",id,json!({"execution_id":execution,"step_id":child["step_id"],
                "content":"Retry this same logical child; preserve the original successful sibling.","idempotency_key":"same-native-retry-command",
                "intent":{"kind":"task_control","control":{"expected_execution_version":row["version"],"expected_step_version":child["version"],"action":{"kind":"retry"}}}}));}
            assert_eq!(returned(body,id).unwrap()["accepted"],true,"native source-scoped retry must be accepted: {:?}",returned(body,id));
        }
        assert_eq!(returned(body,"retry-original-command").unwrap()["operation_id"],returned(body,"retry-replayed-command").unwrap()["operation_id"]);
        if returned(body,"retry-stale-stop").is_none() {return call("agent_message","retry-stale-stop",json!({"execution_id":execution,"step_id":child["step_id"],
            "content":"This intentionally stale Stop targets only the failed original attempt.","idempotency_key":"stale-original-stop",
            "intent":{"kind":"task_control","control":{"expected_execution_version":row["version"],"expected_step_version":child["version"],"action":{"kind":"stop"}}}}));}
        assert_eq!(returned(body,"retry-stale-stop").unwrap()["accepted"],false,"old Step versions cannot stop a successor");
        if returned(body,"retry-wait-successor").is_none() {return call("agent_wait","retry-wait-successor",json!({"execution_ids":[execution],"mode":"all"}));}
        if returned(body,"retry-status-final").is_none() {return call("agent_status","retry-status-final",json!({"execution_ids":[execution]}));}
    }
    let status=returned(body,if provider_failure {"retry-status-final"} else {"retry-status-initial"}).unwrap();
    assert_eq!(status["result"]["all_ready"],true);
    let children=status["result"]["executions"][0]["children"].as_array().unwrap();
    assert!(children.iter().any(|child|child["title"]=="retry-child"&&child["result"]=="retry_result"));
    assert!(children.iter().any(|child|child["title"]=="healthy-child"&&child["result"]=="healthy_result"));
    (json!({"role":"assistant","content":RESULT}),"stop")
}

async fn server(provider_failure:bool)->(String,Arc<State>,tokio::task::JoinHandle<()>) {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}/v1",listener.local_addr().unwrap());
    let state=Arc::new(State {provider_failure,..Default::default()});
    let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(|axum::extract::State(state):axum::extract::State<Arc<State>>,axum::Json(body):axum::Json<Value>|async move {
        state.requests.lock().unwrap().push(body.clone());let users=user_text(&body);
        let (delta,reason)=if users.contains("RETRY_CHILD_FAIL") {
            let call=state.child_calls.fetch_add(1,Ordering::SeqCst);
            if state.provider_failure&&call==0 {
                state.failure_entered.store(true,Ordering::SeqCst);
                loop {let wake=state.wake_failure.notified();tokio::pin!(wake);wake.as_mut().enable();if state.release_failure.load(Ordering::SeqCst){break;}wake.await;}
                return (axum::http::StatusCode::BAD_REQUEST,axum::Json(json!({"error":{"message":"Synthetic known provider rejection before a tool call","type":"invalid_request_error","code":"fixture_rejection"}}))).into_response();
            }
            state.retry_entered.store(true,Ordering::SeqCst);
            loop {let wake=state.wake_retry.notified();tokio::pin!(wake);wake.as_mut().enable();if state.release_retry.load(Ordering::SeqCst){break;}wake.await;}
            (json!({"role":"assistant","content":"retry_result"}),"stop")
        } else if users.contains("RETRY_CHILD_HEALTHY") {(json!({"role":"assistant","content":"healthy_result"}),"stop")}
        else {state.parent_calls.fetch_add(1,Ordering::SeqCst);parent(&body,state.provider_failure)};
        let chunk=json!({"id":"retry-fixture","object":"chat.completion.chunk","created":1,"model":body["model"],"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let end=json!({"id":"retry-fixture","object":"chat.completion.chunk","created":1,"model":body["model"],"choices":[{"index":0,"delta":{},"finish_reason":reason}]});
        ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
    })).with_state(state.clone());
    let task=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});(url,state,task)
}

async fn run(provider_failure:bool) {
    let (url,state,server)=server(provider_failure).await;
    let fixture=Fixture::with_input_model(if provider_failure {"native-parent-same-step-retry"} else {"native-original-input-busy-handoff"},&url).await;
    sqlx::query("CREATE TRIGGER fixture_retry_policy AFTER INSERT ON agent_executions BEGIN UPDATE agent_executions SET max_parallel=4,adaptation_policy='fixed' WHERE execution_id=NEW.execution_id; END").execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER fixture_retry_capacity AFTER INSERT ON agent_execution_participants BEGIN UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":4}' WHERE participant_id=NEW.participant_id; END").execute(fixture.pool()).await.unwrap();
    if !provider_failure {fixture.owner.inject_attempt_input_busy_for_test("RETRY_CHILD_FAIL");}
    let (session,_)=fanout_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({"idempotency_key":"native-parent-original-once","input":{"content":ROOT}})).await;
    if provider_failure {
        tokio::time::timeout(Duration::from_secs(45),async {loop {
            let healthy:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps WHERE title='healthy-child' AND status='completed'").fetch_one(fixture.pool()).await.unwrap();
            if state.failure_entered.load(Ordering::SeqCst)&&healthy==1 {break;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }}).await.expect("healthy sibling completes before the known independent failure");
        state.release_failure.store(true,Ordering::SeqCst);state.wake_failure.notify_waiters();
    }
    tokio::time::timeout(Duration::from_secs(60),async {loop {
        let waiting:bool=sqlx::query_scalar("SELECT COALESCE(MAX(CASE WHEN json_extract(inline_json,'$.event.event')='awaiting_children' THEN seq END),0)>COALESCE(MAX(CASE WHEN json_extract(inline_json,'$.event.event')='children_wait_resolved' THEN seq END),0) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded'")
            .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
        let explicit:bool=if provider_failure {sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.call.call_id')='retry-wait-successor')")
            .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap()} else {true};
        if state.retry_entered.load(Ordering::SeqCst)&&waiting&&explicit {break;}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.expect("the exact original parent yields while the child is still working");
    // The original explicit wait is durably outstanding, and no child fact
    // changes while the controlled provider remains at this barrier.
    let calls=state.parent_calls.load(Ordering::SeqCst);tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state.parent_calls.load(Ordering::SeqCst),calls,"unchanged explicit WAIT cannot poll the provider");
    state.release_retry.store(true,Ordering::SeqCst);state.wake_retry.notify_waiters();
    let operation=await_completion(&fixture,&session,&accepted).await;
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_execution_steps),(SELECT COUNT(*) FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.step_id=attempt.step_id WHERE step.title='retry-child'),(SELECT COUNT(*) FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.step_id=attempt.step_id WHERE step.title='healthy-child')")
        .fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(2,if provider_failure {2}else{1},1));
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(turns,1);
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    assert_eq!(nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap().result_text.as_deref(),Some(RESULT));
    let child_inputs:Vec<(String,i64,i64)>=sqlx::query_as("SELECT DISTINCT link.conversation_id,(SELECT COUNT(*) FROM agent_events event WHERE event.session_id=link.conversation_id AND event.kind='message/user-accepted'),(SELECT COUNT(*) FROM agent_turns turn WHERE turn.session_id=link.conversation_id) FROM conversation_execution_links link WHERE link.relation='attempt'")
        .fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(child_inputs.len(),if provider_failure {3}else{2});
    for (child,accepted,turns) in child_inputs {assert_eq!((accepted,turns),(1,1),"each original child input is accepted and executed once: {child}");}
    assert_eq!(state.child_calls.load(Ordering::SeqCst),if provider_failure {2}else{1},"input recovery cannot add a provider invocation; explicit retry adds exactly its one new Attempt");
    assert_eq!(state.requests.lock().unwrap().iter().filter(|request|user_text(request).contains("RETRY_CHILD_HEALTHY")).count(),1,"the successful sibling is never recomputed");
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.abort();let _=server.await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn parent_retries_same_child_once_preserves_sibling_and_delivers_original_turn() {run(true).await;}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn original_child_input_busy_recovers_attempt_zero_and_parent_delivers() {run(false).await;}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn queued_source_preparation_uses_exact_opening_without_granting_a_runtime_link() {
    use nomifun_agent_execution::AgentExecutionSessionPort;
    use nomifun_db::IAgentExecutionRepository;
    let (url,state,server)=server(false).await;let fixture=Fixture::with_input_model("native-exact-queued-source-preparation",&url).await;
    state.release_retry.store(true,Ordering::SeqCst);
    sqlx::query("CREATE TRIGGER fixture_preparing_parallelism AFTER INSERT ON agent_executions BEGIN UPDATE agent_executions SET max_parallel=4,adaptation_policy='fixed' WHERE execution_id=NEW.execution_id; END").execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER fixture_preparing_capacity AFTER INSERT ON agent_execution_participants BEGIN UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":4}' WHERE participant_id=NEW.participant_id; END").execute(fixture.pool()).await.unwrap();
    let (session,_)=fanout_parent(&fixture).await;
    let (entered,release)=fixture.owner.inject_attempt_input_owner_fault("RETRY_CHILD_FAIL","preparing-source",AppError::DatabaseUnavailable {
        reason:nomifun_common::DatabaseUnavailableReason::Busy},true);
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({"idempotency_key":"queued-source-original-once",
        "input":{"content":ROOT,"files":[ATTACHMENT],"inject_skills":[SKILL]}})).await;
    tokio::time::timeout(Duration::from_secs(45),entered.notified()).await.expect("original child preparation reaches its exact pre-link boundary");
    let (execution,step,attempt,raw):(String,String,String,String)=sqlx::query_as("SELECT attempt.execution_id,attempt.step_id,attempt.attempt_id,attempt.runtime_state FROM agent_execution_attempts attempt JOIN agent_execution_steps step ON step.step_id=attempt.step_id WHERE step.title='retry-child' AND attempt.status='queued'")
        .fetch_one(fixture.pool()).await.unwrap();
    let user=fixture.services.authoritative_user_id.as_ref();let key=format!("user:{user}:open:execution-source:{attempt}");
    let child:String=sqlx::query_scalar("SELECT session_id FROM agent_events WHERE kind='session/opening' AND producer_id='session_api' AND idempotency_key=?")
        .bind(key).fetch_one(fixture.pool()).await.unwrap();
    let repository=nomifun_db::SqliteAgentExecutionRepository::new(fixture.pool().clone());
    let source=repository.step_source(user,&execution,&step).await.unwrap().unwrap();
    let pending=nomifun_db::AttemptConversationEffects::decode(Some(&raw)).unwrap();
    let [nomifun_db::PendingConversationEffect::InitialInput {request}]=pending.pending_conversation_effects.as_slice() else {panic!("original queued request must be retained");};
    let request:nomifun_api_types::SendMessageRequest=serde_json::from_value(request.clone()).unwrap();
    assert!(fixture.owner.read_execution_source_context(user,&child.clone().into()).await.unwrap().is_none(),"preparation cannot manufacture active Runtime context");
    assert!(fixture.owner.prepare_source_child_input(user,session.as_ref(),&attempt,Some(&source),request.clone()).await.is_err(),"a different Session cannot borrow the queued task");
    assert!(fixture.owner.prepare_source_child_input(user,&child,&uuid::Uuid::now_v7().to_string(),Some(&source),request.clone()).await.is_err(),"a different reservation cannot borrow the original opening");
    let mut foreign=source.clone();foreign.tool_operation_id.push_str(":different");
    assert!(fixture.owner.prepare_source_child_input(user,&child,&attempt,Some(&foreign),request.clone()).await.is_err(),"source selection is proved from the original Step, never the caller's guessed source");
    let prepared=fixture.owner.prepare_source_child_input(user,&child,&attempt,Some(&source),request.clone()).await.unwrap();
    assert_eq!(prepared.content,request.content);assert_eq!(prepared.origin,request.origin);assert_eq!(prepared.hidden,request.hidden);
    assert!(prepared.files.iter().any(|file|file.ends_with(ATTACHMENT)));assert!(prepared.inject_skills.iter().any(|skill|skill==SKILL));
    let before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM conversation_execution_links WHERE conversation_id=? AND relation='attempt'").bind(&child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(before,0);
    // A stale runner can lose its lease while the next owner prepares this
    // same reservation. Cleanup registration must release that stale waiter
    // without deleting the still-Queued Attempt's original Session.
    tokio::time::timeout(Duration::from_secs(3),fixture.owner.discard_unlinked_creation(user,&attempt,Some(&child))).await
        .expect("cleanup transfers responsibility without blocking domain disposition").unwrap();
    assert!(fixture.owner.canonical().store().get_live_session(&child.clone().into()).await.is_ok());
    release.unwrap().notify_one();let operation=await_completion(&fixture,&session,&accepted).await;
    let attempts:Vec<String>=sqlx::query_scalar("SELECT attempt_id FROM agent_execution_attempts WHERE step_id=?").bind(&step).fetch_all(fixture.pool()).await.unwrap();assert_eq!(attempts,[attempt.clone()]);
    let linked:Vec<String>=sqlx::query_scalar("SELECT DISTINCT conversation_id FROM conversation_execution_links WHERE attempt_id=? AND relation='attempt'")
        .bind(&attempt).fetch_all(fixture.pool()).await.unwrap();assert_eq!(linked,[child.clone()],"transient preparation keeps the original created Session");
    let preserved:(String,i64,i64,i64)=sqlx::query_as("SELECT session.state,(SELECT COUNT(*) FROM agent_events event WHERE event.session_id=session.agent_session_id AND event.kind='session/opening'),(SELECT COUNT(*) FROM agent_events event WHERE event.session_id=session.agent_session_id AND event.kind='message/user-accepted'),(SELECT COUNT(*) FROM agent_turns turn WHERE turn.session_id=session.agent_session_id) FROM agent_sessions session WHERE session.agent_session_id=?")
        .bind(&child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(preserved,("live".into(),1,1,1));
    let cancelled:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/cancelled'),(SELECT COUNT(*) FROM agent_execution_events WHERE step_id=? AND json_extract(payload,'$.attempt_status')='cancelled')")
        .bind(&child).bind(&step).fetch_one(fixture.pool()).await.unwrap();assert_eq!(cancelled,(0,0),"temporary pre-link storage pressure never becomes cancellation or another business attempt");
    assert_eq!(state.child_calls.load(Ordering::SeqCst),1);
    assert_eq!(state.requests.lock().unwrap().iter().filter(|request|user_text(request).contains("RETRY_CHILD_HEALTHY")).count(),1);
    let parent_turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(parent_turns,1);
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    assert_eq!(nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap().result_text.as_deref(),Some(RESULT));
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.abort();let _=server.await;
}

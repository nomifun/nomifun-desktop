//! Disposable production Host/HTTP collaboration: overlapping real model
//! requests, exact-source joins, parent integration and native verification.
use super::*;
use axum::response::IntoResponse;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) const ROOT_REQUEST: &str = "COORDINATOR_ROOT: Use subagents to obtain two independent text modules. Follow up with the completed alpha child, integrate its new result with beta, write integrated.txt and verify its contents using the native process tool.";
const INHERITED: &str = "INHERITED_COLLABORATION_AUTHORED_INSTRUCTIONS_73921";
const EXPECTED: &str = "alpha_followup+beta";

#[derive(Default)]
struct ModelState {
    requests: Mutex<Vec<Value>>,
    child_active: AtomicUsize,
    child_peak: AtomicUsize,
    child_entered: AtomicUsize,
    release_children: AtomicBool,
    release: tokio::sync::Notify,
    fork_only: AtomicBool,
}

pub(super) struct ModelServer { pub(super) url:String, state:Arc<ModelState>, task:tokio::task::JoinHandle<()> }

fn available(body:&Value,name:&str)->bool {
    body["tools"].as_array().is_some_and(|tools|tools.iter().any(|tool|tool["function"]["name"]==name))
}

fn returned(body:&Value,id:&str)->Option<Value> {
    let message=body["messages"].as_array()?.iter().rev()
        .find(|message|message["role"]=="tool" && message["tool_call_id"]==id)?;
    serde_json::from_str(message["content"].as_str()?).ok()
}

fn returned_with(body:&Value,prefix:&str, predicate:impl Fn(&Value)->bool)->Option<(String,Value)> {
    body["messages"].as_array()?.iter().rev().filter(|message|message["role"]=="tool")
        .filter_map(|message|Some((message["tool_call_id"].as_str()?,serde_json::from_str::<Value>(message["content"].as_str()?).ok()?)))
        .find(|(id,value)|id.starts_with(prefix)&&predicate(value)).map(|(id,value)|(id.into(),value))
}

fn call(name:&str,id:&str,arguments:Value)->(Value,&'static str) {
    (json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
        "function":{"name":name,"arguments":arguments.to_string()}}]}),"tool_calls")
}

fn selected(body:&Value,name:&str,id:&str,action:&str,arguments:Value)->(Value,&'static str) {
    if available(body,name) {call(name,id,arguments)} else {call("ToolSearch",&format!("{id}-discover"),json!({"query":action}))}
}

fn plan(id:&str,status:&str)->(Value,&'static str) {
    call("update_plan",id,json!({"explanation":"Integrate only exact recorded child outputs and verify the native result.",
        "plan":[{"step":"Integrate the child modules and verify their file","status":status}],
        "requirements":[{"id":"original","description":"Complete the original collaboration and integration request",
            "source":{"input":0,"quote":"COORDINATOR_ROOT"}}]}))
}

fn fork_parent_step(body:&Value)->(Value,&'static str) {
    if returned(body,"fork-original").is_none() {
        return selected(body,"fork_agent","fork-original","agent/fork",json!({
            "goal":"COORDINATOR_CHILD_FORK: Return exactly fork_result. Do not delegate or invoke a tool."}));
    }
    let original=returned(body,"fork-original").unwrap();
    let execution=original["execution_id"].as_str().expect("the real fork owner returns an Execution receipt");
    if returned(body,"fork-wait").is_none() {return call("agent_wait","fork-wait",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"fork-status").is_none() {return call("agent_status","fork-status",json!({"execution_ids":[execution]}));}
    let observed=returned(body,"fork-status").unwrap();
    let children=observed["result"]["executions"][0]["children"].as_array().unwrap();
    assert_eq!(children.len(),1);
    assert_eq!(children[0]["result"],"fork_result");
    assert!(children[0]["source"]["terminal_event_id"].is_string());
    (json!({"role":"assistant","content":"The original parent integrated fork_result from the exact forked child result."}),"stop")
}

fn parent_step(body:&Value,ordinal:usize)->(Value,&'static str) {
    let tasks=json!({"strategy":"parallel","tasks":[
        {"name":"alpha","prompt":"COORDINATOR_CHILD_ALPHA: Return exactly alpha as your complete independent module. Do not delegate or invoke a tool.","tool_policy":"full"},
        {"name":"beta","prompt":"COORDINATOR_CHILD_BETA: Return exactly beta as your complete independent module. Do not delegate or invoke a tool.","tool_policy":"full"}
    ]});
    if returned(body,"delegate-original").is_none() {
        return selected(body,"delegate_task","delegate-original","agent/delegate",tasks);
    }
    if returned(body,"delegate-repeat").is_none() {
        return selected(body,"delegate_task","delegate-repeat","agent/delegate",tasks);
    }
    let original=returned(body,"delegate-original").unwrap();
    let execution=original["execution_id"].as_str().expect("the real delegate owner returns an Execution receipt");
    assert_eq!(returned(body,"delegate-repeat").unwrap()["execution_id"],execution,
        "fresh native call IDs for the same accepted work must return its original identity");
    if returned(body,"wait-initial").is_none() {return call("agent_wait","wait-initial",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"status-initial").is_none() {return call("agent_status","status-initial",json!({"execution_ids":[execution]}));}
    let initial=returned(body,"status-initial").unwrap();
    let children=initial["result"]["executions"][0]["children"].as_array().expect("the status control reads canonical child results");
    let alpha=children.iter().find(|child|child["title"]=="alpha").expect("the original alpha child is still addressable");
    assert_eq!(alpha["result"],"alpha");
    assert!(alpha["source"]["terminal_event_id"].is_string());
    if returned(body,"followup-alpha").is_none() {
        return call("agent_message","followup-alpha",json!({"execution_id":execution,"step_id":alpha["step_id"],
            "content":"COORDINATOR_CHILD_ALPHA_FOLLOWUP: Return exactly alpha_followup; continue in your existing child Session.","idempotency_key":"alpha-followup-once"}));
    }
    if returned(body,"wait-followup").is_none() {return call("agent_wait","wait-followup",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"status-followup").is_none() {return call("agent_status","status-followup",json!({"execution_ids":[execution]}));}
    let followed=returned(body,"status-followup").unwrap();
    assert!(followed.to_string().contains("alpha_followup"),"the parent reads the actual followup result before integration");
    if available(body,"update_plan") && returned(body,"parent-plan").is_none() {return plan("parent-plan","in_progress");}
    if returned_with(body,"parent-write-",|result|result["written"]==true).is_none() {
        return selected(body,"write_file",&format!("parent-write-{ordinal}"),"workspace.files/write",json!({"path":"integrated.txt","content":EXPECTED}));
    }
    let started=returned_with(body,"parent-process-",|result|result["process_id"].is_string());
    let terminal=returned_with(body,"parent-poll-",|result|result["state"]=="exited")
        .or_else(||returned_with(body,"parent-process-",|result|result["state"]=="exited"));
    if terminal.is_none() {
        if let Some((_,started))=started {return selected(body,"poll_process",&format!("parent-poll-{ordinal}"),"workspace.process/poll",
            json!({"process_id":started["process_id"],"cursor":0,"wait_ms":30000}));}
        #[cfg(windows)]
        let (command,args)=(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),json!(["/d","/c","type","integrated.txt"]));
        #[cfg(not(windows))]
        let (command,args)=(PathBuf::from("/bin/sh"),json!(["-c","cat integrated.txt"]));
        return selected(body,"start_process",&format!("parent-process-{ordinal}"),"workspace.process/start",
            json!({"command":command,"args":args,"execution_identity":"current_user","tty":false,"wait_ms":0,"timeout_ms":5000}));
    }
    let (_,terminal)=terminal.unwrap();
    assert_eq!(terminal["exit_code"],0,"the native verification process actually succeeds");
    assert!(terminal.to_string().contains(EXPECTED),"the real process read the integrated file");
    if returned(body,"parent-plan-done").is_none() {return plan("parent-plan-done","completed");}
    let definition=body["tools"].as_array().unwrap().iter().find(|tool|tool["function"]["name"]=="report_completion")
        .expect("the completed native work uses the existing completion account");
    let properties=&definition["function"]["parameters"]["properties"];
    let eligible=properties["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"].as_array().unwrap();
    let evidence=eligible.iter().filter_map(Value::as_str).find(|id|id.starts_with("parent-poll-")||id.starts_with("parent-process-"))
        .expect("the original native process's exact observation is eligible");
    let mut report=json!({"summary":"The two concurrent modules and the completed-child followup were integrated; the native verification returned alpha_followup+beta.",
        "criteria":[{"requirement_ids":["original"],"disposition":"supported","evidence_call_ids":[evidence],
            "rationale":"The native process read the parent-integrated file, returned the expected content, and exited zero."}]});
    for field in ["observed_tool_error_count","observed_command_failure_count"] {
        if !properties[field]["const"].is_null() {report[field]=properties[field]["const"].clone();}
        else if let Some(value)=properties[field]["enum"].as_array().and_then(|values|values.first()) {report[field]=value.clone();}
    }
    call("report_completion",&format!("parent-completion-{ordinal}"),report)
}

impl ModelServer {
    pub(super) async fn new()->Self {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/v1",listener.local_addr().unwrap());
        let state=Arc::new(ModelState::default());
        let router=axum::Router::new().route("/v1/chat/completions",axum::routing::post(|
            axum::extract::State(state):axum::extract::State<Arc<ModelState>>,axum::Json(body):axum::Json<Value>,
        |async move {
            let ordinal={let mut requests=state.requests.lock().unwrap();requests.push(body.clone());requests.len()};
            let users=body["messages"].as_array().unwrap().iter().filter(|message|message["role"]=="user")
                .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).collect::<Vec<_>>().join("\n");
            let (delta,reason)=if users.contains("COORDINATOR_CHILD_FORK") {
                (json!({"role":"assistant","content":"fork_result"}),"stop")
            } else if users.contains("COORDINATOR_CHILD_ALPHA_FOLLOWUP") {
                (json!({"role":"assistant","content":"alpha_followup"}),"stop")
            } else if users.contains("COORDINATOR_CHILD_ALPHA") || users.contains("COORDINATOR_CHILD_BETA") {
                let active=state.child_active.fetch_add(1,Ordering::SeqCst)+1;
                state.child_peak.fetch_max(active,Ordering::SeqCst);
                state.child_entered.fetch_add(1,Ordering::SeqCst);
                loop {
                    let wake=state.release.notified();tokio::pin!(wake);wake.as_mut().enable();
                    if state.release_children.load(Ordering::SeqCst) {break;}
                    wake.await;
                }
                state.child_active.fetch_sub(1,Ordering::SeqCst);
                (json!({"role":"assistant","content":if users.contains("COORDINATOR_CHILD_ALPHA") {"alpha"} else {"beta"}}),"stop")
            } else if state.fork_only.load(Ordering::SeqCst) {fork_parent_step(&body)} else {parent_step(&body,ordinal)};
            let chunk=json!({"id":format!("collaboration-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":body["model"],
                "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
            let end=json!({"id":format!("collaboration-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":body["model"],
                "choices":[{"index":0,"delta":{},"finish_reason":reason}]});
            ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
        })).with_state(state.clone());
        let task=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        Self{url,state,task}
    }
    pub(super) fn release(&self){self.state.release_children.store(true,Ordering::SeqCst);self.state.release.notify_waiters();}
    pub(super) async fn close(self){self.release();self.task.abort();let _=self.task.await;}
}

pub(super) async fn create_parent(fixture:&Fixture)->(AgentSessionId,PathBuf) {
    create_parent_with_collaboration_actions(fixture,&["agent/delegate","agent/fork"]).await
}

async fn create_parent_with_collaboration_actions(fixture:&Fixture,actions:&[&str])->(AgentSessionId,PathBuf) {
    let original=fixture.owner.get_session(fixture.services.authoritative_user_id.as_ref(),&fixture.host.options.conversation_id).await.unwrap();
    let model=serde_json::to_value(original.model.unwrap()).unwrap();
    let editor=api(&fixture._router,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Concurrent collaboration fixture","model":model})).await;
    let preset=editor["preset"]["preset_id"].as_str().unwrap();
    let mut draft=editor["draft"].clone();
    draft["document"]["instructions"]=json!(format!("{INHERITED}. Preserve the original task and exact module interfaces."));
    draft["document"]["enabled_capabilities"]=json!([
        {"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/write"]},
        {"capability":{"id":"workspace.process"},"action_allowlist":["workspace.process/start","workspace.process/poll"]},
        {"capability":{"id":"agent.collaboration"},"action_allowlist":actions}
    ]);
    api(&fixture._router,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],"draft":draft,"reason":"isolated production collaboration proof"})).await;
    let session=api(&fixture._router,"/api/agent-sessions",json!({"preset_id":preset,"model":model,
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]})).await;
    let session=AgentSessionId::from(session["agent_session_id"].as_str().unwrap());
    let projection=fixture.owner.get_session(fixture.services.authoritative_user_id.as_ref(),session.as_ref()).await.unwrap();
    (session,PathBuf::from(projection.extra["workspace"].as_str().unwrap()))
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn concurrent_full_children_join_in_the_parent_then_follow_up_integrate_and_verify_native_output() {
    let server=ModelServer::new().await;
    let fixture=Fixture::with_input_model("native-concurrent-collaboration",&server.url).await;
    let (session,workspace)=create_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({
        "idempotency_key":"collaboration-root-once","input":{"content":ROOT_REQUEST}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    let operation=tokio::time::timeout(Duration::from_secs(30),async{loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap(){break claim.operation_id;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the ordinary HTTP input must claim its exact parent Turn");
    tokio::time::timeout(Duration::from_secs(30),async{loop {
        let waiting:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='awaiting_children'")
            .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
        if server.state.child_entered.load(Ordering::SeqCst)>=2 && waiting>0 {break;}
        let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
        assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"parent must stay active while children are held: {receipt:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("two Full children must enter actual model requests simultaneously while the parent natively waits");
    assert_eq!(server.state.child_peak.load(Ordering::SeqCst),2);
    let executions:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_executions").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(executions,1,"fresh repeat call IDs must not create a second execution");
    let steps:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(steps,2,"the repeated accepted request must not create duplicate children");
    let policies:Vec<String>=sqlx::query_scalar("SELECT tool_policy FROM agent_execution_steps").fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(policies,["full","full"]);
    server.release();
    tokio::time::timeout(Duration::from_secs(45),async{loop {
        let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
        if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break;}
        assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"parent integration must actually complete: {receipt:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the original parent must join, follow up, integrate and verify without another human input");
    assert_eq!(std::fs::read_to_string(workspace.join("integrated.txt")).unwrap(),EXPECTED);
    let sources:Vec<(String,String)>=sqlx::query_as("SELECT step.title,link.conversation_id FROM agent_execution_steps step JOIN conversation_execution_links link ON link.step_id=step.step_id AND link.execution_id=step.execution_id WHERE link.relation='attempt' ORDER BY step.id")
        .fetch_all(fixture.pool()).await.unwrap();
    let alpha=sources.iter().find(|(title,_)|title=="alpha").unwrap();
    assert!(sources.iter().filter(|(_,session)|session==&alpha.1).count()>=2,"completed-child followup keeps the exact child Session: {sources:?}");
    let requests=server.state.requests.lock().unwrap().clone();
    let child_requests=requests.iter().filter(|body|body["messages"].as_array().unwrap().iter().any(|message|
        message["role"]=="user" && message["content"].to_string().contains("COORDINATOR_CHILD_"))).collect::<Vec<_>>();
    assert!(child_requests.len()>=3);
    for body in child_requests {
        assert!(body.to_string().contains(INHERITED),"child instructions inherit the selected parent Agent configuration");
        assert!(available(body,"write_file") && available(body,"start_process"),"Full children retain the selected native file and process tools");
    }
    let parent_after_spawn=requests.iter().find(|body|returned(body,"delegate-original").is_some()
        && returned(body,"delegate-repeat").is_none()).expect("the parent actually continues after acceptance");
    assert!(available(parent_after_spawn,"write_file") && available(parent_after_spawn,"start_process"),
        "accepted delegation preserves the parent's selected integration tools");
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(turns,1,"joining and final integration remain in the original parent Turn");
    fixture.voice_execution.shutdown().await.unwrap();
    fixture.finish().await;
    server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn stopping_the_waiting_parent_is_exact_and_does_not_cancel_its_independent_children() {
    let server=ModelServer::new().await;
    let fixture=Fixture::with_input_model("native-collaboration-parent-stop",&server.url).await;
    let (session,workspace)=create_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),
        json!({"idempotency_key":"parent-stop-root","input":{"content":ROOT_REQUEST}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    let operation=tokio::time::timeout(Duration::from_secs(30),async{loop {
        let claim=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap();
        let waiting:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='awaiting_children'")
            .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
        if let Some(claim)=claim {if waiting>0 && server.state.child_entered.load(Ordering::SeqCst)>=2 {break claim.operation_id;}}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the original parent must reach its real native child wait");
    let before=server.state.requests.lock().unwrap().len();
    let stopped=api(&fixture._router,&format!("/api/agent-sessions/{}/turns/cancel",session.as_ref()),
        json!({"idempotency_key":"stop-only-this-parent","expected_turn_id":input.as_ref()})).await;
    assert_eq!(stopped["target_operation_id"],operation.as_ref());
    tokio::time::timeout(Duration::from_secs(5),async{loop {
        let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
        if receipt.status==nomifun_agent_session::TurnReceiptStatus::Cancelled {break;}
        tokio::time::sleep(Duration::from_millis(5)).await;
    }}).await.expect("host Stop must preempt the original native wait without a child answer");
    let running:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_attempts WHERE status='running'").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(running,2,"a local parent stop does not aggregate-cancel its independent children");
    server.release();
    tokio::time::timeout(Duration::from_secs(30),async{loop {
        let completed:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_steps WHERE status='completed'").fetch_one(fixture.pool()).await.unwrap();
        if completed==2 {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("already-authorized independent children retain their exact native owners");
    assert_eq!(server.state.requests.lock().unwrap().len(),before,"the stopped parent cannot sample another coordination step");
    assert!(!workspace.join("integrated.txt").exists(),"no late integration effect runs after the exact parent Stop");
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn fork_only_grant_keeps_native_join_and_exact_result_in_the_original_parent_turn() {
    let server=ModelServer::new().await;server.state.fork_only.store(true,Ordering::SeqCst);
    let fixture=Fixture::with_input_model("native-fork-only-collaboration",&server.url).await;
    let (session,_)=create_parent_with_collaboration_actions(&fixture,&["agent/fork"]).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({
        "idempotency_key":"fork-only-original","input":{"content":"COORDINATOR_FORK_ROOT: Fork one child, wait for it, read its exact result and integrate it in this original parent turn."}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    let operation=tokio::time::timeout(Duration::from_secs(45),async{loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap() {
            let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&claim.operation_id).await.unwrap();
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break claim.operation_id;}
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"fork-only grant must retain the original native join owner: {receipt:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("fork-only parent must join and deliver without delegate permission or another user Turn");
    let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();
    let delivery=nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap();
    assert_eq!(delivery.result_text.as_deref(),Some("The original parent integrated fork_result from the exact forked child result."));
    assert_eq!(delivery.result_ok,Some(true));
    let rows:Vec<(String,String)>=sqlx::query_as("SELECT link.conversation_id,step.status FROM conversation_execution_links link JOIN agent_execution_steps step ON step.step_id=link.step_id AND step.execution_id=link.execution_id WHERE link.relation='attempt'")
        .fetch_all(fixture.pool()).await.unwrap();
    assert_eq!(rows.len(),1);assert_ne!(rows[0].0,session.as_ref());assert_eq!(rows[0].1,"completed");
    let executions:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_executions").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(executions,1);
    let parent_turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(parent_turns,1);
    let requests=server.state.requests.lock().unwrap().clone();
    let parents=requests.iter().filter(|body|body["messages"].as_array().unwrap().iter().any(|message|
        message["role"]=="user"&&message["content"].to_string().contains("COORDINATOR_FORK_ROOT"))).collect::<Vec<_>>();
    assert!(!parents.is_empty());
    assert!(parents.iter().all(|body|!available(body,"delegate_task")),"fork authorization must not acquire delegate permission");
    assert!(available(parents[0],"agent_wait")&&available(parents[0],"agent_status"));
    assert!(parents.iter().any(|body|returned(body,"fork-status").is_some()));
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

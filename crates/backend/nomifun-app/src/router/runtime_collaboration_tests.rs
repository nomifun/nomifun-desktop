//! Disposable production Host/HTTP collaboration: overlapping real model
//! requests, exact-source joins, parent integration and native verification.
use super::*;
use axum::response::IntoResponse;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[path = "runtime_collaboration_fanout_tests.rs"]
mod fanout_tests;

#[path = "runtime_parallel_read_tests.rs"]
pub(super) mod parallel_read_tests;

#[path = "runtime_attempt_input_owner_tests.rs"]
mod attempt_input_owner_tests;

pub(super) const ROOT_REQUEST: &str = "COORDINATOR_ROOT: Use subagents to obtain two independent text modules. Follow up with the completed alpha child, integrate its new result with beta, write integrated.txt and verify its contents using the native process tool.";
const INHERITED: &str = "INHERITED_COLLABORATION_AUTHORED_INSTRUCTIONS_73921";
const EXPECTED: &str = "alpha_followup+beta";

#[test]
fn wait_distinguishes_new_decisions_live_progress_verified_outcomes_and_ownerless_stalls() {
    use super::super::collaboration_port::{wait_disposition,WaitDisposition};
    use nomifun_agent_runtime::AgentChildrenWaitMode::{All,Any};
    assert_eq!(wait_disposition(All,true,false,true,true,false,false),WaitDisposition::Wake);
    assert_eq!(wait_disposition(All,false,false,true,true,true,false),WaitDisposition::Park,
        "a healthy remaining producer may still change the same attention scope");
    assert_eq!(wait_disposition(All,false,false,true,true,false,false),WaitDisposition::DecisionBlocked,
        "repeated Review cannot wait forever after the final healthy producer leaves");
    assert_eq!(wait_disposition(Any,false,false,false,true,false,false),WaitDisposition::DecisionBlocked,
        "a Running/Queued ownerless gap has the same decision exit, even without a Question");
    assert_eq!(wait_disposition(All,true,true,true,false,false,false),WaitDisposition::Wake,
        "verified failure/cancellation outcomes are terminal, regardless of result success");
    assert_eq!(wait_disposition(All,false,true,true,false,false,false),WaitDisposition::DecisionBlocked,
        "already seen final outcomes cannot manufacture a new owner wake");
    assert_eq!(wait_disposition(Any,false,false,true,false,true,false),WaitDisposition::Park,
        "an unchanged successful sibling does not poll the model while another runs");
    assert_eq!(wait_disposition(All,true,false,true,true,false,true),WaitDisposition::Wake,
        "a real exact input question is presented once");
    assert_eq!(wait_disposition(All,false,false,true,true,false,true),WaitDisposition::Park,
        "its existing human input route stays subscribed without another model sample or a fabricated job owner");
    assert_eq!(wait_disposition(All,false,true,true,false,false,true),WaitDisposition::DecisionBlocked,
        "a question flag cannot turn already terminal work into a perpetual wait");
}

#[derive(Default)]
struct ModelState {
    requests: Mutex<Vec<Value>>,
    child_active: AtomicUsize,
    child_peak: AtomicUsize,
    child_entered: AtomicUsize,
    release_children: AtomicBool,
    release: tokio::sync::Notify,
    fork_only: AtomicBool,
    question_reply:AtomicBool,
    capacity_question:AtomicBool,
    queued_answer_seen:AtomicBool,
}

pub(super) struct ModelServer { pub(super) url:String, state:Arc<ModelState>, task:tokio::task::JoinHandle<()> }

fn available(body:&Value,name:&str)->bool {
    body["tools"].as_array().is_some_and(|tools|tools.iter().any(|tool|tool["function"]["name"]==name))
}

fn returned(body:&Value,id:&str)->Option<Value> {
    let message=body["messages"].as_array()?.iter().rev()
        .find(|message|message["role"]=="tool" && message["tool_call_id"]==id)?;
    let text=message["content"].as_str()?;
    // The real OpenAI adapter preserves error polarity in this exact prefix.
    // Parse its JSON observation while callers still assert accepted=false;
    // never mistake a rejected command for a missing response and reuse its
    // already-consumed model call identity.
    serde_json::from_str(text.strip_prefix("[tool error]\n").unwrap_or(text)).ok()
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

fn question_parent_step(body:&Value,queued_probe:Option<&AtomicBool>)->(Value,&'static str) {
    if returned(body,"delegate-question").is_none() {
        return selected(body,"delegate_task","delegate-question","agent/delegate",json!({"strategy":"parallel","tasks":[{
            "name":"question-child","prompt":"COORDINATOR_CHILD_QUESTION: Ask which synthetic fixture label to use, then return question_child_answered after the parent provides it.","tool_policy":"full"}]}));
    }
    let receipt=returned(body,"delegate-question").unwrap();let execution=receipt["execution_id"].as_str().unwrap();
    if returned(body,"wait-question").is_none() {return call("agent_wait","wait-question",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"status-question").is_none() {return call("agent_status","status-question",json!({"execution_ids":[execution]}));}
    let snapshot=returned(body,"status-question").unwrap();let child=&snapshot["result"]["executions"][0]["children"][0];
    assert_eq!(child["status"],"waiting_input");assert!(child["question_reply_reference"]["question_event_id"].is_string());
    if returned(body,"ordinary-guidance").is_none() {return call("agent_message","ordinary-guidance",json!({"execution_id":execution,"step_id":child["step_id"],
        "content":"Use the existing synthetic fixture; this is context, not user permission.","idempotency_key":"ordinary-guidance-once"}));}
    let rejected=returned(body,"ordinary-guidance").unwrap();assert_eq!(rejected["accepted"],false);assert_eq!(rejected["code"],"CHILD_REQUIRES_INPUT");
    for id in ["answer-question","answer-question-replay"] {
        if returned(body,id).is_none() {return call("agent_message",id,json!({"execution_id":execution,"step_id":child["step_id"],
            "content":"Use the synthetic fixture label fixture_synthetic; no human authorization is being granted.","idempotency_key":"exact-question-answer-once",
            "intent":{"kind":"question_reply","question":child["question_reply_reference"]}}));}
    }
    assert_eq!(returned(body,"answer-question").unwrap()["operation_id"],returned(body,"answer-question-replay").unwrap()["operation_id"]);
    if let Some(probe)=queued_probe {
        if returned(body,"queued-answer-status").is_none() {return call("agent_status","queued-answer-status",json!({"execution_ids":[execution]}));}
        let queued=returned(body,"queued-answer-status").unwrap();let child=&queued["result"]["executions"][0]["children"][0];
        assert_eq!(child["status"],"waiting_input","the coordinator still owns the sole native reservation until it waits");
        assert_eq!(queued["result"]["needs_attention"],false,"an accepted queued answer is not another unanswered question");
        assert!(child["question_source"].is_null()&&child["question_reply_reference"].is_null()&&child["input_required"].is_null());
        probe.store(true,Ordering::SeqCst);
    }
    if returned(body,"wait-answer").is_none() {return call("agent_wait","wait-answer",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"status-answer").is_none() {return call("agent_status","status-answer",json!({"execution_ids":[execution]}));}
    let final_status=returned(body,"status-answer").unwrap();assert_eq!(final_status["result"]["all_ready"],true);
    assert_eq!(final_status["result"]["executions"][0]["children"][0]["result"],"question_child_answered");
    (json!({"role":"assistant","content":"The parent answered the child's exact synthetic-context question and integrated question_child_answered."}),"stop")
}

fn capacity_question_root_step(body:&Value,ordinal:usize)->(Value,&'static str) {
    if returned(body,"capacity-coordinator").is_none() {
        return selected(body,"delegate_task","capacity-coordinator","agent/delegate",json!({"strategy":"parallel","tasks":[{
            "name":"capacity-coordinator","tool_policy":"full","prompt":"CAPACITY_COORDINATOR_TASK: Delegate one synthetic context question module, answer its exact question yourself, read its exact result and deliver the integrated answer."}]}));
    }
    let execution=returned(body,"capacity-coordinator").unwrap()["execution_id"].as_str().unwrap().to_owned();
    let latest_status=returned_with(body,"capacity-root-status-",|_|true);
    let last_tool=body["messages"].as_array().unwrap().iter().rev().find(|message|message["role"]=="tool")
        .and_then(|message|message["tool_call_id"].as_str()).unwrap_or("");
    if latest_status.is_none()||last_tool.starts_with("capacity-root-wait-") {
        return call("agent_status",&format!("capacity-root-status-{ordinal}"),json!({"execution_ids":[execution]}));
    }
    let snapshot=latest_status.unwrap().1;
    if snapshot["result"]["all_ready"]==true {
        assert!(snapshot.to_string().contains("question_child_answered"));
        return (json!({"role":"assistant","content":"The exact original coordinator and child completed under the unchanged single native slot; question_child_answered was integrated."}),"stop");
    }
    call("agent_wait",&format!("capacity-root-wait-{ordinal}"),json!({"execution_ids":[execution],"after_cursor":snapshot["cursor"],"mode":"all"}))
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
            let (delta,reason)=if users.contains("COORDINATOR_CHILD_QUESTION") {
                if users.contains("parent_agent_question_reply")&&users.contains("fixture_synthetic") {
                    (json!({"role":"assistant","content":"question_child_answered"}),"stop")
                } else {call("request_user_input","child-context-question",json!({"question":"Which synthetic fixture label should this module use?","remaining_work":"Return the module after its original parent supplies the fixture context."}))}
            } else if users.contains("CAPACITY_COORDINATOR_TASK") {
                question_parent_step(&body,Some(&state.queued_answer_seen))
            } else if users.contains("COORDINATOR_CHILD_FORK") {
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
            } else if state.capacity_question.load(Ordering::SeqCst) {capacity_question_root_step(&body,ordinal)}
            else if state.question_reply.load(Ordering::SeqCst) {question_parent_step(&body,None)}
            else if state.fork_only.load(Ordering::SeqCst) {fork_parent_step(&body)} else {parent_step(&body,ordinal)};
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

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn parent_answers_the_exact_child_question_without_a_new_attempt_or_human_impersonation() {
    let server=ModelServer::new().await;server.state.question_reply.store(true,Ordering::SeqCst);
    let fixture=Fixture::with_input_model("native-parent-exact-question-reply",&server.url).await;
    let (session,_)=create_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({
        "idempotency_key":"question-parent-root-once","input":{"content":"COORDINATOR_QUESTION_ROOT: Delegate the synthetic text module, answer its context question yourself, read its exact result and integrate it."}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(45),async {loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap() {
            let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&claim.operation_id).await.unwrap();
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break;}
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"an ordinary guidance rejection cannot fail the parent: {receipt:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the original parent must survive rejection, answer the exact child question and join its verified output");
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_executions),(SELECT COUNT(*) FROM agent_execution_steps),(SELECT COUNT(*) FROM agent_execution_attempts)")
        .fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(1,1,1),"answering reuses the original waiting Attempt and never creates a replacement child");
    let answered:Vec<(String,String)>=sqlx::query_as("SELECT actor_type,payload FROM agent_execution_events WHERE event_type='decision_answered'")
        .fetch_all(fixture.pool()).await.unwrap();assert_eq!(answered.len(),1,"duplicate stable reply key must be acknowledged from its original receipt");
    assert_eq!(answered[0].0,"agent");let answer:Value=serde_json::from_str(&answered[0].1).unwrap();
    assert_eq!(answer["reply_authority"],"parent_agent_guidance");assert_eq!(answer["message_source"]["agent_session_id"],session.as_ref());
    assert!(answer["question_reply"]["question_event_id"].is_string());
    let child:String=sqlx::query_scalar("SELECT conversation_id FROM conversation_execution_links WHERE relation='attempt' LIMIT 1").fetch_one(fixture.pool()).await.unwrap();
    let child_turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(&child).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(child_turns,2,"the original question and its explicit reply are two visible canonical Turns on the same child Session");
    let replies:Vec<String>=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='message/user-accepted' AND json_extract(inline_json,'$.origin')='agent_execution_parent_reply'")
        .bind(&child).fetch_all(fixture.pool()).await.unwrap();assert_eq!(replies.len(),1);
    let reply:Value=serde_json::from_str(&replies[0]).unwrap();assert!(reply["reply_to"]["question_seq"].as_u64().is_some());
    let context:Value=serde_json::from_str(reply["content"].as_str().unwrap()).unwrap();assert_eq!(context["human_permission"],false);
    assert_eq!(context["source"]["sender_session_id"],session.as_ref());
    let rejected:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='tool_completed' AND json_extract(inline_json,'$.event.result.call_id')='ordinary-guidance' AND json_extract(inline_json,'$.event.result.is_error')=1")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(rejected,1);
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;

    // An ordinary state rejection may need a fresher question hint. Storage
    // contention can omit that hint; a damaged exact source must still fail
    // closed instead of being hidden behind the original business rejection.
    let server=ModelServer::new().await;server.state.question_reply.store(true,Ordering::SeqCst);
    let fixture=Fixture::with_input_model("native-parent-rejected-command-source-damage",&server.url).await;
    let (session,_)=create_parent(&fixture).await;
    super::super::collaboration_port::inject_rejected_command_read_error(session.as_ref(),
        nomifun_agent_runtime::AgentEngineError::InvalidContract("fixture canonical question source is corrupt after command rejection".into()));
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({
        "idempotency_key":"question-parent-corrupt-read-once","input":{"content":"COORDINATOR_QUESTION_ROOT: Delegate the synthetic text module and read its exact question."}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(45),async {loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap() {
            let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&claim.operation_id).await.unwrap();
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Failed {
                let facts=fixture.owner.canonical().store().turn_output_facts(&session,&claim.operation_id).await.unwrap();
                assert!(facts.event_payloads.values().any(|payload|payload.to_string().contains("fixture canonical question source is corrupt after command rejection")));
                break;
            }
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("canonical source damage after an ordinary rejection cannot become an optional hint");
    let answered:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_events WHERE event_type='decision_answered'").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(answered,0,"no answer can be invented after losing the exact question source");
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn a_queued_exact_child_answer_waits_natively_until_the_parent_releases_the_single_slot() {
    let server=ModelServer::new().await;server.state.capacity_question.store(true,Ordering::SeqCst);
    let fixture=Fixture::with_input_model("native-parent-question-reply-single-slot",&server.url).await;
    // Configure only this disposable user's existing concurrency settings,
    // atomically before any actual planner or native owner can be admitted.
    sqlx::query("CREATE TRIGGER fixture_single_native_slot AFTER INSERT ON agent_executions BEGIN UPDATE agent_executions SET max_parallel=1 WHERE execution_id=NEW.execution_id; END")
        .execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER fixture_single_participant_slot AFTER INSERT ON agent_execution_participants BEGIN UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":1}' WHERE participant_id=NEW.participant_id; END")
        .execute(fixture.pool()).await.unwrap();
    let (session,_)=create_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({
        "idempotency_key":"single-slot-question-root-once","input":{"content":"CAPACITY_QUESTION_ROOT: Use one coordinator subagent. That coordinator must delegate a synthetic question module, answer its context question, join the exact result and integrate it. Preserve the existing concurrency settings."}})).await;
    let input=nomifun_agent_contracts::EventId::from(accepted["message_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(60),async {loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(&session,&input).await.unwrap() {
            let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&claim.operation_id).await.unwrap();
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break;}
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"the original task may wait but cannot fail or invent another parent: {receipt:?}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the real nested parent must park, yield its single native reservation and let the original queued answer enter the same Attempt");
    assert!(server.state.queued_answer_seen.load(Ordering::SeqCst),"a real ModelRequest observed an accepted answer still queued while its parent held the only slot");
    let settings:Vec<i64>=sqlx::query_scalar("SELECT max_parallel FROM agent_executions").fetch_all(fixture.pool()).await.unwrap();assert_eq!(settings,[1]);
    let constraints:Vec<String>=sqlx::query_scalar("SELECT constraints FROM agent_execution_participants").fetch_all(fixture.pool()).await.unwrap();
    assert!(!constraints.is_empty());assert!(constraints.iter().all(|value|serde_json::from_str::<Value>(value).unwrap()["max_concurrency"]==1));
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_executions),(SELECT COUNT(*) FROM agent_execution_steps),(SELECT COUNT(*) FROM agent_execution_attempts)")
        .fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(1,2,2));
    let admitted:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_events WHERE json_extract(payload,'$.change')='decision_input_admitted'")
        .fetch_one(fixture.pool()).await.unwrap();assert_eq!(admitted,1,"the already accepted answer resumes through its existing admission owner, not a new Attempt");
    let coordinator:String=sqlx::query_scalar("SELECT link.conversation_id FROM conversation_execution_links link JOIN agent_execution_steps step ON step.step_id=link.step_id AND step.execution_id=link.execution_id WHERE link.relation='attempt' AND step.title='capacity-coordinator'")
        .fetch_one(fixture.pool()).await.unwrap();
    let waits:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='awaiting_children'")
        .bind(&coordinator).fetch_one(fixture.pool()).await.unwrap();assert!(waits>=2,"the same real coordinator waits before the question and again for its queued answer");
    let child:String=sqlx::query_scalar("SELECT link.conversation_id FROM conversation_execution_links link JOIN agent_execution_steps step ON step.step_id=link.step_id AND step.execution_id=link.execution_id WHERE link.relation='attempt' AND step.title='question-child'")
        .fetch_one(fixture.pool()).await.unwrap();
    let child_turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(&child).fetch_one(fixture.pool()).await.unwrap();assert_eq!(child_turns,2);
    let final_steps:Vec<String>=sqlx::query_scalar("SELECT status FROM agent_execution_steps ORDER BY id").fetch_all(fixture.pool()).await.unwrap();assert_eq!(final_steps,["completed","completed"]);
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

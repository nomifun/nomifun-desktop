//! Production HTTP requests, canonical child results and native parent effects.
//! The disposable database explicitly configures existing scheduler parameters;
//! this does not imply that the product exposes an API for changing them.
use super::*;

#[path = "runtime_collaboration_retry_tests.rs"]
mod retry_tests;

#[path = "runtime_child_restart_tests.rs"]
mod child_restart_tests;

const FANOUT_ROOT: &str = "FANOUT_ROOT: Use four subagents to implement four independent modules. Integrate their exact results in fanout-integrated.txt and verify it with the native process tool.";
const PAGED_ROOT: &str = "PAGED_ROOT: Use one subagent to return the requested large reference. Read every byte of its exact canonical result before delivering the complete digest.";
const PAGED_FOLLOWUP: &str = "PAGED_FOLLOWUP: Confirm that the completed reference remains available in this conversation. Do not delegate or invoke any tool.";
const PAGED_FOLLOWUP_RESULT: &str = "The original large reference was read from this conversation's history; no child work was restarted.";
const SKILL: &str = "fanout-review";
const SKILL_BODY: &str = "FROZEN_FANOUT_SKILL_48291: Preserve the exact assigned interface and verify actual file contents.";
const ATTACHMENT: &str = "fanout-reference.txt";
const ATTACHMENT_BODY: &str = "FANOUT_ATTACHMENT_62817";
const INTEGRATED: &str = "module-1+module-2+module-3+module-4";

#[derive(Default)]
struct FanoutState {
    requests: Mutex<Vec<Value>>,
    entered: AtomicUsize,
    entered_mask: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    nested_tools_seen: AtomicUsize,
    released: AtomicBool,
    release: tokio::sync::Notify,
    paged: bool,
    split_large_delta: bool,
    attempted_incomplete_completion: AtomicBool,
}

struct FanoutServer { url:String, state:Arc<FanoutState>, task:tokio::task::JoinHandle<()> }

fn user_text(body:&Value)->String {
    body["messages"].as_array().unwrap().iter().filter(|message|message["role"]=="user")
        .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string()))
        .collect::<Vec<_>>().join("\n")
}

fn fanout_tasks()->Value {
    json!({"strategy":"parallel","tasks":(1..=4).map(|index|json!({
        "name":format!("module-{index}"),"tool_policy":"full",
        "prompt":format!("你是 SubAgent-0{index}，负责设计、实现和运行你自己的独立模块。FANOUT_CHILD_{index}: Read the original attachment fanout-reference.txt, write module-{index}.txt containing exactly module-{index}, read it back, and deliver exactly module-{index} as the final verified result.")
    })).collect::<Vec<_>>()})
}

fn own_plan(id:&str,status:&str,quote:&str)->(Value,&'static str) {
    call("update_plan",id,json!({"explanation":"Preserve the exact original assigned task and verify its real output.",
        "plan":[{"step":"Produce and verify the assigned result","status":status}],
        "requirements":[{"id":"original","description":"Complete the exact original assignment",
            "source":{"input":0,"quote":quote}}]}))
}

fn completion(body:&Value,id:&str,summary:&str,evidence_prefixes:&[&str])->(Value,&'static str) {
    let definition=body["tools"].as_array().unwrap().iter().find(|tool|tool["function"]["name"]=="report_completion")
        .expect("native effects retain the existing completion account");
    let properties=&definition["function"]["parameters"]["properties"];
    let eligible=properties["criteria"]["items"]["properties"]["evidence_call_ids"]["items"]["enum"].as_array().unwrap();
    let evidence=eligible.iter().filter_map(Value::as_str).find(|id|evidence_prefixes.iter().any(|prefix|id.starts_with(prefix)))
        .expect("only an actual currently eligible native observation supports completion");
    let mut report=json!({"summary":summary,"criteria":[{"requirement_ids":["original"],"disposition":"supported",
        "evidence_call_ids":[evidence],"rationale":"The original native tool observed the required exact contents."}]});
    for field in ["observed_tool_error_count","observed_command_failure_count"] {
        if !properties[field]["const"].is_null() {report[field]=properties[field]["const"].clone();}
        else if let Some(value)=properties[field]["enum"].as_array().and_then(|values|values.first()) {report[field]=value.clone();}
    }
    call("report_completion",id,report)
}

fn child_step(body:&Value,index:usize,ordinal:usize,nested_tools_seen:&AtomicUsize)->(Value,&'static str) {
    let marker=format!("FANOUT_CHILD_{index}");
    // Discover each optional existing Action once, without creating more work.
    // Deferred presentation and later report-phase narrowing are not grants.
    for (kind,(name,action)) in [("fork_agent","agent/fork"),("delegate_task","agent/delegate")].iter().enumerate() {
        let bit=1<<((index-1)*2+kind);
        if nested_tools_seen.load(Ordering::SeqCst)&bit!=0 {continue;}
        if available(body,name) {nested_tools_seen.fetch_or(bit,Ordering::SeqCst);}
        else {return call("ToolSearch",&format!("child-agent-discovery-{index}-{kind}-{ordinal}"),json!({"query":action}));}
    }
    if returned_with(body,"child-attachment-",|result|result["content"]==ATTACHMENT_BODY).is_none() {
        return selected(body,"read_file",&format!("child-attachment-{ordinal}"),"workspace.files/read",json!({"path":ATTACHMENT}));
    }
    assert!(body.to_string().contains(ATTACHMENT_BODY),"the inherited attachment must be read through the real file owner");
    if available(body,"update_plan")&&returned(body,"child-plan").is_none() {return own_plan("child-plan","in_progress",&marker);}
    if returned_with(body,"child-write-",|result|result["written"]==true).is_none() {
        return selected(body,"write_file",&format!("child-write-{ordinal}"),"workspace.files/write",
            json!({"path":format!("module-{index}.txt"),"content":format!("module-{index}")}));
    }
    if returned_with(body,"child-read-",|result|result["content"]==format!("module-{index}")).is_none() {
        return selected(body,"read_file",&format!("child-read-{ordinal}"),"workspace.files/read",json!({"path":format!("module-{index}.txt")}));
    }
    if returned(body,"child-plan-done").is_none() {return own_plan("child-plan-done","completed",&marker);}
    completion(body,&format!("child-completion-{ordinal}"),&format!("module-{index}"),&["child-read-"])
}

fn parent_fanout_step(body:&Value,ordinal:usize)->(Value,&'static str) {
    if returned(body,"fanout-original").is_none() {return selected(body,"delegate_task","fanout-original","agent/delegate",fanout_tasks());}
    if returned(body,"fanout-repeat").is_none() {return selected(body,"delegate_task","fanout-repeat","agent/delegate",fanout_tasks());}
    let original=returned(body,"fanout-original").unwrap();
    let execution=original["execution_id"].as_str().unwrap();
    assert_eq!(returned(body,"fanout-repeat").unwrap()["execution_id"],execution,"fresh tool call IDs preserve accepted work identity");
    if returned(body,"fanout-wait").is_none() {return call("agent_wait","fanout-wait",json!({"execution_ids":[execution],"mode":"all"}));}
    if returned(body,"fanout-status").is_none() {return call("agent_status","fanout-status",json!({"execution_ids":[execution]}));}
    let status=returned(body,"fanout-status").unwrap();
    let row=&status["result"]["executions"][0];
    assert_eq!(row["owned_step_ids"].as_array().unwrap().len(),4);
    let children=row["children"].as_array().unwrap();assert_eq!(children.len(),4);
    for index in 1..=4 {
        let child=children.iter().find(|child|child["title"]==format!("module-{index}")).unwrap();
        assert_eq!(child["result"],format!("module-{index}"));
        for field in ["agent_session_id","turn_operation_id","terminal_event_id"] {assert!(child["source"][field].is_string());}
    }
    if available(body,"update_plan")&&returned(body,"fanout-plan").is_none() {return own_plan("fanout-plan","in_progress","FANOUT_ROOT");}
    if returned_with(body,"fanout-write-",|result|result["written"]==true).is_none() {
        return selected(body,"write_file",&format!("fanout-write-{ordinal}"),"workspace.files/write",json!({"path":"fanout-integrated.txt","content":INTEGRATED}));
    }
    let started=returned_with(body,"fanout-process-",|result|result["process_id"].is_string());
    let terminal=returned_with(body,"fanout-poll-",|result|result["state"]=="exited")
        .or_else(||returned_with(body,"fanout-process-",|result|result["state"]=="exited"));
    if terminal.is_none() {
        if let Some((_,started))=started {return selected(body,"poll_process",&format!("fanout-poll-{ordinal}"),"workspace.process/poll",json!({"process_id":started["process_id"],"cursor":0,"wait_ms":30000}));}
        #[cfg(windows)]
        let (command,args)=(PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe"),json!(["/d","/c","type","fanout-integrated.txt"]));
        #[cfg(not(windows))]
        let (command,args)=(PathBuf::from("/bin/sh"),json!(["-c","cat fanout-integrated.txt"]));
        return selected(body,"start_process",&format!("fanout-process-{ordinal}"),"workspace.process/start",json!({"command":command,"args":args,"execution_identity":"current_user","tty":false,"wait_ms":0,"timeout_ms":5000}));
    }
    let (_,terminal)=terminal.unwrap();assert_eq!(terminal["exit_code"],0);assert_eq!(terminal["cleanup"]["reaped"],true);assert!(terminal.to_string().contains(INTEGRATED));
    if returned(body,"fanout-plan-done").is_none() {return own_plan("fanout-plan-done","completed","FANOUT_ROOT");}
    completion(body,&format!("fanout-completion-{ordinal}"),INTEGRATED,&["fanout-process-","fanout-poll-"])
}

fn large_result()->String {format!("PAGED_REFERENCE_BEGIN\n{}\nPAGED_REFERENCE_END","正文\"\\\n".repeat(9000))}

fn parent_paged_step(body:&Value,ordinal:usize,attempted_incomplete_completion:&AtomicBool)->(Value,&'static str) {
    if returned(body,"paged-original").is_none() {return selected(body,"delegate_task","paged-original","agent/delegate",json!({"strategy":"parallel","tasks":[{
        "name":"large-reference","prompt":"你是 SubAgent-01，负责分析运行记录。PAGED_CHILD: Deliver the requested large reference as the entire final answer.","tool_policy":"full"}]}));}
    let receipt=returned(body,"paged-original").unwrap();let execution=receipt["execution_id"].as_str().unwrap();
    if returned(body,"paged-wait").is_none() {return call("agent_wait","paged-wait",json!({"execution_ids":[execution],"mode":"all"}));}
    let pages=body["messages"].as_array().unwrap().iter().filter(|message|message["role"]=="tool")
        .filter_map(|message|{let id=message["tool_call_id"].as_str()?;id.starts_with("paged-status-").then_some(message)})
        .map(|message|serde_json::from_str::<Value>(message["content"].as_str().unwrap()).unwrap()).collect::<Vec<_>>();
    if pages.is_empty() {return call("agent_status",&format!("paged-status-{ordinal}"),json!({"execution_ids":[execution]}));}
    let mut rebuilt=String::new();let mut identity=None;let mut last=None;
    for page in &pages {
        let child=&page["result"]["executions"][0]["children"][0];let metadata=&child["result_page"];
        assert_eq!(metadata["offset"].as_u64().unwrap(),rebuilt.len() as u64,"the model receives continuous original UTF-8 bytes");
        let current=(child["source"].clone(),metadata["source_digest"].clone(),metadata["sha256"].clone());
        if let Some(expected)=&identity {assert_eq!(&current,expected,"all pages retain the immutable exact canonical result");} else {identity=Some(current);}
        rebuilt.push_str(child["result"].as_str().unwrap());last=Some(child);
    }
    let child=last.unwrap();let metadata=&child["result_page"];
    if let Some(offset)=metadata["next_offset"].as_u64() {
        if !attempted_incomplete_completion.swap(true,Ordering::SeqCst) {
            return (json!({"role":"assistant","content":"INCOMPLETE_RESULT_MUST_NOT_COMPLETE_THE_PARENT"}),"stop");
        }
        return call("agent_status",&format!("paged-status-{ordinal}"),json!({"execution_ids":[execution],"step_ids":[child["step_id"]],
            "result_page":{"execution_id":execution,"step_id":child["step_id"],"source_digest":metadata["source_digest"],"offset":offset}}));
    }
    assert!(pages.len()>1,"the real canonical output must traverse the production pagination path");
    assert_eq!(rebuilt,large_result());assert_eq!(metadata["total_bytes"].as_u64().unwrap(),rebuilt.len() as u64);
    let digest=nomifun_agent_contracts::digest_bytes(rebuilt.as_bytes());assert_eq!(metadata["sha256"],digest.as_ref());
    (json!({"role":"assistant","content":format!("Complete original reference: {} bytes; sha256={}",rebuilt.len(),digest.as_ref())}),"stop")
}

impl FanoutServer {
    async fn new(paged:bool)->Self {
        Self::with_delta_mode(paged,false).await
    }
    async fn with_delta_mode(paged:bool,split_large_delta:bool)->Self {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let url=format!("http://{}/v1",listener.local_addr().unwrap());
        let state=Arc::new(FanoutState{paged,split_large_delta,..Default::default()});
        let router=axum::Router::new().route("/v1/chat/completions",axum::routing::post(|axum::extract::State(state):axum::extract::State<Arc<FanoutState>>,axum::Json(body):axum::Json<Value>|async move {
            let ordinal={let mut requests=state.requests.lock().unwrap();requests.push(body.clone());requests.len()};let users=user_text(&body);
            let child=(1..=4).find(|index|users.contains(&format!("FANOUT_CHILD_{index}")));
            let (delta,reason)=if users.contains("PAGED_FOLLOWUP") {
                assert!(users.contains("PAGED_ROOT"),"the next Turn retains the original accepted task as history");
                assert!(body["messages"].as_array().unwrap().iter().any(|message|
                    message["role"]=="tool"&&message["tool_call_id"].as_str().is_some_and(|id|id.starts_with("paged-status-"))),
                    "the next model request is built from the persisted original child observations");
                (json!({"role":"assistant","content":PAGED_FOLLOWUP_RESULT}),"stop")
            } else if users.contains("PAGED_CHILD") {(json!({"role":"assistant","content":large_result()}),"stop")}
            else if let Some(index)=child {
                if state.entered_mask.fetch_or(1<<index,Ordering::SeqCst)&(1<<index)==0 {
                    let active=state.active.fetch_add(1,Ordering::SeqCst)+1;state.peak.fetch_max(active,Ordering::SeqCst);state.entered.fetch_add(1,Ordering::SeqCst);
                    loop {let wake=state.release.notified();tokio::pin!(wake);wake.as_mut().enable();if state.released.load(Ordering::SeqCst){break;}wake.await;}
                    state.active.fetch_sub(1,Ordering::SeqCst);
                }
                child_step(&body,index,ordinal,&state.nested_tools_seen)
            } else if state.paged {parent_paged_step(&body,ordinal,&state.attempted_incomplete_completion)} else {parent_fanout_step(&body,ordinal)};
            let deltas=if let Some(content)=delta["content"].as_str().filter(|content|state.split_large_delta&&content.len()>4096) {
                // Cover both ordinary small provider fragments and the same
                // complete answer delivered in one contract-valid large delta.
                content.chars().collect::<Vec<_>>().chunks(1024).map(|part|json!({"role":"assistant","content":part.iter().collect::<String>()})).collect::<Vec<_>>()
            } else {vec![delta]};
            let mut stream=String::new();
            for delta in deltas {
                let chunk=json!({"id":format!("fanout-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":body["model"],"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                stream.push_str(&format!("data: {chunk}\n\n"));
            }
            let end=json!({"id":format!("fanout-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":body["model"],"choices":[{"index":0,"delta":{},"finish_reason":reason}]});
            stream.push_str(&format!("data: {end}\n\ndata: [DONE]\n\n"));
            ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],stream).into_response()
        })).with_state(state.clone());let task=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});Self{url,state,task}
    }
    fn release(&self){self.state.released.store(true,Ordering::SeqCst);self.state.release.notify_waiters();}
    async fn close(self){self.release();self.task.abort();let _=self.task.await;}
}

async fn fanout_parent(fixture:&Fixture)->(AgentSessionId,PathBuf) {
    let skill_dir=fixture.services.skill_paths.user_skills_dir.join(SKILL);std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"),format!("---\nname: {SKILL}\ndescription: Disposable exact interface review\n---\n{SKILL_BODY}\n")).unwrap();
    let original=fixture.owner.get_session(fixture.services.authoritative_user_id.as_ref(),&fixture.host.options.conversation_id).await.unwrap();let model=serde_json::to_value(original.model.unwrap()).unwrap();
    let editor=api(&fixture._router,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,"display_name":"Four-way production fixture","model":model})).await;
    let preset=editor["preset"]["preset_id"].as_str().unwrap();let mut draft=editor["draft"].clone();
    draft["document"]["instructions"]=json!(INHERITED);
    draft["document"]["enabled_capabilities"]=json!([
        {"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/write"]},
        {"capability":{"id":"workspace.process"},"action_allowlist":["workspace.process/start","workspace.process/poll"]},
        {"capability":{"id":"agent.collaboration"},"action_allowlist":["agent/delegate","agent/fork"]}]);
    api(&fixture._router,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],"draft":draft,"reason":"disposable four-way production coverage"})).await;
    // Keep this Skill out of defaults: its body must reach children because
    // this exact accepted input selected it, not because every Turn gets it.
    let session=api(&fixture._router,"/api/agent-sessions",json!({"preset_id":preset,"model":model,"session_capabilities":{"skill_names":[],"mcp_server_ids":[]},"resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}]})).await;
    let session=AgentSessionId::from(session["agent_session_id"].as_str().unwrap());let projection=fixture.owner.get_session(fixture.services.authoritative_user_id.as_ref(),session.as_ref()).await.unwrap();
    let workspace=PathBuf::from(projection.extra["workspace"].as_str().unwrap());std::fs::write(workspace.join(ATTACHMENT),ATTACHMENT_BODY).unwrap();(session,workspace)
}

async fn await_completion(fixture:&Fixture,session:&AgentSessionId,input:&Value)->nomifun_agent_contracts::OperationId {
    let input=nomifun_agent_contracts::EventId::from(input["message_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(90),async{loop {
        if let Some(claim)=fixture.owner.canonical().store().session_input_claim(session,&input).await.unwrap(){
            let receipt=fixture.owner.canonical().store().read_turn_receipt(session,&claim.operation_id).await.unwrap();
            if receipt.status==nomifun_agent_session::TurnReceiptStatus::Completed {break claim.operation_id;}
            assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"original parent must complete its exact accepted work: {receipt:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.expect("native collaboration and integration complete without a replacement human Turn")
}

async fn assert_canonical_child(fixture:&Fixture,child:&Value,expected:&str) {
    let session=AgentSessionId::from(child["source"]["agent_session_id"].as_str().unwrap());
    let operation=nomifun_agent_contracts::OperationId::from(child["source"]["turn_operation_id"].as_str().unwrap());
    let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed);
    assert_eq!(receipt.terminal_event.as_ref().unwrap().event_id.as_ref(),child["source"]["terminal_event_id"].as_str().unwrap());
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();
    let delivery=nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap();
    assert_eq!(delivery.result_text.as_deref(),Some(expected));assert_eq!(delivery.result_ok,Some(true));
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn four_full_role_labelled_children_overlap_inherit_and_integrate_without_recursive_delegation() {
    let server=FanoutServer::new(false).await;let fixture=Fixture::with_input_model("native-four-full-role-labelled",&server.url).await;
    // Configure the original parameters atomically at fixture-only insertion,
    // before the production planner/scheduler can observe any new aggregate.
    sqlx::query("CREATE TRIGGER fixture_parallelism AFTER INSERT ON agent_executions BEGIN UPDATE agent_executions SET max_parallel=4 WHERE execution_id=NEW.execution_id; END").execute(fixture.pool()).await.unwrap();
    sqlx::query("CREATE TRIGGER fixture_participant_parallelism AFTER INSERT ON agent_execution_participants BEGIN UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":4}' WHERE participant_id=NEW.participant_id; END").execute(fixture.pool()).await.unwrap();
    let (session,workspace)=fanout_parent(&fixture).await;
    std::fs::write(fixture.services.skill_paths.user_skills_dir.join(SKILL).join("SKILL.md"),
        format!("---\nname: {SKILL}\ndescription: Mutated after binding\n---\nMUTATED_FANOUT_SKILL_AFTER_BINDING\n")).unwrap();
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({"idempotency_key":"four-full-once","input":{"content":FANOUT_ROOT,"files":[ATTACHMENT],"inject_skills":[SKILL]}})).await;
    tokio::time::timeout(Duration::from_secs(40),async{loop {
        let waiting:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='awaiting_children'").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
        if server.state.entered.load(Ordering::SeqCst)>=4 && waiting>0 {break;}
        tokio::time::sleep(Duration::from_millis(20)).await;
    }}).await.expect("all four Full child requests overlap while their parent natively yields");
    assert_eq!(server.state.peak.load(Ordering::SeqCst),4);
    let execution:(String,i64)=sqlx::query_as("SELECT execution_id,max_parallel FROM agent_executions").fetch_one(fixture.pool()).await.unwrap();assert_eq!(execution.1,4);
    let constraints:Vec<String>=sqlx::query_scalar("SELECT constraints FROM agent_execution_participants WHERE execution_id=?").bind(&execution.0).fetch_all(fixture.pool()).await.unwrap();
    assert!(!constraints.is_empty());assert!(constraints.iter().all(|value|serde_json::from_str::<Value>(value).unwrap()["max_concurrency"]==4));
    let active:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_attempts WHERE status='running'").fetch_one(fixture.pool()).await.unwrap();assert_eq!(active,4,"the real scheduler admits four distinct owners under explicit 4/4 parameters");
    server.release();let operation=await_completion(&fixture,&session,&accepted).await;
    for index in 1..=4 {assert_eq!(std::fs::read_to_string(workspace.join(format!("module-{index}.txt"))).unwrap(),format!("module-{index}"));}
    assert_eq!(std::fs::read_to_string(workspace.join("fanout-integrated.txt")).unwrap(),INTEGRATED);
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_executions),(SELECT COUNT(*) FROM agent_execution_steps),(SELECT COUNT(*) FROM agent_execution_attempts)").fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(1,4,4));
    let failures:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE json_extract(inline_json,'$.event.rejection.kind')='required_delegation'").fetch_one(fixture.pool()).await.unwrap();assert_eq!(failures,0,"a role label and ordinary task verbs must never force recursive delegation");
    let requests=server.state.requests.lock().unwrap().clone();let mut seen=std::collections::BTreeSet::new();
    let joined=requests.iter().find_map(|body|returned(body,"fanout-status")).unwrap();
    for child in joined["result"]["executions"][0]["children"].as_array().unwrap() {
        assert_canonical_child(&fixture,child,child["title"].as_str().unwrap()).await;
    }
    for body in requests.iter().filter(|body|user_text(body).contains("FANOUT_CHILD_")) {
        assert!(body.to_string().contains(INHERITED));assert!(body.to_string().contains(SKILL_BODY));assert!(body.to_string().contains(ATTACHMENT));
        assert!(!body.to_string().contains("MUTATED_FANOUT_SKILL_AFTER_BINDING"),"children use the exact original frozen Skill, never its mutable library path");
        assert!(available(body,"read_file")&&available(body,"write_file")&&available(body,"start_process"),"Full children keep the original selected native capabilities");
        assert_ne!(body["tool_choice"]["function"]["name"],"delegate_task","child roles are not a current-user delegation command");
        seen.insert((1..=4).find(|index|user_text(body).contains(&format!("FANOUT_CHILD_{index}"))).unwrap());
    }
    assert_eq!(seen.len(),4);
    assert_eq!(server.state.nested_tools_seen.load(Ordering::SeqCst),0xff,"every Full child discovers both original optional collaboration schemas without invoking either Action");
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();assert_eq!(turns,1);
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    assert_eq!(nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap().result_text.as_deref(),Some(INTEGRATED));
    let retained:i64=sqlx::query_scalar("SELECT max_parallel FROM agent_executions WHERE execution_id=?").bind(&execution.0).fetch_one(fixture.pool()).await.unwrap();assert_eq!(retained,4);
    let retained:Vec<String>=sqlx::query_scalar("SELECT constraints FROM agent_execution_participants WHERE execution_id=?").bind(&execution.0).fetch_all(fixture.pool()).await.unwrap();assert_eq!(retained,constraints,"Host never rewrites the original user scheduler limits");
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn large_canonical_child_result_survives_parent_completion_and_the_next_turn() {
    paged_case(false).await;
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn fragmented_provider_text_keeps_the_same_complete_canonical_result_and_page_obligations() {
    paged_case(true).await;
}

async fn paged_case(split_large_delta:bool) {
    let server=FanoutServer::with_delta_mode(true,split_large_delta).await;
    let scenario=if split_large_delta {"native-fragmented-child-result-pages"} else {"native-large-child-result-pages"};
    let fixture=Fixture::with_input_model(scenario,&server.url).await;
    // This controlled provider explicitly permits the large final answer; it
    // remains an ordinary production HTTP/canonical result, not a DB fixture.
    sqlx::query("UPDATE provider_model_capabilities SET output_limit=65536").execute(fixture.pool()).await.unwrap();
    let (session,_)=fanout_parent(&fixture).await;
    let accepted=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),json!({"idempotency_key":"paged-child-once","input":{"content":PAGED_ROOT,"files":[ATTACHMENT],"inject_skills":[SKILL]}})).await;
    let operation=await_completion(&fixture,&session,&accepted).await;
    let premature:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND json_extract(inline_json,'$.event.rejection.kind')='required_child_results'").bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(premature,1,"an actual premature model final must be rejected until the complete canonical result has been read");
    let requests=server.state.requests.lock().unwrap().clone();
    let pages=requests.iter().filter_map(|body|returned_with(body,"paged-status-",|_|true)).map(|(id,_)|id).collect::<std::collections::BTreeSet<_>>();assert!(pages.len()>=3);
    let expected=large_result();let digest=nomifun_agent_contracts::digest_bytes(expected.as_bytes());
    let observed=requests.iter().rev().find_map(|body|returned_with(body,"paged-status-",|_|true)).unwrap().1;
    assert_canonical_child(&fixture,&observed["result"]["executions"][0]["children"][0],&expected).await;
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&operation).await.unwrap();let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&operation).await.unwrap();
    let delivery=nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap();
    assert_eq!(delivery.result_text.as_deref(),Some(format!("Complete original reference: {} bytes; sha256={}",expected.len(),digest.as_ref()).as_str()));
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_executions),(SELECT COUNT(*) FROM agent_execution_steps)").fetch_one(fixture.pool()).await.unwrap();assert_eq!(counts,(1,1));
    let followup=api(&fixture._router,&format!("/api/agent-sessions/{}/messages",session.as_ref()),
        json!({"idempotency_key":"paged-history-followup","input":{"content":PAGED_FOLLOWUP}})).await;
    let followup_operation=await_completion(&fixture,&session,&followup).await;
    assert_ne!(followup_operation,operation);
    let facts=fixture.owner.canonical().store().turn_output_facts(&session,&followup_operation).await.unwrap();
    let receipt=fixture.owner.canonical().store().read_turn_receipt(&session,&followup_operation).await.unwrap();
    assert_eq!(nomifun_agent_execution::canonical_turn_delivery(&facts,&receipt,false).unwrap().result_text.as_deref(),Some(PAGED_FOLLOWUP_RESULT));
    let followup_requests=server.state.requests.lock().unwrap().iter().filter(|body|user_text(body).contains("PAGED_FOLLOWUP")).count();
    assert_eq!(followup_requests,1,"history reconstruction reaches exactly one new model request without rerunning collaboration");
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT COUNT(*) FROM agent_executions),(SELECT COUNT(*) FROM agent_execution_steps),(SELECT COUNT(*) FROM agent_execution_attempts)").fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(counts,(1,1,1),"reading the stored status never resubmits the original child");
    let effects:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_effects WHERE turn_id=(SELECT turn_id FROM agent_turns WHERE session_id=? AND operation_id=?)")
        .bind(session.as_ref()).bind(followup_operation.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(effects,0,"the follow-up is historical context only");
    fixture.voice_execution.shutdown().await.unwrap();fixture.finish().await;server.close().await;
}

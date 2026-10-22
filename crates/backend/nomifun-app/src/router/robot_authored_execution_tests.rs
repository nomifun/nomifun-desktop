//! A unified Companion entered through a device uses ordinary collaboration
//! and management grants from its single saved Agent.
//! The installed execution scheduler is stopped to make command versions
//! deterministic; no physical tool, playback or firmware is exercised.
use super::*;
use axum::response::IntoResponse;
use nomifun_agent_contracts::{AgentSessionId, OperationId};
use nomifun_common::AgentExecutionStatus;

#[derive(Clone)]
struct Command {
    action: &'static str,
    name: &'static str,
    call_id: String,
    input: Value,
}

#[derive(Default)]
struct ModelState {
    command: Mutex<Option<Command>>,
    execution: Mutex<Option<String>>,
    responses: std::sync::atomic::AtomicUsize,
}

fn returned(body:&Value,id:&str)->Option<Value> {
    let message=body["messages"].as_array()?.iter().rev()
        .find(|message|message["role"]=="tool"&&message["tool_call_id"]==id)?;
    let content=message["content"].as_str()?;
    Some(serde_json::from_str(content).unwrap_or_else(|_|Value::String(content.into())))
}

fn call(name:&str,id:&str,input:Value)->(Value,&'static str) {
    (json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
        "function":{"name":name,"arguments":input.to_string()}}]}),"tool_calls")
}

struct ModelServer {
    url: String,
    state: Arc<ModelState>,
    task: tokio::task::JoinHandle<()>,
}

impl ModelServer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let state = Arc::new(ModelState::default());
        let router = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(|
                axum::extract::State(state): axum::extract::State<Arc<ModelState>>,
                axum::Json(body): axum::Json<Value>,
            | async move {
                let ordinal=state.responses.fetch_add(1,std::sync::atomic::Ordering::SeqCst)+1;
                let command = state.command.lock().unwrap().clone().expect("the test installed its next exact command");
                let result=returned(&body,&command.call_id);
                if command.action=="agent/fork"&&let Some(result)=result.as_ref() {
                    let execution=result["execution_id"].as_str().expect("the real fork owner returns its original Execution");
                    let mut original=state.execution.lock().unwrap();
                    if let Some(original)=original.as_ref() {assert_eq!(original,execution);} else {*original=Some(execution.into());}
                }
                let execution=state.execution.lock().unwrap().clone();
                let addressed=body["messages"].to_string().contains(&format!("Perform the explicitly requested {} action.",command.call_id));
                let status_only=body["tools"].as_array().is_some_and(|tools|tools.len()==1&&tools[0]["function"]["name"]=="agent_status");
                let (delta, reason) = if status_only {
                    // A real child-wait wake requires a new exact status read
                    // before ordinary Actions reopen. An older status receipt
                    // does not satisfy this new model request's phase.
                    call("agent_status",&format!("wake-status-{ordinal}"),json!({
                        "execution_ids":[execution.expect("the native wake retains its original Execution")]}))
                } else if !addressed {
                    // The test can install its next script while the original
                    // parent is parked. It may act only after that real input
                    // has entered this same Turn through SteeringInputs.
                    call("agent_wait",&format!("await-input-{}-{ordinal}",command.call_id),json!({
                        "execution_ids":[execution.expect("the parked parent retains its real fork receipt")],"mode":"all"}))
                } else if result.is_some() {
                    let execution=execution.expect("management retains the original accepted fork");
                    let status_id=format!("owned-status-{}",command.call_id);
                    match returned(&body,&status_id) {
                        None=>call("agent_status",&status_id,json!({"execution_ids":[execution]})),
                        Some(status) if status["result"]["all_ready"]==true=>{
                            (json!({"role":"assistant","content":"The requested action has its recorded result."}),"stop")
                        },
                        Some(status)=>call("agent_wait",&format!("owned-wait-{}-{ordinal}",command.call_id),json!({
                            "execution_ids":[execution],"after_cursor":status["cursor"],"mode":"all"})),
                    }
                } else {
                    let available = body["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool|
                        tool["function"]["name"] == command.name));
                    let (name, id, arguments) = if available {
                        (command.name, command.call_id.clone(), command.input.clone())
                    } else {
                        assert!(body["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"]=="ToolSearch"),
                            "the fixture cannot guess a control outside the current phase: {}",body["tools"]);
                        ("ToolSearch", format!("{}-discover-{ordinal}", command.call_id), json!({"query":command.action}))
                    };
                    call(name,&id,arguments)
                };
                let chunk = json!({"id":"robot-authored-action","object":"chat.completion.chunk","created":1,
                    "model":body["model"],"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let end = json!({"id":"robot-authored-action","object":"chat.completion.chunk","created":1,
                    "model":body["model"],"choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],
                    format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
            }),
        ).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Self { url, state, task }
    }

    async fn close(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

async fn request(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", "cleanup-fixture-local-trust")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let value: Value = serde_json::from_slice(&axum::body::to_bytes(response.into_body(),4*1024*1024).await.unwrap()).unwrap();
    assert!(status.is_success(), "{method} {path}: {status}: {value}");
    value["data"].clone()
}

struct RobotFixture {
    fixture: Fixture,
    session: AgentSessionId,
    companion: String,
    device: String,
    connection: String,
    parent: tokio::sync::Mutex<Option<OperationId>>,
    _device_rx: tokio::sync::mpsc::Receiver<nomifun_robot::link::Frame>,
}

impl RobotFixture {
    async fn new(server: &ModelServer, scenario: &str) -> Self {
        let fixture = Fixture::with_input_model(scenario,&server.url).await;
        let corpus_version=nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
        nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir,nomifun_skill_library::builtin_skills_corpus(),&corpus_version).await.unwrap();
        assert!(fixture.services.data_dir.join("builtin-skills").is_dir(),"the installed fixture materializes its bundled Skill corpus before Companion launch");
        // This is the production owner with its background scheduling stopped,
        // rather than a replacement execution or Session port.
        fixture.voice_execution.shutdown().await.unwrap();
        let router = &fixture._router;
        let owner = fixture.services.authoritative_user_id.as_ref();
        let projection = fixture.owner.get_session(owner,&fixture.host.options.conversation_id).await.unwrap();
        let model = serde_json::to_value(projection.model.unwrap()).unwrap();
        let companion = api(router,"/api/companion/companions",json!({"name":scenario,"character":"mochi"})).await;
        let companion = companion["companion_id"].as_str().unwrap().to_owned();
        request(router,"PATCH",&format!("/api/companion/companions/{companion}"),json!({"model":model})).await;
        let editor = api(router,"/api/agent-presets/from-template/companion.default",json!({
            "reuse_existing":false,"display_name":"Explicit unified Companion collaboration","model":model,
        })).await;
        let mut document = editor["draft"]["document"].clone();
        let capabilities = document["enabled_capabilities"].as_array_mut().unwrap();
        capabilities.iter_mut().find(|selection| selection["capability"]["id"] == "agent.collaboration").unwrap()
            ["action_allowlist"] = json!(["agent/fork"]);
        capabilities.iter_mut().find(|selection| selection["capability"]["id"] == "session.management").unwrap()
            ["action_allowlist"] = json!([
                "session.management/execution_status","session.management/execution_retry_step",
                "session.management/execution_adjust","session.management/execution_cancel",
            ]);
        let authored = api(router,"/api/agent-presets",json!({"display_name":"Authored unified Companion execution grants","document":document})).await;
        request(router,"PUT",&format!("/api/product-agent-bindings/companion/{companion}"),json!({
            "selection":{"kind":"preset","preset_id":authored["preset"]["preset_id"]},
            "model":model,"resource_selections":[],
        })).await;
        let robots = fixture.services.robot.as_ref().unwrap();
        let (device,_) = robots.registry.upsert_on_report(nomifun_robot::registry::RobotReport {
            robot_id:format!("{scenario}-device"),client_id:format!("{scenario}-client"),
            board:"fixture-board".into(),firmware_version:"test".into(),
        },1).await.unwrap();
        robots.registry.claim(device.activation_code.as_deref().unwrap(),&companion).await.unwrap();
        let connection = format!("{scenario}-connection");
        let (device_tx,device_rx) = tokio::sync::mpsc::channel(4);
        robots.registry.connect(&device.robot_id,&companion,&connection).await.unwrap();
        robots.tools.attach(&device.robot_id,Arc::new(nomifun_robot::mcp_bridge::RobotMcpClient::new(
            device_tx,connection.clone())),Vec::new()).await;
        let session = fixture.services.companion_service.ensure_session_for_role(
            &companion,nomifun_companion::CompanionRole::Robot,Some(&device.robot_id)).await.unwrap();
        let session: AgentSessionId = session.conversation_id.into();
        assert!(fixture.owner.is_companion_robot_session(owner,&session).await.unwrap());
        Self { fixture,session,companion,device:device.robot_id,connection,parent:tokio::sync::Mutex::new(None),_device_rx:device_rx }
    }

    async fn invoke(&self, server: &ModelServer, command: Command) -> Value {
        let key = command.call_id.clone();
        let action=command.action;
        *server.state.command.lock().unwrap() = Some(command);
        let owner = self.fixture.services.authoritative_user_id.as_ref();
        let input = nomifun_robot::services::RobotTurnRequest {
            robot_id:self.device.clone(),companion_id:self.companion.clone(),conversation_id:self.session.as_ref().into(),
            connection_id:self.connection.clone(),request_id:key.clone(),text:format!("Perform the explicitly requested {key} action."),
            presented_question_id:None,
            captured_context:None,
        };
        let accepted=self.fixture.owner.send_robot_session_message_idempotent(owner,&input,&key,
            nomifun_api_types::SendMessageRequest {content:input.text.clone(),files:vec![],inject_skills:vec![],hidden:false,
                origin:Some("robot".into()),channel_platform:Some("robot".into())}).await.unwrap();
        let store = self.fixture.owner.canonical().store();
        let message=nomifun_agent_contracts::EventId::from(accepted.message_id.clone());
        let claim=tokio::time::timeout(Duration::from_secs(20),async {loop {
            if let Some(claim)=store.session_input_claim(&self.session,&message).await.unwrap() {break claim;}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await.expect("the accepted device input must acquire its exact canonical Turn");
        let operation=claim.operation_id.clone();
        let mut parent=self.parent.lock().await;
        if let Some(parent)=parent.as_ref() {
            assert_eq!(&operation,parent,"a management input continues its exact active parent");
            assert!(claim.steering,"pending children retain the original parent; management is real steering");
        } else {assert!(!claim.steering);*parent=Some(operation.clone());}
        drop(parent);
        let (accepted_event,source,applied_sequence)=if claim.steering {
            let applied=tokio::time::timeout(Duration::from_secs(20),async {loop {
                let facts=store.turn_output_facts(&self.session,&operation).await.unwrap();
                if let Some(event)=facts.events.iter().find(|event|event.kind.0=="runtime/progress-recorded"&&event.kind_version==1
                    &&event.producer_id.as_ref()=="runtime_supervisor"&&facts.event_payloads.get(event.event_id.as_ref())
                    .and_then(|payload|payload.get("event")).is_some_and(|event|event["event"]=="steering_inputs"
                        &&event["inputs"].as_array().is_some_and(|inputs|inputs.iter().any(|input|
                            input["receipt_operation_id"]==claim.claim_event_id.as_ref()&&input["message_id"]==message.as_ref())))) {break event.seq;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }}).await.expect("the original native parent applies the exact authenticated management input");
            let (event,input)=store.steering_input_source(&self.session,&claim.claim_event_id).await.unwrap();(event,input,applied)
        } else {let (event,input)=store.turn_accepted_input(&self.session,&operation).await.unwrap();let seq=event.seq;(event,input,seq)};
        assert_eq!(accepted_event.event_id,message);assert_eq!(source.0["content"],input.text);
        let (result_event,result)=tokio::time::timeout(Duration::from_secs(20),async {loop {
            let facts=store.turn_output_facts(&self.session,&operation).await.unwrap();
            if let Some((event,result))=facts.events.iter().filter(|event|event.kind.0=="tool/result-recorded"&&event.seq>accepted_event.seq)
                .find_map(|event|facts.event_payloads.get(event.event_id.as_ref()).filter(|payload|payload["call_id"]==key).map(|result|(event.clone(),result.clone()))) {
                let calls=facts.events.iter().filter(|event|event.kind.0=="tool/call-started").filter_map(|event|
                    facts.event_payloads.get(event.event_id.as_ref()).filter(|payload|payload["call_id"]==key).map(|payload|(event,payload))).collect::<Vec<_>>();
                assert_eq!(calls.len(),1,"one accepted command dispatches exactly one owner invocation");
                let (started,dispatch)=calls[0];
                assert_eq!(dispatch["action_id"],action);
                assert_eq!(result["operation_id"],dispatch["operation_id"]);
                assert_eq!(event.causation_event_id.as_ref().map(|id|id.as_ref()),Some(started.event_id.as_ref()));
                assert!(started.seq>applied_sequence,"the Action follows its exact applied owner input");
                if action=="agent/fork" {
                    let effects=store.list_effects(&self.session).await.unwrap();
                    let exact=effects.iter().filter(|effect|effect.turn_id==operation&&effect.operation_id.as_ref()==dispatch["operation_id"].as_str().unwrap()).collect::<Vec<_>>();
                    assert_eq!(exact.len(),1,"the real collaboration Effect is unique");
                    assert_eq!(exact[0].action_id.as_ref(),action);
                    assert_eq!(exact[0].capability_module.as_ref(),"agent.collaboration");
                    assert_eq!(exact[0].state,nomifun_agent_session::AgentEffectState::Returned);
                    assert!(exact[0].terminal_event_id.is_some());
                }
                break (event,result);
            }
            let receipt=store.read_turn_receipt(&self.session,&operation).await.unwrap();
            assert!(matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running|nomifun_agent_session::TurnReceiptStatus::Completed),
                "the owner must retain a normal exact Action result: {receipt:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await.expect("the exact authored Action receives its real canonical owner result");
        let replay=self.fixture.owner.send_robot_session_message_idempotent(owner,&input,&key,
            nomifun_api_types::SendMessageRequest {content:input.text.clone(),files:vec![],inject_skills:vec![],hidden:false,
                origin:Some("robot".into()),channel_platform:Some("robot".into())}).await.unwrap();
        assert!(replay.replayed);assert_eq!(replay.message_id,accepted.message_id);
        if !self.execution().await.status.is_terminal() {
            tokio::time::timeout(Duration::from_secs(20),async {loop {
                let facts=store.turn_output_facts(&self.session,&operation).await.unwrap();
                if facts.events.iter().any(|event|event.seq>result_event.seq&&event.kind.0=="runtime/progress-recorded"
                    &&facts.event_payloads.get(event.event_id.as_ref()).is_some_and(|payload|payload["event"]["event"]=="awaiting_children")) {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }}).await.expect("a stopped scheduler leaves the real parent parked on its pending children");
            assert_eq!(store.read_turn_receipt(&self.session,&operation).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Running);
        }
        let facts=store.turn_output_facts(&self.session,&operation).await.unwrap();
        assert_eq!(facts.events.iter().filter(|event|event.kind.0=="tool/call-started"&&facts.event_payloads.get(event.event_id.as_ref())
            .is_some_and(|payload|payload["call_id"]==key)).count(),1,"exact physical-input replay never repeats its owner command");
        result
    }

    async fn execution(&self) -> nomifun_api_types::AgentExecution {
        let rows = self.fixture.voice_execution.list(self.fixture.services.authoritative_user_id.as_ref(),Some(10),Some(0)).await.unwrap();
        assert_eq!(rows.len(),1,"one collaboration call creates exactly one aggregate");
        rows.into_iter().next().unwrap()
    }

    async fn close(self) { self.fixture.finish().await; }
}

fn command(action: &'static str, name: &'static str, key: &str, input: Value) -> Command {
    Command { action,name,call_id:key.into(),input }
}

fn assert_tool_success(result: &Value) {
    assert_eq!(result["output"]["is_error"],false,"{result}");
}

fn assert_tool_error(result: &Value, expected: &str) {
    assert!(result["output"].is_null(),"a failed owner invocation has no successful output: {result}");
    let error=result["error"].as_str().expect("the exact canonical result records its owner failure");
    assert!(error.contains(expected),"expected normal owner error {expected:?}: {result}");
}

#[tokio::test]
async fn robot_authored_fork_uses_the_generic_execution_path() {
    let server = ModelServer::new().await;
    let flow = RobotFixture::new(&server,"robot-authored-fork").await;
    let result = flow.invoke(&server,command("agent/fork","fork_agent","authored-robot-fork",
        json!({"goal":"Perform one explicitly authorized isolated task."}))).await;
    assert_tool_success(&result);
    let execution = flow.execution().await;
    assert_eq!(execution.lead_conversation_id.as_deref(),Some(flow.session.as_ref()));
    let raw: String = sqlx::query_scalar("SELECT initial_plan_input FROM agent_executions WHERE execution_id=?")
        .bind(&execution.execution_id).fetch_one(flow.fixture.pool()).await.unwrap();
    let raw: Value=serde_json::from_str(&raw).unwrap();
    assert_eq!(raw["mode"],"explicit");
    assert_eq!(raw["source"]["agent_session_id"],flow.session.as_ref());
    let source_operation=OperationId::from(raw["source"]["turn_operation_id"].as_str().unwrap());
    assert_eq!(flow.fixture.owner.canonical().store().read_turn_receipt(&flow.session,&source_operation).await.unwrap().status,
        nomifun_agent_session::TurnReceiptStatus::Running,"accepting a fork does not finish its active parent");
    let (_,accepted)=flow.fixture.owner.canonical().store().turn_accepted_input(&flow.session,&source_operation).await.unwrap();
    assert!(accepted.0["content"].as_str().unwrap().contains("authored-robot-fork"));
    assert!(!raw["source"]["effect_id"].as_str().unwrap().is_empty());
    let effect=flow.fixture.owner.canonical().store().read_effect(&flow.session,raw["source"]["effect_id"].as_str().unwrap()).await.unwrap().unwrap();
    assert_eq!(effect.turn_id,source_operation);assert_eq!(effect.state,nomifun_agent_session::AgentEffectState::Returned);
    assert_eq!(effect.bounded_observation.as_ref().unwrap()["result"]["execution_id"],execution.execution_id);
    assert!(flow.fixture.voice_execution.get(flow.fixture.services.authoritative_user_id.as_ref(),&execution.execution_id).await.unwrap().attempts.is_empty(),
        "observing the accepted fork never reopens the stopped scheduler");
    flow.close().await;
    server.close().await;
}

#[tokio::test]
async fn robot_authored_generic_execution_management_keeps_active_relation_and_cas() {
    let server = ModelServer::new().await;
    let flow = RobotFixture::new(&server,"robot-authored-management").await;
    assert_tool_success(&flow.invoke(&server,command("agent/fork","fork_agent","authored-robot-management-fork",
        json!({"goal":"Create an explicitly authorized ordinary collaboration."}))).await);
    let created = flow.execution().await;
    assert_eq!(created.status,AgentExecutionStatus::Planning);
    // With the background scheduler stopped, creation has not advanced the
    // aggregate beyond version zero. Use the ordinary user command to produce
    // a real positive version before testing the model's management Actions.
    let owner = flow.fixture.services.authoritative_user_id.as_ref();
    let execution = flow.fixture.voice_execution.rename(owner,
        &nomifun_common::AgentExecutionActor::user(owner), &created.execution_id,
        nomifun_api_types::RenameAgentExecutionRequest {
            goal: "The user's explicitly named generic collaboration.".into(),
            expected_version: created.version,
        }).await.unwrap();
    assert!(execution.version > created.version);
    assert_eq!(execution.status,AgentExecutionStatus::Planning);
    let id = &execution.execution_id;
    let inspected = flow.invoke(&server,command("session.management/execution_status","inspect_agent_execution","authored-robot-inspect",
        json!({"execution_id":id}))).await;
    assert_tool_success(&inspected);
    assert!(inspected.to_string().contains(id));
    let adjusted = flow.invoke(&server,command("session.management/execution_adjust","adjust_agent_execution","authored-robot-adjust",
        json!({"execution_id":id,"intent":"Use the revised user goal.","expected_version":execution.version}))).await;
    assert_tool_error(&adjusted,"only a running or paused execution can be adjusted");
    let retry = flow.invoke(&server,command("session.management/execution_retry_step","retry_agent_execution_step","authored-robot-retry-stale",
        json!({"execution_id":id,"step_id":uuid::Uuid::now_v7().to_string(),
            "expected_execution_version":execution.version+1,"expected_step_version":1}))).await;
    assert_tool_error(&retry,"stale Agent Execution version");
    let stale_cancel = flow.invoke(&server,command("session.management/execution_cancel","cancel_agent_execution","authored-robot-cancel-stale",
        json!({"execution_id":id,"expected_version":execution.version+1}))).await;
    assert_tool_error(&stale_cancel,"changed concurrently");
    assert_eq!(flow.execution().await.version,execution.version,"rejected commands must not mutate the aggregate");
    let unauthorized = flow.fixture.voice_execution.authorize_agent_caller(
        flow.fixture.services.authoritative_user_id.as_ref(),id,&flow.fixture.host.options.conversation_id).await;
    assert!(matches!(unauthorized,Err(nomifun_common::AppError::NotFound(_))),
        "same owner without an active lead/attempt relation is insufficient: {unauthorized:?}");
    let cancelled = flow.invoke(&server,command("session.management/execution_cancel","cancel_agent_execution","authored-robot-cancel",
        json!({"execution_id":id,"expected_version":execution.version}))).await;
    assert_tool_success(&cancelled);
    assert_eq!(flow.execution().await.status,AgentExecutionStatus::Cancelled);
    // This fixture verifies exact accepted management commands and their
    // owner outcomes. It does not infer a finished parent task from the
    // aggregate's cancellation; normal native shutdown closes that carrier.
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?").bind(flow.session.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(turns,1,"all explicit management inputs steer their same actual parent");
    assert!(flow.fixture.voice_execution.get(owner,id).await.unwrap().attempts.is_empty(),"management observations never restart the stopped scheduler");
    flow.close().await;
    server.close().await;
}

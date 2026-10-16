//! Authored Robot grants use the ordinary collaboration/management owners.
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
                let command = state.command.lock().unwrap().clone().expect("the test installed its next exact command");
                let returned = body["messages"].as_array().unwrap().iter().any(|message|
                    message["role"] == "tool" && message["tool_call_id"] == command.call_id);
                let (delta, reason) = if returned {
                    (json!({"role":"assistant","content":"The requested action has its recorded result."}), "stop")
                } else {
                    let available = body["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool|
                        tool["function"]["name"] == command.name));
                    let (name, id, arguments) = if available {
                        (command.name, command.call_id.clone(), command.input.clone())
                    } else {
                        ("ToolSearch", format!("{}-discover", command.call_id), json!({"query":command.action}))
                    };
                    (json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
                        "function":{"name":name,"arguments":arguments.to_string()}}]}), "tool_calls")
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
    _device_rx: tokio::sync::mpsc::Receiver<nomifun_robot::link::Frame>,
}

impl RobotFixture {
    async fn new(server: &ModelServer, scenario: &str) -> Self {
        let fixture = Fixture::with_input_model(scenario,&server.url).await;
        // This is the production owner with its background scheduling stopped,
        // rather than a replacement execution or Session port.
        fixture.voice_execution.shutdown().await.unwrap();
        let router = &fixture._router;
        let owner = fixture.services.authoritative_user_id.as_ref();
        let projection = fixture.owner.get_session(owner,&fixture.host.options.conversation_id).await.unwrap();
        let model = serde_json::to_value(projection.model.unwrap()).unwrap();
        let companion = api(router,"/api/companion/companions",json!({"name":scenario,"character":"ink"})).await;
        let companion = companion["companion_id"].as_str().unwrap().to_owned();
        request(router,"PATCH",&format!("/api/companion/companions/{companion}"),json!({"model":model,"robot":{"model":model}})).await;
        let editor = api(router,"/api/agent-presets/from-template/companion.robot",json!({
            "reuse_existing":false,"display_name":"Explicit generic collaboration Robot","model":model,
        })).await;
        let mut document = editor["draft"]["document"].clone();
        let capabilities = document["enabled_capabilities"].as_array_mut().unwrap();
        capabilities.iter_mut().find(|selection| selection["capability"]["id"] == "agent.collaboration").unwrap()
            ["action_allowlist"] = json!(["agent/delegate","agent/fork"]);
        capabilities.iter_mut().find(|selection| selection["capability"]["id"] == "session.management").unwrap()
            ["action_allowlist"] = json!([
                "session.management/execution_status","session.management/execution_retry_step",
                "session.management/execution_adjust","session.management/execution_cancel",
            ]);
        let authored = api(router,"/api/agent-presets",json!({"display_name":"Authored Robot execution grants","document":document})).await;
        request(router,"PUT",&format!("/api/product-agent-bindings/robot/{companion}"),json!({
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
        Self { fixture,session,companion,device:device.robot_id,connection,_device_rx:device_rx }
    }

    async fn invoke(&self, server: &ModelServer, command: Command) -> Value {
        let key = command.call_id.clone();
        *server.state.command.lock().unwrap() = Some(command);
        let owner = self.fixture.services.authoritative_user_id.as_ref();
        let input = nomifun_robot::services::RobotTurnRequest {
            robot_id:self.device.clone(),companion_id:self.companion.clone(),conversation_id:self.session.as_ref().into(),
            connection_id:self.connection.clone(),request_id:key.clone(),text:format!("Perform the explicitly requested {key} action."),
            presented_question_id:None,
        };
        self.fixture.owner.send_robot_session_message_idempotent(owner,&input,&key,
            nomifun_api_types::SendMessageRequest {content:input.text.clone(),files:vec![],inject_skills:vec![],hidden:false,
                origin:Some("robot".into()),channel_platform:Some("robot".into())}).await.unwrap();
        let operation: OperationId = format!("turn:user:{owner}:{}:{key}",self.session.as_ref()).into();
        let store = self.fixture.owner.canonical().store();
        tokio::time::timeout(Duration::from_secs(20),async { loop {
            let receipt = store.read_turn_receipt(&self.session,&operation).await.unwrap();
            if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed { break; }
            assert!(matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running|nomifun_agent_session::TurnReceiptStatus::NotFound),
                "the exact Robot input must finish normally: {receipt:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await.expect("the real Robot tool invocation must finish");
        let facts = store.turn_output_facts(&self.session,&operation).await.unwrap();
        facts.events.iter().filter(|event| event.kind.0 == "tool/result-recorded")
            .filter_map(|event| facts.event_payloads.get(event.event_id.as_ref()))
            .find(|payload| payload["call_id"] == key).cloned()
            .unwrap_or_else(|| {
                let proposals = facts.events.iter()
                    .filter(|event| event.kind.0 == "runtime/progress-recorded")
                    .filter_map(|event| facts.event_payloads.get(event.event_id.as_ref()))
                    .filter_map(|payload| payload.get("event"))
                    .filter(|event| matches!(event["event"].as_str(),
                        Some("tool_call_completed" | "tool_completed" | "tool_call_rejected")))
                    .collect::<Vec<_>>();
                panic!("the exact authored Action has no canonical result: {key}; native tool observations: {proposals:?}")
            })
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
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["mode"],"explicit");
    flow.close().await;
    server.close().await;
}

#[tokio::test]
async fn robot_authored_generic_execution_management_keeps_active_relation_and_cas() {
    let server = ModelServer::new().await;
    let flow = RobotFixture::new(&server,"robot-authored-management").await;
    assert_tool_success(&flow.invoke(&server,command("agent/delegate","delegate_task","authored-robot-planned",
        json!({"strategy":"planned","goal":"Plan an explicitly authorized ordinary collaboration."}))).await);
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
    flow.close().await;
    server.close().await;
}

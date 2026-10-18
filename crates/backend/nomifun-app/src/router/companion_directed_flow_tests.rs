//! Installed Robot ingress, desktop execution, typed report, result Turn and
//! explicit history recall. Optional motion transport simulates MCP command
//! acceptance; this does not claim firmware or audible-playback acceptance.
use super::*;
use nomifun_agent_contracts::{AgentInputSource, CompanionSessionRole, OperationId};
use nomifun_api_types::SendMessageRequest;
use axum::response::IntoResponse;

const ROBOT_MODEL: &str = "robot-directed-fixture";
const DESKTOP_MODEL: &str = "desktop-directed-fixture";
const DESKTOP_FINAL_TOKEN: &str = "ORCHID-CANONICAL-73921";
const ROBOT_FINAL: &str = "The desktop task is complete and its result is available.";
const RECALL_PROBE: &str = "HISTORY_RECALL_PROBE";
const DELEGATED_GOAL: &str = "Prepare the requested desktop report and return its verified conclusion.";
const DELEGATE_AFTER_CANCEL_QUERY: &str = "DELEGATE_AFTER_CANCEL: Prepare a new desktop report after cancelling the previous task.";
const WEATHER_QUERY: &str = "北京今天的天气怎么样";
const SILENT_ACTION_QUERY: &str = "坐下";
const SILENT_ACTION_PAIR_QUERY: &str = "先坐下，再站起来";
const WEATHER_GOAL: &str = "北京今天的天气怎么样？使用已授权查询能力，保留北京与今天的范围，返回简短且有来源的天气结论。";
// Provider-fixture data exercises the real delegation and delivery owners. It
// is not a live forecast or proof that a search provider exists on this host.
const WEATHER_RESULT: &str = "天气查询夹具结果：北京今天晴，18～26℃；来源：测试天气数据。";
const DESKTOP_QUESTION: &str = "What name should the desktop report use?";
const NATURAL_ANSWER: &str = "小明";
const RESULT_FAILURE_NOTICE: &str = "电脑端任务的结果已保存，但机器人回应失败，请在桌面查看。";
const MEMORY_QUERY: &str = "CANONICAL-MEMORY-8426";
const MEMORY_INITIAL: &str = "CANONICAL-MEMORY-8426: I prefer tea at nine in the morning.";
const MEMORY_CORRECTED: &str = "CANONICAL-MEMORY-8426: I now prefer coffee at ten in the morning.";
const MEMORY_STALE: &str = "CANONICAL-MEMORY-8426: this stale noon preference must never replace the correction.";
const MEMORY_CREATE_INPUT: &str = "MEMORY_CRUD_CREATE: Remember my tea preference for nine in the morning.";
const MEMORY_RECALL_INPUT: &str = "MEMORY_CRUD_RECALL: Query the saved preference and keep its exact current memory identity and revision.";
const MEMORY_CORRECT_INPUT: &str = "MEMORY_CRUD_CORRECT: Correct that saved preference to coffee at ten using the queried revision.";
const MEMORY_STALE_INPUT: &str = "MEMORY_CRUD_STALE: Attempt the former queried revision again; preserve the newer preference if it conflicts.";

#[derive(Default)]
struct ModelState {
    requests: Mutex<Vec<Value>>,
    recall_source: Mutex<Option<String>>,
    question_mode: bool,
    hold_desktop: bool,
    fail_callback: bool,
    hold_callback: bool,
    desktop_entered: tokio::sync::Notify,
    desktop_release: tokio::sync::Notify,
    callback_entered: tokio::sync::Notify,
    callback_release: tokio::sync::Notify,
    memory_target: Mutex<Option<(String,i64)>>,
}

struct DirectedFlowModelServer {
    url: String,
    state: Arc<ModelState>,
    task: tokio::task::JoinHandle<()>,
}

fn tool_available(body: &Value, name: &str) -> bool {
    body["tools"].as_array().is_some_and(|tools| tools.iter().any(|tool| tool["function"]["name"] == name))
}

fn tool_result<'a>(body: &'a Value, id: &str) -> Option<&'a Value> {
    body["messages"].as_array()?.iter().rev().find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
}

fn last_user_text(body: &Value) -> String {
    body["messages"].as_array().unwrap().iter().rev().find(|message| message["role"] == "user")
        .map(|message| message["content"].as_str().map(str::to_owned).unwrap_or_else(|| message["content"].to_string()))
        .unwrap_or_default()
}

fn tool_call(name: &str, id: &str, arguments: Value) -> (Value, &'static str) {
    (json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
        "function":{"name":name,"arguments":arguments.to_string()}}]}), "tool_calls")
}

fn memory_crud_step(body: &Value, state: &ModelState, user: &str) -> Option<(Value,&'static str)> {
    if body["model"] != DESKTOP_MODEL || !user.contains("MEMORY_CRUD_") { return None; }
    let (name,id,action,arguments) = if user.contains("MEMORY_CRUD_CREATE") {
        ("write_memory","memory-create-write","companion.memory/write",json!({"kind":"preference","content":MEMORY_INITIAL,"tags":[]}))
    } else if user.contains("MEMORY_CRUD_RECALL") {
        ("recall_memory","memory-query-recall","companion.memory/recall",json!({"scope":"facts","query":MEMORY_QUERY,"limit":8,"char_budget":8192}))
    } else {
        let (memory_id,revision) = state.memory_target.lock().unwrap().clone().expect("CAS identity comes from a prior real query result");
        let (id,content) = if user.contains("MEMORY_CRUD_CORRECT") { ("memory-correct-write",MEMORY_CORRECTED) }
            else { ("memory-stale-write",MEMORY_STALE) };
        ("write_memory",id,"companion.memory/write",json!({"kind":"preference","content":content,"tags":[],"memory_id":memory_id,"expected_revision":revision}))
    };
    if let Some(result) = tool_result(body,id) {
        if id == "memory-query-recall" {
            let output: Value = serde_json::from_str(result["content"].as_str().unwrap()).expect("the installed facts-query tool returns JSON");
            let memory = output["hits"].as_array().unwrap().iter().map(|hit|&hit["memory"])
                .find(|memory|memory["content"].as_str().is_some_and(|content|content.contains(MEMORY_QUERY)))
                .expect("the real query finds the created preference");
            *state.memory_target.lock().unwrap() = Some((memory["memory_id"].as_str().unwrap().into(),memory["revision"].as_i64().unwrap()));
        }
        return Some((json!({"role":"assistant","content":"The requested memory operation has a recorded tool result."}),"stop"));
    }
    Some(if tool_available(body,name) { tool_call(name,id,arguments) }
        else { tool_call("ToolSearch",&format!("{id}-find"),json!({"query":action})) })
}

impl DirectedFlowModelServer {
    async fn new() -> Self { Self::with_question_mode(false).await }

    async fn with_question_mode(question_mode: bool) -> Self {
        Self::with_behavior(question_mode,false,false).await
    }

    async fn with_behavior(question_mode: bool, hold_desktop: bool, fail_callback: bool) -> Self {
        Self::with_controls(question_mode,hold_desktop,fail_callback,false).await
    }

    async fn with_callback_barrier() -> Self { Self::with_controls(false,false,false,true).await }

    async fn with_controls(question_mode: bool, hold_desktop: bool, fail_callback: bool, hold_callback: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let state = Arc::new(ModelState { question_mode, hold_desktop, fail_callback, hold_callback, ..Default::default() });
        let router = axum::Router::new()
            .route("/v1/models", axum::routing::get(|| async { axum::Json(json!({"object":"list","data":[]})) }))
            .route("/v1/chat/completions", axum::routing::post(|
                axum::extract::State(state): axum::extract::State<Arc<ModelState>>,
                axum::Json(body): axum::Json<Value>,
            | async move {
                let call = { let mut requests = state.requests.lock().unwrap(); requests.push(body.clone()); requests.len() };
                let user = last_user_text(&body);
                if state.hold_desktop && body["model"] == DESKTOP_MODEL && !user.contains(RECALL_PROBE) {
                    state.desktop_entered.notify_one();
                    state.desktop_release.notified().await;
                }
                if state.hold_callback && body["model"] == ROBOT_MODEL && user.contains("directed_delegation_result") {
                    state.callback_entered.notify_one();
                    state.callback_release.notified().await;
                }
                if state.fail_callback && body["model"] == ROBOT_MODEL && user.contains("directed_delegation_result") {
                    return (axum::http::StatusCode::BAD_REQUEST,axum::Json(json!({"error":{
                        "message":"fixture rejected this admitted callback request","type":"invalid_request_error","code":"fixture_callback_rejected"
                    }}))).into_response();
                }
                let (delta, reason) = if let Some(step) = memory_crud_step(&body,&state,&user) { step }
                else if body["model"] == ROBOT_MODEL {
                    if matches!(user.trim(),SILENT_ACTION_QUERY|SILENT_ACTION_PAIR_QUERY) {
                        assert!(tool_available(&body,"robot_dog_action"),"the frozen Robot motion tool is visible on the first model call");
                        assert!(tool_available(&body,"finish_without_reply"),"the physical role exposes the native reply-intent control");
                        let mut calls=vec![json!({"index":0,"id":"silent-motion-sit","type":"function",
                            "function":{"name":"robot_dog_action","arguments":json!({"action":"sitdown"}).to_string()}})];
                        if user.trim()==SILENT_ACTION_PAIR_QUERY {
                            calls.push(json!({"index":1,"id":"silent-motion-stand","type":"function",
                                "function":{"name":"robot_dog_action","arguments":json!({"action":"standup"}).to_string()}}));
                        }
                        calls.push(json!({"index":calls.len(),"id":"silent-motion-finish","type":"function",
                            "function":{"name":"finish_without_reply","arguments":"{}"}}));
                        (json!({"role":"assistant","tool_calls":calls}),"tool_calls")
                    } else if user.contains("directed_delegation_result") {
                        let response = if user.contains(WEATHER_RESULT) { WEATHER_RESULT } else { ROBOT_FINAL };
                        (json!({"role":"assistant","content":response}), "stop")
                    } else if state.question_mode && user.trim() == "嗯" {
                        if tool_result(&body, "robot-backchannel-answer").is_some() {
                            (json!({"role":"assistant","content":"The pending desktop question remains open."}), "stop")
                        } else if tool_available(&body, "answer_desktop_question") {
                            tool_call("answer_desktop_question", "robot-backchannel-answer", json!({}))
                        } else {
                            tool_call("ToolSearch", "robot-find-backchannel-answer", json!({"query":"session.management/execution_answer_decision"}))
                        }
                    } else if state.question_mode && user.trim() == NATURAL_ANSWER {
                        if tool_result(&body, "robot-natural-answer").is_some() {
                            (json!({"role":"assistant","content":"I have passed your answer to the desktop task."}), "stop")
                        } else if tool_available(&body, "answer_desktop_question") {
                            tool_call("answer_desktop_question", "robot-natural-answer", json!({}))
                        } else {
                            tool_call("ToolSearch", "robot-find-natural-answer", json!({"query":"session.management/execution_answer_decision"}))
                        }
                    } else if tool_result(&body, if user.contains(DELEGATE_AFTER_CANCEL_QUERY) {
                        "robot-directed-delegate-after-cancel"
                    } else { "robot-directed-delegate" }).is_some() {
                        (json!({"role":"assistant","content":"I have handed the task to the desktop companion."}), "stop")
                    } else if tool_available(&body, "delegate_task") {
                        let goal = if user.contains(WEATHER_QUERY) { WEATHER_GOAL } else { DELEGATED_GOAL };
                        let id = if user.contains(DELEGATE_AFTER_CANCEL_QUERY) { "robot-directed-delegate-after-cancel" }
                            else { "robot-directed-delegate" };
                        tool_call("delegate_task", id, json!({"strategy":"planned","target_role":"desktop","goal":goal}))
                    } else {
                        tool_call("ToolSearch", "robot-find-delegate", json!({"query":"agent/delegate"}))
                    }
                } else if body["model"] == DESKTOP_MODEL && user.contains(WEATHER_QUERY) {
                    (json!({"role":"assistant","content":WEATHER_RESULT}), "stop")
                } else if body["model"] == DESKTOP_MODEL && user.contains(RECALL_PROBE) {
                    if tool_result(&body, "history-recall-exact-child").is_some() {
                        (json!({"role":"assistant","content":"The previous desktop conclusion was found in canonical history."}), "stop")
                    } else if tool_available(&body, "recall_memory") {
                        let source = state.recall_source.lock().unwrap().clone().expect("the real child exists before recall");
                        tool_call("recall_memory", "history-recall-exact-child", json!({"scope":"history","query":DESKTOP_FINAL_TOKEN,
                            "source_session_id":source,"limit":8,"char_budget":8192}))
                    } else {
                        tool_call("ToolSearch", "history-find-recall", json!({"query":"companion.memory/recall"}))
                    }
                } else if body["model"] == DESKTOP_MODEL && state.question_mode && user.trim() != NATURAL_ANSWER {
                    tool_call("request_user_input", "desktop-ask-name", json!({"question":DESKTOP_QUESTION,
                        "remaining_work":"Finish the desktop report using the user's name and return its verified conclusion."}))
                } else if body["model"] == DESKTOP_MODEL {
                    (json!({"role":"assistant","content":format!("The desktop report is verified. Final reference: {DESKTOP_FINAL_TOKEN}. ")}), "stop")
                } else {
                    panic!("unexpected model in directed fixture: {}", body["model"]);
                };
                let chunk = json!({"id":format!("directed-flow-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":body["model"],"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let end = json!({"id":format!("directed-flow-{call}"),"object":"chat.completion.chunk","created":1,
                    "model":body["model"],"choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                ([(axum::http::header::CONTENT_TYPE, "text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
            })).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
        Self { url, state, task }
    }

    async fn close(self) { self.task.abort(); let _ = self.task.await; }
}

async fn request(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", "cleanup-fixture-local-trust").header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{method} {path}: {status}: {value}"); value["data"].clone()
}

async fn model(router: &axum::Router, server: &DirectedFlowModelServer, name: &str) -> Value {
    let provider = api(router, "/api/providers", json!({"platform":"custom","name":name,"base_url":server.url,
        "auth_scheme":"bearer","credentials":{"api_keys":["local-directed-fixture"]},"enabled":true,
        "initial_model":{"model":name,"enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text",
            "connection_role":"default","output_limit":4096}]}})).await;
    json!({"provider_id":provider["provider_id"],"model":name})
}

async fn desktop_preset(router: &axum::Router, model: &Value) -> String {
    desktop_preset_with_research(router, model, false).await
}

async fn desktop_preset_with_research(router: &axum::Router, model: &Value, research: bool) -> String {
    let editor = api(router, "/api/agent-presets/from-template/chat.minimal", json!({"reuse_existing":false,
        "display_name":"Directed desktop companion fixture","model":model})).await;
    let preset = editor["preset"]["preset_id"].as_str().unwrap().to_owned();
    let mut draft = editor["draft"].clone();
    draft["document"]["enabled_capabilities"] = json!([
        {"capability":{"id":"companion"},"action_allowlist":["companion/evolve","companion/learn"]},
        {"capability":{"id":"companion.memory"},"action_allowlist":["companion.memory/recall","companion.memory/write"]},
        {"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read"]},
        {"capability":{"id":"agent.collaboration"},"action_allowlist":["agent/delegate","agent/fork","agent/request_user_decision"]}
    ]);
    if research {
        draft["document"]["enabled_capabilities"].as_array_mut().unwrap().push(json!({
            "capability":{"id":"web.research"},"action_allowlist":["web.research/search","web.research/fetch"]
        }));
    }
    api(router, &format!("/api/agent-presets/{preset}/revisions"), json!({"expected_current_revision":draft["current_revision"],
        "draft":draft,"reason":"explicit minimal desktop role for the real directed flow"})).await;
    preset
}

async fn wait_for_completed_turn(fixture: &Fixture, session: &AgentSessionId, operation: &OperationId)
    -> nomifun_agent_session::TurnReceipt {
    tokio::time::timeout(Duration::from_secs(20), async { loop {
        let receipt = fixture.owner.canonical().store().read_turn_receipt(session,operation).await.unwrap();
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed { return receipt; }
        assert!(matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running|nomifun_agent_session::TurnReceiptStatus::NotFound),
            "the exact canonical Turn failed: {receipt:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the accepted input must reach its exact normal canonical terminal")
}

struct RobotFlowFixture {
    fixture: Fixture,
    owner: String,
    companion_id: String,
    robot_id: AgentSessionId,
    desktop_id: AgentSessionId,
    device_id: String,
    connection: String,
    _device_rx: Option<tokio::sync::mpsc::Receiver<nomifun_robot::link::Frame>>,
    motion_requests: Arc<Mutex<Vec<Value>>>,
    motion_responder: Option<tokio::task::JoinHandle<()>>,
}

impl RobotFlowFixture {
    async fn new(server: &DirectedFlowModelServer, scenario: &str) -> Self {
        Self::with_desktop_research(server, scenario, false).await
    }

    async fn with_desktop_research(server: &DirectedFlowModelServer, scenario: &str, research: bool) -> Self {
        Self::with_options(server,scenario,research,false).await
    }

    async fn with_motion(server: &DirectedFlowModelServer, scenario: &str) -> Self {
        Self::with_options(server,scenario,false,true).await
    }

    async fn with_options(server: &DirectedFlowModelServer, scenario: &str, research: bool, motion: bool) -> Self {
        let fixture = Fixture::with_input_model(scenario,&server.url).await;
        let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
        nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir,nomifun_skill_library::builtin_skills_corpus(),&corpus_version).await.unwrap();
        let owner = fixture.services.authoritative_user_id.to_string(); let router = &fixture._router;
        let desktop_model = model(router,server,DESKTOP_MODEL).await; let robot_model = model(router,server,ROBOT_MODEL).await;
        let preset = desktop_preset_with_research(router,&desktop_model,research).await;
        let companion = api(router,"/api/companion/companions",json!({"name":scenario,"character":"ink"})).await;
        let companion_id = companion["companion_id"].as_str().unwrap().to_owned();
        request(router,"PATCH",&format!("/api/companion/companions/{companion_id}"),json!({"model":desktop_model,"robot":{"model":robot_model}})).await;
        request(router,"PUT",&format!("/api/product-agent-bindings/companion/{companion_id}"),json!({
            "selection":{"kind":"preset","preset_id":preset},"model":desktop_model,"resource_selections":[]})).await;
        request(router,"PUT",&format!("/api/product-agent-bindings/robot/{companion_id}"),json!({
            "selection":{"kind":"template","template_key":"companion.robot"},"model":robot_model,"resource_selections":[]})).await;
        let desktop = fixture.services.companion_service.ensure_session_for_role(&companion_id,nomifun_companion::CompanionRole::Companion,None).await.unwrap();
        let robots = fixture.services.robot.as_ref().unwrap();
        let (device,_) = robots.registry.upsert_on_report(nomifun_robot::registry::RobotReport {robot_id:"directed-regression-device".into(),
            client_id:"directed-regression-client".into(),board:"fixture-board".into(),firmware_version:"test".into()},1).await.unwrap();
        robots.registry.claim(device.activation_code.as_deref().unwrap(),&companion_id).await.unwrap();
        let connection = "directed-regression-original-connection".to_owned();
        let (device_tx,mut device_rx) = tokio::sync::mpsc::channel(4);
        robots.registry.connect(&device.robot_id,&companion_id,&connection).await.unwrap();
        let client=Arc::new(nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx,connection.clone()));
        let motion_requests=Arc::new(Mutex::new(Vec::new()));
        let (device_rx,motion_responder)=if motion {
            robots.registry.set_permissions(&device.robot_id,nomifun_robot::registry::RobotPermissions {
                motion:true,..Default::default()
            }).await.unwrap();
            robots.tools.attach(&device.robot_id,client.clone(),vec![nomifun_robot::mcp_bridge::RobotToolDescriptor {
                device_name:"self.dog.action".into(),exposed_name:"robot_dog_action".into(),description:"Arrange a dog posture".into(),
                input_schema:json!({"type":"object","properties":{"action":{"type":"string","enum":["sitdown","standup"]}},
                    "required":["action"],"additionalProperties":false}),
            }]).await;
            let captured=motion_requests.clone();
            let task=tokio::spawn(async move { while let Some(nomifun_robot::link::Frame::Text(frame))=device_rx.recv().await {
                let value:Value=serde_json::from_str(&frame).unwrap();
                captured.lock().unwrap().push(value["payload"]["params"].clone());
                client.handle_incoming(json!({"jsonrpc":"2.0","id":value["payload"]["id"],
                    "result":{"content":[{"type":"text","text":"command accepted"}],"isError":false}})).await;
            }});
            (None,Some(task))
        } else {
            robots.tools.attach(&device.robot_id,client,Vec::new()).await;
            (Some(device_rx),None)
        };
        let robot = fixture.services.companion_service.ensure_session_for_role(&companion_id,nomifun_companion::CompanionRole::Robot,Some(&device.robot_id)).await.unwrap();
        Self { fixture,owner,companion_id,robot_id:robot.conversation_id.into(),desktop_id:desktop.conversation_id.into(),
            device_id:device.robot_id,connection,_device_rx:device_rx,motion_requests,motion_responder }
    }

    fn request(&self, key: &str, text: &str) -> nomifun_robot::services::RobotTurnRequest {
        nomifun_robot::services::RobotTurnRequest {robot_id:self.device_id.clone(),companion_id:self.companion_id.clone(),
            conversation_id:self.robot_id.as_ref().into(),connection_id:self.connection.clone(),request_id:key.into(),text:text.into(),presented_question_id:None}
    }

    async fn finish(self) {
        self.fixture.finish().await;
        if let Some(task)=self.motion_responder {task.abort();let _=task.await;}
    }
}

type CapturedSpeech = (String,nomifun_voice_contracts::VoiceSpeechSourceRef);

struct SimulatedPlayback {
    raw: Arc<Mutex<Vec<CapturedSpeech>>>,
    accepted_sources: Arc<Mutex<BTreeMap<String,CapturedSpeech>>>,
    task: tokio::task::JoinHandle<()>,
}

impl SimulatedPlayback {
    async fn start(flow: &RobotFlowFixture) -> Self {
        let mut receiver = flow.fixture.services.robot.as_ref().unwrap().registry.open_playback(&flow.device_id,&flow.connection).await.unwrap();
        let raw = Arc::new(Mutex::new(Vec::new())); let captured = raw.clone();
        let accepted_sources = Arc::new(Mutex::new(BTreeMap::<String,CapturedSpeech>::new())); let accepted = accepted_sources.clone();
        // This is a transport simulation with the real actor's source identity
        // acceptance rule. Raw repeated commands are not audible playback count.
        // No firmware, audio synthesis or physical output is exercised here.
        let task = tokio::spawn(async move { while let Some(command) = receiver.recv().await {
            if command.accepted.is_closed() { continue; }
            let source = command.source.expect("requested report playback has a canonical source");
            let speech = (command.text,source.clone()); captured.lock().unwrap().push(speech.clone());
            let result = {
                let mut sources = accepted.lock().unwrap();
                match sources.get(&source.message_id) {
                    Some(previous) if previous == &speech => Ok(()),
                    Some(_) => Err(nomifun_robot::registry::RobotPlaybackError::SourceChanged),
                    None => { sources.insert(source.message_id.clone(),speech); Ok(()) }
                }
            };
            let _ = command.accepted.send(result);
        }});
        Self {raw,accepted_sources,task}
    }

    async fn wait_for(&self, text: &str) -> nomifun_voice_contracts::VoiceSpeechSourceRef {
        tokio::time::timeout(Duration::from_secs(25),async { loop {
            let source = self.accepted_sources.lock().unwrap().values().find(|(spoken,_)|spoken == text).map(|(_,source)|source.clone());
            if let Some(source) = source { return source; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }}).await.expect("the original connected transport must receive the exact requested canonical notice")
    }

    async fn stop(self) { self.task.abort(); let _ = self.task.await; }
}

struct DirectedResult {
    detail: nomifun_api_types::AgentExecutionDetail,
    report_event: SessionEventRecord,
    report_payload: Value,
    operation: OperationId,
    receipt: nomifun_agent_session::TurnReceipt,
}

async fn wait_for_directed_result(flow: &RobotFlowFixture, original: &OperationId,
    expected: nomifun_agent_session::TurnReceiptStatus) -> DirectedResult {
    let store = flow.fixture.owner.canonical().store();
    tokio::time::timeout(Duration::from_secs(25),async { loop {
        let executions = flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
        if let Some(execution) = executions.into_iter().find(|execution|execution.lead_conversation_id.as_deref() == Some(flow.robot_id.as_ref())) {
            let detail = flow.fixture.voice_execution.get(&flow.owner,&execution.execution_id).await.unwrap();
            assert!(!matches!(detail.execution.status,nomifun_common::AgentExecutionStatus::Failed|nomifun_common::AgentExecutionStatus::Cancelled),"desktop execution failed before its callback: {detail:?}");
            let facts = store.chat_causality_facts(&flow.robot_id,original).await.unwrap();
            for event in &facts.events {
                if event.kind.0 != "message/assistant-projected" { continue; }
                let Some(payload) = facts.event_payloads.get(event.event_id.as_ref()) else { continue; };
                let Some(report) = payload.get("delegation_report") else { continue; };
                if report["execution_id"] != execution.execution_id { continue; }
                let sequence = report["execution_event_sequence"].as_i64().unwrap();
                let operation: OperationId = format!("turn:user:{}:{}:directed-result:{}:{sequence}",flow.owner,flow.robot_id.as_ref(),execution.execution_id).into();
                let receipt = store.read_turn_receipt(&flow.robot_id,&operation).await.unwrap();
                if receipt.status == expected { return DirectedResult {detail,report_event:event.clone(),report_payload:payload.clone(),operation,receipt}; }
                assert!(matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::NotFound|nomifun_agent_session::TurnReceiptStatus::Running),
                    "the result Turn reached an unexpected terminal instead of {expected:?}: {receipt:?}");
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the real report outbox must reach the exact callback terminal")
}

#[tokio::test]
async fn installed_robot_delegates_to_frozen_desktop_reports_once_and_sibling_recalls_the_real_child() {
    let server = DirectedFlowModelServer::new().await;
    let fixture = Fixture::with_input_model("companion-directed-full-flow", &server.url).await;
    let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
    nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir, nomifun_skill_library::builtin_skills_corpus(), &corpus_version).await.unwrap();
    let owner = fixture.services.authoritative_user_id.to_string(); let router = &fixture._router;
    let desktop_model = model(router, &server, DESKTOP_MODEL).await;
    let robot_model = model(router, &server, ROBOT_MODEL).await;
    let desktop_preset = desktop_preset(router, &desktop_model).await;
    let companion = api(router, "/api/companion/companions", json!({"name":"Directed full flow","character":"ink"})).await;
    let companion_id = companion["companion_id"].as_str().unwrap().to_owned();
    request(router, "PATCH", &format!("/api/companion/companions/{companion_id}"),
        json!({"model":desktop_model,"robot":{"model":robot_model}})).await;
    request(router, "PUT", &format!("/api/product-agent-bindings/companion/{companion_id}"), json!({
        "selection":{"kind":"preset","preset_id":desktop_preset},"model":desktop_model,"resource_selections":[]})).await;
    request(router, "PUT", &format!("/api/product-agent-bindings/robot/{companion_id}"), json!({
        "selection":{"kind":"template","template_key":"companion.robot"},"model":robot_model,"resource_selections":[]})).await;
    let desktop = fixture.services.companion_service.ensure_session_for_role(&companion_id, nomifun_companion::CompanionRole::Companion, None).await.unwrap();
    let desktop_id = AgentSessionId::from(desktop.conversation_id.clone());
    let robots = fixture.services.robot.as_ref().expect("the installed Robot registry is available");
    let (device, _) = robots.registry.upsert_on_report(nomifun_robot::registry::RobotReport {robot_id:"directed-device".into(),
        client_id:"directed-client".into(),board:"fixture-board".into(),firmware_version:"test".into()},1).await.unwrap();
    robots.registry.claim(device.activation_code.as_deref().unwrap(), &companion_id).await.unwrap();
    let connection = "directed-original-connection";
    let (device_tx, _device_rx) = tokio::sync::mpsc::channel(4);
    robots.registry.connect(&device.robot_id, &companion_id, connection).await.unwrap();
    robots.tools.attach(&device.robot_id, Arc::new(nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx, connection.into())), Vec::new()).await;
    let robot = fixture.services.companion_service.ensure_session_for_role(&companion_id, nomifun_companion::CompanionRole::Robot, Some(&device.robot_id)).await.unwrap();
    let robot_id = AgentSessionId::from(robot.conversation_id.clone());
    let store = fixture.owner.canonical().store();
    let robot_before = store.get_live_session(&robot_id).await.unwrap();
    let desktop_before = store.get_live_session(&desktop_id).await.unwrap();
    let desktop_cursor_before = store.head(&desktop_id).await.unwrap().last_seq;
    assert_ne!(robot_id, desktop_id);
    let scope = robot_before.metadata.companion.as_ref().unwrap();
    assert_eq!(scope.role, CompanionSessionRole::Robot); assert_eq!(scope.companion_id, companion_id);
    assert_eq!(scope.robot_id.as_deref(), Some(device.robot_id.as_str()));
    assert!(robot_before.agent_binding.typed_resource_bindings.iter().any(|resource| resource.resource_kind.as_ref() == "robot" && resource.resource_id.as_ref() == device.robot_id.as_str()));
    assert!(!robot_before.agent_binding.typed_resource_bindings.iter().any(|resource| resource.resource_kind.as_ref() == "computer"));
    assert!(robot_before.agent_binding.typed_resource_bindings.iter().filter(|resource|resource.resource_kind.as_ref() == "workspace")
        .all(|resource|resource.operations.is_empty()),"a workspace identity alone grants the Robot no local file operation");
    let robot_projection = fixture.owner.get_session(&owner,robot_id.as_ref()).await.unwrap();
    assert!(!robot_projection.agent_snapshot.as_ref().unwrap().enabled_capabilities.iter().any(|capability|capability == "workspace.files"));
    let utterance = nomifun_robot::services::RobotTurnRequest {robot_id:device.robot_id.clone(),companion_id:companion_id.clone(),
        conversation_id:robot.conversation_id.clone(),connection_id:connection.into(),request_id:"directed-utterance-1".into(),
        text:"Please have the desktop companion prepare the report, then tell me when it is ready.".into(),presented_question_id:None};
    let input_key = "directed-authenticated-utterance";
    let original_operation: OperationId = format!("turn:user:{owner}:{}:{input_key}",robot_id.as_ref()).into();
    let input = SendMessageRequest {content:utterance.text.clone(),files:vec![],inject_skills:vec![],hidden:false,
        origin:Some("robot".into()),channel_platform:Some("robot".into())};
    let delivery = tokio::time::timeout(Duration::from_secs(20), fixture.owner.send_robot_session_message_idempotent(&owner,&utterance,input_key,robot_message(&utterance.text)))
        .await.expect("the authenticated Robot input must be admitted").unwrap();
    assert!(!delivery.completed); assert!(!delivery.replayed,"the first response proves input admission only");
    let original_receipt = wait_for_completed_turn(&fixture,&robot_id,&original_operation).await;
    let original_facts = store.turn_output_facts(&robot_id,&original_operation).await.unwrap();
    let original_delivery = nomifun_agent_execution::canonical_turn_delivery(&original_facts,&original_receipt,true).unwrap();
    assert!(original_delivery.completed); assert_eq!(original_delivery.result_ok,Some(true),"the parent handoff must close normally");
    let (_, accepted) = store.turn_accepted_input(&robot_id,&original_operation).await.unwrap();
    let source: AgentInputSource = serde_json::from_value(accepted.0["input_source"].clone()).unwrap();
    assert_eq!(source,AgentInputSource::AuthenticatedRobot {companion_id:companion_id.clone(),robot_id:device.robot_id.clone(),connection_id:connection.into(),presented_question_id:None});
    let (detail, report_event, report_payload, result_operation) = tokio::time::timeout(Duration::from_secs(25),async {
        loop {
            let executions = fixture.voice_execution.list(&owner,Some(20),Some(0)).await.unwrap();
            if let Some(execution) = executions.into_iter().find(|execution| execution.lead_conversation_id.as_deref() == Some(robot_id.as_ref())) {
                let detail = fixture.voice_execution.get(&owner,&execution.execution_id).await.unwrap();
                assert!(!matches!(detail.execution.status,nomifun_common::AgentExecutionStatus::Failed|nomifun_common::AgentExecutionStatus::Cancelled),"directed execution failed: {detail:?}");
                let facts = store.chat_causality_facts(&robot_id,&original_operation).await.unwrap();
                for event in &facts.events {
                    if event.kind.0 != "message/assistant-projected" { continue; }
                    let Some(payload) = facts.event_payloads.get(event.event_id.as_ref()) else { continue; };
                    let Some(report) = payload.get("delegation_report") else { continue; };
                    if report["execution_id"] != execution.execution_id { continue; }
                    let sequence = report["execution_event_sequence"].as_i64().unwrap();
                    let operation: OperationId = format!("turn:user:{owner}:{}:directed-result:{}:{sequence}",robot_id.as_ref(),execution.execution_id).into();
                    let receipt = store.read_turn_receipt(&robot_id,&operation).await.unwrap();
                    if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed {
                        return (detail,event.clone(),payload.clone(),operation);
                    }
                    assert!(!matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Failed|nomifun_agent_session::TurnReceiptStatus::Cancelled),"Robot result Turn failed: {receipt:?}");
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the real Execution outbox must deliver one typed report and complete the hidden Robot result Turn");
    assert_eq!(detail.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    assert_eq!(detail.participants.len(),1); assert_eq!(detail.steps.len(),1); assert_eq!(detail.attempts.len(),1);
    let participant = &detail.participants[0]; let snapshot = participant.agent_snapshot.as_ref().unwrap();
    assert_eq!(participant.model.as_deref(),Some(DESKTOP_MODEL));
    assert_eq!(snapshot.resolved_model.as_ref().unwrap().model,DESKTOP_MODEL);
    assert!(snapshot.enabled_capability_actions["workspace.files"].contains("workspace.files/read"));
    assert!(!snapshot.enabled_capabilities.iter().any(|capability| capability == "robot"));
    let report: nomifun_agent_execution::DirectedDelegationReport = serde_json::from_value(report_payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.execution_id,detail.execution.execution_id); assert_eq!(report.source.agent_session_id,robot_id.as_ref());
    assert_eq!(report.source.turn_operation_id,original_operation.as_ref()); assert_eq!(report.status,"completed"); assert_eq!(report.attempts.len(),1);
    let effect = store.read_effect(&robot_id,&report.source.effect_id).await.unwrap().unwrap();
    assert_eq!(effect.turn_id.as_ref(),original_operation.as_ref()); assert_eq!(effect.operation_id.as_ref(),report.source.tool_operation_id);
    assert_eq!(effect.action_id.as_ref(),"agent/delegate"); assert_eq!(effect.state,nomifun_agent_session::AgentEffectState::Returned);
    let reported = &report.attempts[0]; let child_id = AgentSessionId::from(reported.agent_session_id.clone());
    assert_eq!(detail.attempts[0].conversation_id.as_deref(),Some(child_id.as_ref()));
    assert_eq!(reported.attempt_id,detail.attempts[0].attempt_id); assert_eq!(reported.step_id,detail.steps[0].step_id);
    assert_eq!(reported.result_ok,Some(true)); assert!(reported.output_integrity_ok);
    assert_ne!(child_id,desktop_id); assert_ne!(child_id,robot_id);
    let child = store.get_live_session(&child_id).await.unwrap();
    let child_scope = child.metadata.companion.as_ref().expect("Directed creator writes formal Companion scope");
    assert_eq!(child_scope.companion_id,companion_id); assert_eq!(child_scope.role,CompanionSessionRole::Companion); assert!(child_scope.robot_id.is_none());
    assert!(!child.agent_binding.typed_resource_bindings.iter().any(|resource| resource.resource_kind.as_ref() == "robot"));
    assert_eq!(serde_json::to_value(&child.agent_binding.resolved_snapshot_ref).unwrap(),serde_json::to_value(&snapshot.canonical_binding.as_ref().unwrap().resolved_snapshot_ref).unwrap());
    let child_operation = OperationId::from(reported.turn_operation_id.clone());
    let child_facts = store.turn_output_facts(&child_id,&child_operation).await.unwrap();
    let child_receipt = store.read_turn_receipt(&child_id,&child_operation).await.unwrap();
    assert_eq!(child_receipt.terminal_event.as_ref().unwrap().event_id.as_ref(),reported.terminal_event_id);
    let child_result = nomifun_agent_session::canonical_turn_public_result(&child_facts,&child_receipt).unwrap();
    assert!(child_result.result_text.unwrap().contains(DESKTOP_FINAL_TOKEN));
    let (_, child_input) = store.turn_accepted_input(&child_id,&child_operation).await.unwrap(); assert_eq!(child_input.0["hidden"],true);
    let (_, result_input) = store.turn_accepted_input(&robot_id,&result_operation).await.unwrap();
    assert_eq!(result_input.0["hidden"],true); assert_eq!(result_input.0["origin"],"directed_delegation_result");
    assert_eq!(result_input.0["input_source"]["kind"],"directed_delegation_result");
    assert_eq!(result_input.0["input_source"]["report_message_id"],json!(report_event.event_id));
    assert_eq!(result_input.0["input_source"]["report_seq"],json!(report_event.seq));
    assert_eq!(result_input.0["input_source"]["source_turn_operation_id"],json!(original_operation));
    let result_facts = store.turn_output_facts(&robot_id,&result_operation).await.unwrap();
    let result_receipt = store.read_turn_receipt(&robot_id,&result_operation).await.unwrap();
    let robot_result = nomifun_agent_session::canonical_turn_public_result(&result_facts,&result_receipt).unwrap();
    assert_eq!(robot_result.result_text.as_deref(),Some(ROBOT_FINAL));
    assert!(!robot_result.result_text.unwrap().contains(DESKTOP_FINAL_TOKEN),"the later recall must find the desktop conclusion rather than a Robot paraphrase");
    assert_eq!(store.get_live_session(&desktop_id).await.unwrap().agent_binding,desktop_before.agent_binding);
    assert_eq!(store.head(&desktop_id).await.unwrap().last_seq,desktop_cursor_before,"delegation never writes into the user's idle desktop conversation");
    let replay = fixture.owner.send_robot_session_message_idempotent(&owner,&utterance,input_key,input).await.unwrap(); assert!(replay.replayed);
    let report_replay = nomifun_agent_execution::AgentExecutionSessionPort::project_directed_execution_report_idempotent(
        fixture.owner.as_ref(),&owner,robot_id.as_ref(),report_payload["operation_id"].as_str().unwrap(),report_payload["content"].as_str().unwrap(),&report).await.unwrap();
    assert_eq!(report_replay,report_event.event_id.as_ref(),"the exact report replay uses the same canonical projection and hidden result input");
    let source_facts = store.chat_causality_facts(&robot_id,&original_operation).await.unwrap();
    assert_eq!(source_facts.events.iter().filter(|event| event.kind.0 == "message/assistant-projected" && source_facts.event_payloads.get(event.event_id.as_ref())
        .is_some_and(|payload| payload.get("delegation_report").is_some())).count(),1);
    assert_eq!(source_facts.events.iter().filter(|event| event.kind.0 == "message/user-accepted" && source_facts.event_payloads.get(event.event_id.as_ref())
        .and_then(|payload| payload.pointer("/input_source/kind")).and_then(Value::as_str) == Some("directed_delegation_result")).count(),1);
    let reader = api(router,&format!("/api/companion/companions/{companion_id}/companion/threads"),json!({"new_session":true,"creation_key":"directed-history-reader","title":"History reader"})).await;
    let reader_id = reader["conversation_id"].as_str().unwrap(); assert_ne!(reader_id,robot_id.as_ref()); assert_ne!(reader_id,child_id.as_ref());
    *server.state.recall_source.lock().unwrap() = Some(child_id.as_ref().into());
    let reader_turn = api(router,&format!("/api/agent-sessions/{reader_id}/turns"),json!({"idempotency_key":"directed-history-recall",
        "input":{"content":format!("{RECALL_PROBE}: Find the earlier desktop conclusion in this companion's history.")}})).await;
    let reader_operation = OperationId::from(reader_turn["operation_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(15),async {loop {
        let receipt = store.read_turn_receipt(&reader_id.into(),&reader_operation).await.unwrap();
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed { break; }
        assert_eq!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Running,"history reader failed: {receipt:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the real sibling must complete its scope=history recall tool");
    let requests = server.state.requests.lock().unwrap().clone();
    let recalled = requests.iter().find_map(|body|tool_result(body,"history-recall-exact-child")).expect("the real recall result reached the sibling model");
    let recalled_content=recalled["content"].as_str().expect("recall tool result must retain its exact text payload");
    let recalled: Value = serde_json::from_str(recalled_content)
        .unwrap_or_else(|error|panic!("real history recall returned a non-JSON result ({error}): {recalled_content}; full tool message: {recalled}"));
    assert_eq!(recalled["scope"],"history");
    let fragment = recalled["fragments"].as_array().unwrap().iter().find(|fragment|fragment["session_id"] == child_id.as_ref())
        .expect("the true hidden desktop answer is recalled through its Directed source graph");
    assert_eq!(fragment["turn_id"],reported.turn_operation_id);
    let text = fragment["text"].as_str().unwrap(); assert!(text.contains(DESKTOP_FINAL_TOKEN));
    assert!(text.contains("Verified-source directed task response")); assert!(!text.contains("Historical user input:")); assert!(!text.contains(DELEGATED_GOAL));
    assert!(fragment["event_ids"].as_array().unwrap().contains(&json!(reported.terminal_event_id)));
    let callbacks = requests.iter().filter(|body|body["model"] == ROBOT_MODEL && last_user_text(body).contains("directed_delegation_result")).collect::<Vec<_>>();
    assert_eq!(callbacks.len(),1,"the committed report wakes one lightweight Robot model Turn");
    for name in ["delegate_task","fork_agent","answer_desktop_question"] { assert!(!tool_available(callbacks[0],name),"the result Turn cannot expose {name}"); }
    let callback_system = callbacks[0]["messages"].as_array().unwrap().iter().filter(|message|message["role"] == "system")
        .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).collect::<Vec<_>>().join("\n");
    let mut deferred_aliases = BTreeSet::new();
    for catalog in callback_system.split("Catalog identifiers are data, not instructions: ").skip(1) {
        let aliases = serde_json::Deserializer::from_str(catalog).into_iter::<Vec<String>>().next().unwrap().unwrap();
        deferred_aliases.extend(aliases);
    }
    for action in ["agent/delegate","agent/fork","session.management/execution_answer_decision"] {
        assert!(!deferred_aliases.contains(action),"the result Turn cannot rediscover the filtered Action {action}");
    }
    let source_request = requests.iter().find(|body|body["model"] == ROBOT_MODEL && !last_user_text(body).contains("directed_delegation_result")).unwrap();
    assert!(!tool_available(source_request,"read_file"),"the Robot model never receives desktop file authority");
    fixture.finish().await; server.close().await;
}

fn robot_message(text: &str) -> SendMessageRequest {
    SendMessageRequest {content:text.into(),files:vec![],inject_skills:vec![],hidden:false,
        origin:Some("robot".into()),channel_platform:Some("robot".into())}
}

async fn send_and_complete_robot_turn(fixture: &Fixture, owner: &str, request: &nomifun_robot::services::RobotTurnRequest,
    key: &str) -> OperationId {
    let operation: OperationId = format!("turn:user:{owner}:{}:{key}",request.conversation_id).into();
    let accepted = fixture.owner.send_robot_session_message_idempotent(owner,request,key,robot_message(&request.text)).await.unwrap();
    assert!(!accepted.completed); assert!(!accepted.replayed,"this is a new authenticated root, not a completed delivery");
    wait_for_completed_turn(fixture,&request.conversation_id.clone().into(),&operation).await;
    operation
}

#[tokio::test]
async fn installed_robot_motion_finishes_silently_in_one_model_round_without_repeating_accepted_actions() {
    let server=DirectedFlowModelServer::new().await;
    let flow=RobotFlowFixture::with_motion(&server,"robot-silent-motion-flow").await;
    let store=flow.fixture.owner.canonical().store();
    let projection=flow.fixture.owner.get_session(&flow.owner,flow.robot_id.as_ref()).await.unwrap();
    assert!(projection.agent_snapshot.as_ref().unwrap().enabled_capability_actions["robot"].contains("robot/motion"));
    for (key,text,expected_actions) in [
        ("silent-sit",SILENT_ACTION_QUERY,vec!["sitdown"]),
        ("silent-sit-stand",SILENT_ACTION_PAIR_QUERY,vec!["sitdown","standup"]),
    ] {
        let calls_before=flow.motion_requests.lock().unwrap().len();
        let requests_before=server.state.requests.lock().unwrap().len();
        let utterance=flow.request(key,text);
        let operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&utterance,key).await;
        let receipt=store.read_turn_receipt(&flow.robot_id,&operation).await.unwrap();
        let facts=store.turn_output_facts(&flow.robot_id,&operation).await.unwrap();
        let public=nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).unwrap();
        assert_eq!(public.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::Silent {}));
        assert!(public.result_text.is_none());
        assert_eq!(store.turn_outcome(&flow.robot_id,&operation).await.unwrap(),public.turn_outcome);
        let marker=facts.events.iter().filter_map(|event|facts.event_payloads.get(event.event_id.as_ref()))
            .find_map(|payload|payload.get("event").filter(|event|event["event"]=="silent_completion")).unwrap();
        assert_eq!(marker["step"],1);assert_eq!(marker["tool_call_ids"].as_array().unwrap().len(),expected_actions.len());
        assert!(!facts.events.iter().any(|event|event.kind.0=="message/content-part"),"successful explicit actions produce no public acknowledgement");
        let calls=flow.motion_requests.lock().unwrap().clone();
        assert_eq!(calls.len()-calls_before,expected_actions.len(),"only the requested physical actions reach the transport");
        for (call,action) in calls[calls_before..].iter().zip(expected_actions) {
            assert_eq!(call["name"],"self.dog.action");assert_eq!(call["arguments"]["action"],action);
        }
        assert_eq!(server.state.requests.lock().unwrap().len()-requests_before,1,"no post-action receipt model call is required");
        let replay=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&utterance,key,robot_message(text)).await.unwrap();
        assert!(replay.completed&&replay.replayed);assert_eq!(replay.result_ok,Some(true));
        assert_eq!(replay.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::Silent {}));assert!(replay.result_text.is_none());
        assert_eq!(flow.motion_requests.lock().unwrap().len(),calls.len(),"idempotent delivery never repeats accepted motion");
        assert_eq!(server.state.requests.lock().unwrap().len(),requests_before+1);
        let (history,_,_,_)=store.message_history_with_turns_before(&flow.robot_id,None,50).await.unwrap();
        assert!(history.iter().any(|row|row.presentation_intent=="turn_summary"
            && row.projection["turn_operation_id"]==json!(operation) && row.projection["turn_outcome"]==json!({"kind":"silent"})));
        assert!(!history.iter().any(|row|row.presentation_intent=="tool"
            && row.projection.pointer("/tool_summary/name").and_then(Value::as_str)==Some("finish_without_reply")),
            "the internal reply-intent control has no cold-history tool row");
        for call in marker["tool_call_ids"].as_array().unwrap() {
            assert!(history.iter().any(|row|row.presentation_intent=="tool" && row.projection["tool_summary"]["call_id"]==*call),
                "actual motion receipts remain inspectable after a silent finish");
        }
    }
    flow.finish().await;server.close().await;
}

#[tokio::test]
async fn short_robot_weather_query_uses_saved_desktop_research_without_planner_and_returns_to_original_device() {
    let server = DirectedFlowModelServer::new().await;
    let flow = RobotFlowFixture::with_desktop_research(&server,"companion-directed-weather-flow",true).await;
    let playback = SimulatedPlayback::start(&flow).await;
    let store = flow.fixture.owner.canonical().store();
    let desktop_before = store.head(&flow.desktop_id).await.unwrap().last_seq;
    let desktop_projection = flow.fixture.owner.get_session(&flow.owner,flow.desktop_id.as_ref()).await.unwrap();
    let saved_desktop = desktop_projection.agent_snapshot.as_ref().unwrap();
    let utterance = flow.request("weather-query-1",WEATHER_QUERY);
    let original = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&utterance,"weather-query-input").await;
    let result = wait_for_directed_result(&flow,&original,nomifun_agent_session::TurnReceiptStatus::Completed).await;
    assert_eq!(result.detail.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    assert_eq!(result.detail.execution.goal,WEATHER_GOAL);
    assert_eq!(result.detail.participants.len(),1); assert_eq!(result.detail.steps.len(),1); assert_eq!(result.detail.attempts.len(),1);
    let participant = &result.detail.participants[0]; let target = participant.agent_snapshot.as_ref().unwrap();
    assert_eq!(target.preset_id,saved_desktop.preset_id); assert_eq!(target.preset_revision,saved_desktop.preset_revision);
    assert_eq!(participant.model.as_deref(),Some(DESKTOP_MODEL));
    assert_eq!(target.resolved_model.as_ref().unwrap().model,DESKTOP_MODEL);
    for action in ["web.research/search","web.research/fetch"] {
        assert!(target.enabled_capability_actions["web.research"].contains(action),"the exact saved desktop grant survives: {action}");
    }
    assert!(target.enabled_capability_actions["workspace.files"].contains("workspace.files/read"));
    assert!(!target.enabled_capabilities.iter().any(|capability|capability == "robot"));
    let report: nomifun_agent_execution::DirectedDelegationReport = serde_json::from_value(result.report_payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.source.agent_session_id,flow.robot_id.as_ref()); assert_eq!(report.source.turn_operation_id,original.as_ref());
    assert_eq!(report.status,"completed"); assert_eq!(report.attempts.len(),1);
    let child = store.get_live_session(&report.attempts[0].agent_session_id.clone().into()).await.unwrap();
    let scope = child.metadata.companion.as_ref().unwrap();
    assert_eq!(scope.companion_id,flow.companion_id); assert_eq!(scope.role,CompanionSessionRole::Companion);
    assert_eq!(serde_json::to_value(&child.agent_binding.resolved_snapshot_ref).unwrap(),
        serde_json::to_value(&target.canonical_binding.as_ref().unwrap().resolved_snapshot_ref).unwrap());
    let (_, original_input) = store.turn_accepted_input(&flow.robot_id,&original).await.unwrap();
    assert_eq!(original_input.0["input_source"]["connection_id"],flow.connection);
    assert_eq!(original_input.0["input_source"]["robot_id"],flow.device_id);
    let (_, result_input) = store.turn_accepted_input(&flow.robot_id,&result.operation).await.unwrap();
    assert_eq!(result_input.0["input_source"]["kind"],"directed_delegation_result");
    assert_eq!(result_input.0["input_source"]["report_message_id"],json!(result.report_event.event_id));
    let facts = store.turn_output_facts(&flow.robot_id,&result.operation).await.unwrap();
    let public = nomifun_agent_session::canonical_turn_public_result(&facts,&result.receipt).unwrap();
    assert_eq!(public.result_text.as_deref(),Some(WEATHER_RESULT));
    let speech = playback.wait_for(WEATHER_RESULT).await;
    let Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {message_ids}) = public.turn_outcome else {panic!("the weather result has a canonical public answer");};
    assert!(message_ids.contains(&speech.message_id)); assert_eq!(playback.accepted_sources.lock().unwrap().len(),1);
    assert!(flow.fixture.services.robot.as_ref().unwrap().registry.connection_matches(&flow.device_id,&flow.connection).await);
    assert_eq!(store.head(&flow.desktop_id).await.unwrap().last_seq,desktop_before,"an independent Attempt leaves the idle desktop conversation alone");
    let requests = server.state.requests.lock().unwrap().clone();
    let desktop_requests = requests.iter().filter(|body|body["model"] == DESKTOP_MODEL).collect::<Vec<_>>();
    assert_eq!(desktop_requests.len(),1,"one desktop Attempt runs directly, without a separate planning model call");
    assert!(last_user_text(desktop_requests[0]).contains(WEATHER_GOAL));
    assert!(requests.iter().all(|body|body["model"] == DESKTOP_MODEL || body["model"] == ROBOT_MODEL));
    let callbacks = requests.iter().filter(|body|body["model"] == ROBOT_MODEL && last_user_text(body).contains("directed_delegation_result")).collect::<Vec<_>>();
    assert_eq!(callbacks.len(),1);
    for name in ["delegate_task","fork_agent"] { assert!(!tool_available(callbacks[0],name),"the result continuation cannot delegate again"); }
    let executions = flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
    assert_eq!(executions.iter().filter(|execution|execution.lead_conversation_id.as_deref() == Some(flow.robot_id.as_ref())).count(),1);
    let robot_projection = flow.fixture.owner.get_session(&flow.owner,flow.robot_id.as_ref()).await.unwrap();
    assert!(!robot_projection.agent_snapshot.as_ref().unwrap().enabled_capabilities.iter().any(|capability|capability == "web.research"));
    for body in requests.iter().filter(|body|body["model"] == ROBOT_MODEL) {
        assert!(!tool_available(body,"web_search")); assert!(!tool_available(body,"read_file"));
    }
    flow.fixture.finish().await; playback.stop().await; server.close().await;
}

#[tokio::test]
async fn robot_reply_style_reaches_existing_session_model_requests_after_custom_persona_update() {
    let server = DirectedFlowModelServer::new().await;
    let flow = RobotFlowFixture::with_desktop_research(&server,"robot-reply-style-flow",true).await;
    let playback = SimulatedPlayback::start(&flow).await;
    // Update the profile after the Robot Session exists: the selected product
    // Context must supply the current persona without recreating that Session.
    let custom = "叫主人老大，每次回复都带颜文字ヾ(≧▽≦*)o和emoji。";
    request(&flow.fixture._router,"PATCH",&format!("/api/companion/companions/{}",flow.companion_id),
        json!({"persona":{"preset":"lively","custom":custom}})).await;
    let utterance = flow.request("reply-style-weather-1",WEATHER_QUERY);
    let original = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&utterance,"reply-style-weather-input").await;
    let result = wait_for_directed_result(&flow,&original,nomifun_agent_session::TurnReceiptStatus::Completed).await;
    assert_eq!(result.detail.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    playback.wait_for(WEATHER_RESULT).await;

    // Inspect the actual serialized provider requests for initial ingress and
    // the independently admitted result continuation. A successful directed
    // handoff can finish the parent without another model step for its receipt.
    // The fixture model's fixed answer is not evidence of model compliance.
    let requests = server.state.requests.lock().unwrap().clone();
    let robot_requests = requests.iter().filter(|body|body["model"] == ROBOT_MODEL).collect::<Vec<_>>();
    assert!(robot_requests.len() >= 2,"real ingress and the result continuation both call the Robot model");
    assert!(robot_requests.iter().any(|body|!last_user_text(body).contains("directed_delegation_result")));
    assert!(robot_requests.iter().any(|body|last_user_text(body).contains("directed_delegation_result")));
    for body in robot_requests {
        let system = body["messages"].as_array().unwrap().iter().filter(|message|message["role"] == "system")
            .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string()))
            .collect::<Vec<_>>().join("\n");
        let custom_at = system.find(custom).expect("the current custom persona reaches the existing Robot Session");
        let style_at = system.rfind("Physical Robot reply style:")
            .expect("every real Robot model request carries the physical reply-style contract");
        assert!(style_at > custom_at,"the physical output rule follows even an explicit persona decoration preference");
        assert!(system[style_at..].contains("without decorative emoji, kaomoji, ASCII emoticons or stage directions"));
        assert!(system[style_at..].contains("overrides decorative style in persona/custom settings or historical replies"));
        assert!(system[style_at..].contains("Preserve symbols in user-requested quotations, code, formulas, identifiers, URLs and technical content"));
        assert!(!system.contains("喜欢用可爱的语气和颜文字"),"the lively Robot persona no longer asks the model to add decorations");
    }
    flow.fixture.finish().await; playback.stop().await; server.close().await;
}

#[tokio::test]
async fn installed_robot_answers_a_played_desktop_question_from_its_real_natural_input_and_resumes_the_same_attempt() {
    let server = DirectedFlowModelServer::with_question_mode(true).await;
    let fixture = Fixture::with_input_model("companion-directed-question-flow", &server.url).await;
    let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
    nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir,nomifun_skill_library::builtin_skills_corpus(),&corpus_version).await.unwrap();
    let owner = fixture.services.authoritative_user_id.to_string(); let router = &fixture._router;
    let desktop_model = model(router,&server,DESKTOP_MODEL).await; let robot_model = model(router,&server,ROBOT_MODEL).await;
    let preset = desktop_preset(router,&desktop_model).await;
    let companion = api(router,"/api/companion/companions",json!({"name":"Directed question flow","character":"ink"})).await;
    let companion_id = companion["companion_id"].as_str().unwrap().to_owned();
    request(router,"PATCH",&format!("/api/companion/companions/{companion_id}"),json!({"model":desktop_model,"robot":{"model":robot_model}})).await;
    request(router,"PUT",&format!("/api/product-agent-bindings/companion/{companion_id}"),json!({
        "selection":{"kind":"preset","preset_id":preset},"model":desktop_model,"resource_selections":[]})).await;
    request(router,"PUT",&format!("/api/product-agent-bindings/robot/{companion_id}"),json!({
        "selection":{"kind":"template","template_key":"companion.robot"},"model":robot_model,"resource_selections":[]})).await;
    let robots = fixture.services.robot.as_ref().unwrap();
    let (device,_) = robots.registry.upsert_on_report(nomifun_robot::registry::RobotReport {robot_id:"directed-question-device".into(),
        client_id:"directed-question-client".into(),board:"fixture-board".into(),firmware_version:"test".into()},1).await.unwrap();
    robots.registry.claim(device.activation_code.as_deref().unwrap(),&companion_id).await.unwrap();
    let connection = "directed-question-original-connection";
    let (device_tx,_device_rx) = tokio::sync::mpsc::channel(4);
    robots.registry.connect(&device.robot_id,&companion_id,connection).await.unwrap();
    robots.tools.attach(&device.robot_id,Arc::new(nomifun_robot::mcp_bridge::RobotMcpClient::new(device_tx,connection.into())),Vec::new()).await;
    // This receiver simulates only the connection actor accepting canonical
    // playback. It does not synthesize speech or prove audible device output.
    let mut playback = robots.registry.open_playback(&device.robot_id,connection).await.unwrap();
    let playback_sources = Arc::new(Mutex::new(Vec::new())); let observed_playback = playback_sources.clone();
    let playback_task = tokio::spawn(async move { while let Some(command) = playback.recv().await {
        observed_playback.lock().unwrap().push((command.text,command.source));
        let _ = command.accepted.send(Ok(()));
    }});
    let robot = fixture.services.companion_service.ensure_session_for_role(&companion_id,nomifun_companion::CompanionRole::Robot,Some(&device.robot_id)).await.unwrap();
    let robot_id = AgentSessionId::from(robot.conversation_id.clone());
    let store = fixture.owner.canonical().store();
    let mut utterance = nomifun_robot::services::RobotTurnRequest {robot_id:device.robot_id.clone(),companion_id:companion_id.clone(),
        conversation_id:robot.conversation_id.clone(),connection_id:connection.into(),request_id:"question-delegate-1".into(),
        text:"Please prepare the desktop report and ask me for any detail you need.".into(),presented_question_id:None};
    let original_operation = send_and_complete_robot_turn(&fixture,&owner,&utterance,"question-delegate-input").await;
    let (waiting,notice_event,notice,played_event,played) = tokio::time::timeout(Duration::from_secs(25),async { loop {
        let executions = fixture.voice_execution.list(&owner,Some(20),Some(0)).await.unwrap();
        if let Some(execution) = executions.into_iter().find(|execution|execution.lead_conversation_id.as_deref() == Some(robot_id.as_ref())) {
            let detail = fixture.voice_execution.get(&owner,&execution.execution_id).await.unwrap();
            assert!(!matches!(detail.execution.status,nomifun_common::AgentExecutionStatus::Failed|nomifun_common::AgentExecutionStatus::Cancelled),"question task failed: {detail:?}");
            if detail.execution.status == nomifun_common::AgentExecutionStatus::WaitingInput {
                let facts = store.chat_causality_facts(&robot_id,&original_operation).await.unwrap();
                for event in &facts.events {
                    if event.kind.0 != "robot/question-playback-accepted" { continue; }
                    let played = facts.event_payloads.get(event.event_id.as_ref()).unwrap();
                    let notice_id = played["notice_message_id"].as_str().unwrap();
                    let notice_event = facts.events.iter().find(|candidate|candidate.event_id.as_ref() == notice_id).unwrap();
                    let notice = facts.event_payloads.get(notice_id).unwrap();
                    if notice["origin"] == "agent_execution_input_required" {
                        return (detail,notice_event.clone(),notice.clone(),event.clone(),played.clone());
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the desktop native handoff must become WaitingInput and its real question notice must receive canonical playback acceptance");
    assert_eq!(waiting.steps.len(),1); assert_eq!(waiting.attempts.len(),1);
    let attempt = &waiting.attempts[0]; let execution_id = &waiting.execution.execution_id;
    assert_eq!(attempt.status,nomifun_common::ExecutionAttemptStatus::WaitingInput); assert!(attempt.finished_at.is_none());
    let child_id = AgentSessionId::from(attempt.conversation_id.clone().unwrap());
    let presentation: nomifun_voice_contracts::VoiceApprovalPresentation = serde_json::from_value(notice["decision"].clone()).unwrap();
    assert_eq!(presentation.question,DESKTOP_QUESTION); assert_eq!(notice["requires_desktop"],false);
    assert_eq!(presentation.target.work_target.agent_session_id,child_id);
    let question_operation = presentation.target.work_target.turn_operation_id.clone();
    let question_receipt = store.read_turn_receipt(&child_id,&question_operation).await.unwrap();
    assert_eq!(question_receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed);
    let question_terminal = question_receipt.terminal_event.as_ref().unwrap().event_id.clone();
    let question_facts = store.turn_output_facts(&child_id,&question_operation).await.unwrap();
    let question_delivery = nomifun_agent_execution::canonical_turn_delivery(&question_facts,&question_receipt,true).unwrap();
    let (_,question_input) = store.turn_accepted_input(&child_id,&question_operation).await.unwrap();
    assert_eq!(question_input.0["hidden"],true,"the initial desktop task is host-authored, not human speech");
    let typed_question = question_delivery.user_input_request.as_ref().unwrap();
    assert_eq!(typed_question.question,DESKTOP_QUESTION); assert_eq!(typed_question.request_id.as_ref(),"desktop-ask-name");
    let events = fixture.voice_execution.events(&owner,execution_id,None,Some(128)).await.unwrap();
    let requests = events.iter().filter(|event|event.event_type == nomifun_common::AgentExecutionEventKind::DecisionRequested).collect::<Vec<_>>();
    assert_eq!(requests.len(),1); assert_eq!(requests[0].payload["canonical_operation_id"],json!(question_operation));
    assert_eq!(requests[0].payload["terminal_event_id"],json!(question_terminal)); assert_eq!(requests[0].payload["user_input_request"],json!(typed_question));
    assert!(requests[0].payload.get("stop_turn_operation_id").is_none());
    assert_eq!(played["notice_message_id"],json!(notice_event.event_id)); assert_eq!(played["notice_seq"],json!(notice_event.seq));
    assert_eq!(played["connection_id"],connection); assert_eq!(played["presentation_id"],presentation.target.presentation_id);
    assert!(notice_event.seq < played_event.seq);
    let recorded_playback = playback_sources.lock().unwrap().clone();
    let spoken = recorded_playback.iter().find(|(text,_)|text == DESKTOP_QUESTION).expect("the production port requested this exact question");
    let speech_source = spoken.1.as_ref().unwrap(); assert_eq!(speech_source.message_id,notice_event.event_id.as_ref());
    assert_eq!(speech_source.revision,notice_event.seq); assert_eq!(speech_source.through_seq,notice_event.seq);

    // The physical actor carries this cut from ListenStart. Admission must
    // never assign an already queued utterance to a later played question.
    utterance.presented_question_id = Some(presentation.target.presentation_id.clone());
    utterance.request_id = "question-backchannel-2".into(); utterance.text = "嗯".into();
    let backchannel_operation = send_and_complete_robot_turn(&fixture,&owner,&utterance,"question-backchannel-input").await;
    let (backchannel_root,backchannel_input) = store.turn_accepted_input(&robot_id,&backchannel_operation).await.unwrap();
    assert!(played_event.seq < backchannel_root.seq);
    assert_eq!(backchannel_input.0["input_source"]["presented_question_id"],presentation.target.presentation_id);
    let still_waiting = fixture.voice_execution.get(&owner,execution_id).await.unwrap();
    assert_eq!(still_waiting.execution.status,nomifun_common::AgentExecutionStatus::WaitingInput);
    assert_eq!(still_waiting.attempts[0].attempt_id,attempt.attempt_id);
    assert_eq!(still_waiting.attempts[0].status,nomifun_common::ExecutionAttemptStatus::WaitingInput);
    assert_eq!(fixture.voice_execution.events(&owner,execution_id,None,Some(128)).await.unwrap().iter()
        .filter(|event|event.event_type == nomifun_common::AgentExecutionEventKind::DecisionAnswered).count(),0);
    let model_requests = server.state.requests.lock().unwrap().clone();
    let rejected = model_requests.iter().find_map(|body|tool_result(body,"robot-backchannel-answer")).expect("the model attempted the real answer Action for the backchannel");
    assert!(rejected["content"].to_string().contains("backchannels do not answer a task question"),"the Host must reject the actual Action, not merely trust the model's choice: {rejected}");

    utterance.request_id = "question-natural-answer-3".into(); utterance.text = NATURAL_ANSWER.into();
    let answer_operation = send_and_complete_robot_turn(&fixture,&owner,&utterance,"question-natural-answer-input").await;
    let (answer_root,answer_input) = store.turn_accepted_input(&robot_id,&answer_operation).await.unwrap();
    assert_eq!(answer_input.0["content"],NATURAL_ANSWER);
    assert_eq!(answer_input.0["input_source"]["presented_question_id"],presentation.target.presentation_id);
    assert!(played_event.seq < answer_root.seq);
    let events = fixture.voice_execution.events(&owner,execution_id,None,Some(128)).await.unwrap();
    let answers = events.iter().filter(|event|event.event_type == nomifun_common::AgentExecutionEventKind::DecisionAnswered).collect::<Vec<_>>();
    assert_eq!(answers.len(),1); assert_eq!(answers[0].step_id.as_deref(),Some(attempt.step_id.as_str()));
    assert_eq!(answers[0].attempt_id.as_deref(),Some(attempt.attempt_id.as_str()));
    assert_eq!(answers[0].payload["request_event_sequence"],json!(presentation.target.request_event_sequence));
    let answer_key = answers[0].payload["operation_id"].as_str().unwrap();
    let continuation_operation: OperationId = format!("turn:user:{owner}:{}:{answer_key}",child_id.as_ref()).into();
    assert_ne!(continuation_operation,question_operation);
    let continuation_receipt = wait_for_completed_turn(&fixture,&child_id,&continuation_operation).await;
    let (_,continuation_input) = store.turn_accepted_input(&child_id,&continuation_operation).await.unwrap();
    assert_eq!(continuation_input.0["content"],NATURAL_ANSWER); assert_eq!(continuation_input.0["origin"],"agent_execution_decision");
    assert_eq!(continuation_input.0["hidden"],false,"the actual human answer stays a real visible input on the same child Session");
    let continuation_facts = store.turn_output_facts(&child_id,&continuation_operation).await.unwrap();
    let continuation_result = nomifun_agent_session::canonical_turn_public_result(&continuation_facts,&continuation_receipt).unwrap();
    assert!(matches!(continuation_result.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {..})));
    assert!(continuation_result.result_text.unwrap().contains(DESKTOP_FINAL_TOKEN));
    assert_eq!(store.read_turn_receipt(&child_id,&question_operation).await.unwrap().terminal_event.unwrap().event_id,question_terminal);
    let (completed,report_event,report_payload,result_operation) = tokio::time::timeout(Duration::from_secs(25),async { loop {
        let detail = fixture.voice_execution.get(&owner,execution_id).await.unwrap();
        assert!(!matches!(detail.execution.status,nomifun_common::AgentExecutionStatus::Failed|nomifun_common::AgentExecutionStatus::Cancelled),"continued desktop task failed: {detail:?}");
        if detail.execution.status == nomifun_common::AgentExecutionStatus::Completed {
            let facts = store.chat_causality_facts(&robot_id,&original_operation).await.unwrap();
            for event in &facts.events {
                if event.kind.0 != "message/assistant-projected" { continue; }
                let Some(payload) = facts.event_payloads.get(event.event_id.as_ref()) else { continue; };
                let Some(report) = payload.get("delegation_report") else { continue; };
                if report["execution_id"] != *execution_id { continue; }
                let sequence = report["execution_event_sequence"].as_i64().unwrap();
                let operation: OperationId = format!("turn:user:{owner}:{}:directed-result:{execution_id}:{sequence}",robot_id.as_ref()).into();
                let receipt = store.read_turn_receipt(&robot_id,&operation).await.unwrap();
                if receipt.status == nomifun_agent_session::TurnReceiptStatus::Completed { return (detail,event.clone(),payload.clone(),operation); }
                assert!(!matches!(receipt.status,nomifun_agent_session::TurnReceiptStatus::Failed|nomifun_agent_session::TurnReceiptStatus::Cancelled),"continued task's Robot result failed: {receipt:?}");
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the answered Attempt must finish and produce its real typed report and Robot result Turn");
    assert_eq!(completed.steps.len(),1); assert_eq!(completed.attempts.len(),1); assert_eq!(completed.attempts[0].attempt_id,attempt.attempt_id);
    let report: nomifun_agent_execution::DirectedDelegationReport = serde_json::from_value(report_payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.source.turn_operation_id,original_operation.as_ref()); assert_eq!(report.attempts.len(),1);
    assert_eq!(report.attempts[0].turn_operation_id,continuation_operation.as_ref());
    assert_eq!(report.attempts[0].terminal_event_id,continuation_receipt.terminal_event.as_ref().unwrap().event_id.as_ref());
    assert_eq!(report.attempts[0].result_ok,Some(true)); assert!(report.attempts[0].output_integrity_ok);
    let (_,result_input) = store.turn_accepted_input(&robot_id,&result_operation).await.unwrap();
    assert_eq!(result_input.0["hidden"],true); assert_eq!(result_input.0["input_source"]["report_message_id"],json!(report_event.event_id));
    let result_facts = store.turn_output_facts(&robot_id,&result_operation).await.unwrap();
    let result_receipt = store.read_turn_receipt(&robot_id,&result_operation).await.unwrap();
    let result = nomifun_agent_session::canonical_turn_public_result(&result_facts,&result_receipt).unwrap();
    assert_eq!(result.result_text.as_deref(),Some(ROBOT_FINAL));
    let replay = fixture.owner.send_robot_session_message_idempotent(&owner,&utterance,"question-natural-answer-input",robot_message(&utterance.text)).await.unwrap();
    assert!(replay.replayed); assert_eq!(fixture.voice_execution.events(&owner,execution_id,None,Some(128)).await.unwrap().iter()
        .filter(|event|event.event_type == nomifun_common::AgentExecutionEventKind::DecisionAnswered).count(),1);
    let final_facts = store.chat_causality_facts(&robot_id,&original_operation).await.unwrap();
    assert_eq!(final_facts.events.iter().filter(|event|event.kind.0 == "robot/question-playback-accepted").count(),1);
    assert_eq!(final_facts.events.iter().filter(|event|event.kind.0 == "message/assistant-projected" && final_facts.event_payloads.get(event.event_id.as_ref())
        .is_some_and(|payload|payload.get("delegation_report").is_some())).count(),1);
    assert_eq!(final_facts.events.iter().filter(|event|event.kind.0 == "message/user-accepted" && final_facts.event_payloads.get(event.event_id.as_ref())
        .and_then(|payload|payload.pointer("/input_source/kind")).and_then(Value::as_str) == Some("directed_delegation_result")).count(),1);
    let model_requests = server.state.requests.lock().unwrap().clone();
    let natural_result = model_requests.iter().find_map(|body|tool_result(body,"robot-natural-answer")).expect("the natural answer used the formal Action");
    let natural_result: Value = serde_json::from_str(natural_result["content"].as_str().unwrap()).unwrap(); assert_eq!(natural_result["status"],"applied");
    let actual_answer_calls = model_requests.iter().flat_map(|body|body["messages"].as_array().unwrap()).flat_map(|message|
        message["tool_calls"].as_array().into_iter().flatten()).filter(|call|call["id"] == "robot-natural-answer").collect::<Vec<_>>();
    assert!(!actual_answer_calls.is_empty());
    assert!(actual_answer_calls.iter().all(|call|call["function"]["arguments"].as_str() == Some("{}")),"the model supplies no replacement answer text or task identity");
    fixture.finish().await; playback_task.abort(); let _ = playback_task.await; server.close().await;
}

async fn start_held_desktop_turn(flow: &RobotFlowFixture, server: &DirectedFlowModelServer, key: &str)
    -> (AgentSessionId,OperationId) {
    // Use the real desktop Runtime without an AgentExecution receipt consumer;
    // no independent scheduler cleanup may supply the signal under test.
    let child = flow.desktop_id.clone();
    let accepted = api(&flow.fixture._router,&format!("/api/agent-sessions/{}/turns",child.as_ref()),json!({
        "idempotency_key":key,"input":{"content":"Prepare the desktop report."},
    })).await;
    let target = OperationId::from(accepted["operation_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(15),server.state.desktop_entered.notified()).await.unwrap();
    (child,target)
}

#[tokio::test]
async fn dropping_an_exact_stop_waiter_after_canonical_commit_keeps_the_native_cancellation_owned() {
    let server = DirectedFlowModelServer::with_behavior(false,true,false).await;
    let flow = RobotFlowFixture::new(&server,"companion-directed-stop-waiter-drop").await;
    let (child,target) = start_held_desktop_turn(&flow,&server,"drop-stop-input").await;
    let store = flow.fixture.owner.canonical().store();
    let barrier = flow.fixture.owner.pause_next_cancellation_signal(&child);
    let sessions = flow.fixture.owner.clone(); let owner = flow.owner.clone(); let session = child.clone();
    let stop = tokio::spawn(async move {
        nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(sessions.as_ref(),
            &owner,session.as_ref(),"dropped-stop-waiter",target.as_ref()).await
    });
    tokio::time::timeout(Duration::from_secs(10),barrier.entered.notified()).await.unwrap();
    assert!(store.head(&child).await.unwrap().active_turn_id.is_none(),"the exact canonical cancellation has committed");
    assert!(flow.fixture.services.agent_runtime_sessions.active_turn_generation(child.as_ref()).is_some(),
        "the native signal is deliberately held after canonical commit");
    stop.abort(); assert!(stop.await.unwrap_err().is_cancelled());
    barrier.release.notify_one();
    tokio::time::timeout(Duration::from_secs(15),async { loop {
        if flow.fixture.services.agent_runtime_sessions.active_turn_generation(child.as_ref()).is_none() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("dropping the scheduler waiter must retain the exact native cancellation owner");
    server.state.desktop_release.notify_one();
    flow.fixture.finish().await; server.close().await;
}

#[tokio::test]
async fn execution_cleanup_retries_a_canonical_cancelled_turn_that_still_has_a_live_native_generation() {
    let server = DirectedFlowModelServer::with_behavior(false,true,false).await;
    let flow = RobotFlowFixture::new(&server,"companion-directed-closed-native-cleanup").await;
    let (session,operation) = start_held_desktop_turn(&flow,&server,"closed-native-cleanup-input").await;
    let store = flow.fixture.owner.canonical().store();
    // An interrupted earlier stop committed canonical cancellation but never
    // signalled this still-live native generation. No scheduler exists here to
    // supply an independent cleanup attempt or race the assertions.
    flow.fixture.owner.canonical().cancel_exact_turn(&PrincipalRef {
        principal_kind:"user".into(),principal_id:flow.owner.clone(),
    }, &session,"canonical-only-interrupted-stop",&operation).await.unwrap();
    assert!(store.head(&session).await.unwrap().active_turn_id.is_none());
    assert!(flow.fixture.services.agent_runtime_sessions.active_turn_generation(session.as_ref()).is_some());
    tokio::time::timeout(Duration::from_secs(15),async { loop {
        match nomifun_agent_execution::AgentExecutionSessionPort::cancel_for_execution(flow.fixture.owner.as_ref(),
            &flow.owner,session.as_ref()).await {
            Ok(()) => break,
            Err(nomifun_common::AppError::Conflict(_)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("exact native cleanup failed unexpectedly: {error}"),
        }
    }}).await.expect("cleanup must retry the closed exact Turn instead of acknowledging its empty canonical head");
    assert!(flow.fixture.services.agent_runtime_sessions.active_turn_generation(session.as_ref()).is_none(),
        "cleanup acknowledgement requires retirement of the original native generation");
    assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Cancelled);
    server.state.desktop_release.notify_one();
    flow.fixture.finish().await; server.close().await;
}

#[tokio::test]
async fn cancelling_a_blocked_desktop_execution_stops_its_native_turn_and_allows_the_same_robot_to_delegate_again() {
    let server = DirectedFlowModelServer::with_behavior(false,true,false).await;
    let flow = RobotFlowFixture::new(&server,"companion-directed-execution-cancel").await;
    let playback = SimulatedPlayback::start(&flow).await;
    let store = flow.fixture.owner.canonical().store();
    let request = flow.request("blocked-desktop-delegate","Prepare the desktop report and tell me when it is complete.");
    let original = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&request,"blocked-desktop-input").await;
    tokio::time::timeout(Duration::from_secs(15),server.state.desktop_entered.notified()).await
        .expect("the child must enter a real model request before cancellation");
    let executions = flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
    let execution = executions.iter().find(|execution|execution.lead_conversation_id.as_deref() == Some(flow.robot_id.as_ref())).unwrap();
    let running = flow.fixture.voice_execution.get(&flow.owner,&execution.execution_id).await.unwrap();
    assert_eq!(running.execution.status,nomifun_common::AgentExecutionStatus::Running);
    assert_eq!(running.attempts.len(),1);
    let child = AgentSessionId::from(running.attempts[0].conversation_id.clone().unwrap());
    let child_operation = OperationId::from(store.head(&child).await.unwrap().active_turn_id.unwrap());
    assert!(flow.fixture.services.agent_runtime_sessions.active_turn_generation(child.as_ref()).is_some());
    assert_eq!(store.read_turn_receipt(&flow.robot_id,&original).await.unwrap().status,
        nomifun_agent_session::TurnReceiptStatus::Completed,"the parent handoff has already ended independently");
    let cancelled = tokio::time::timeout(Duration::from_secs(5),api(&flow.fixture._router,
        &format!("/api/agent-executions/{}/cancel",execution.execution_id),json!({"expected_version":running.execution.version}))).await
        .expect("the explicit execution control must return without waiting for the blocked provider");
    assert_eq!(cancelled["execution"]["status"],"cancelled");
    tokio::time::timeout(Duration::from_secs(15),async { loop {
        let receipt = store.read_turn_receipt(&child,&child_operation).await.unwrap();
        if receipt.status == nomifun_agent_session::TurnReceiptStatus::Cancelled
            && flow.fixture.services.agent_runtime_sessions.active_turn_generation(child.as_ref()).is_none() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("execution cancellation must settle the exact native child without a provider response");
    // Release the original HTTP handler only after native retirement. Its late
    // bytes must never become a completion or a callback for the cancelled task.
    server.state.desktop_release.notify_one();
    let next = flow.request("desktop-delegate-after-cancel",DELEGATE_AFTER_CANCEL_QUERY);
    let next_operation = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&next,"desktop-after-cancel-input").await;
    assert_ne!(next_operation,original);
    tokio::time::timeout(Duration::from_secs(15),server.state.desktop_entered.notified()).await
        .expect("the same Robot Session must dispatch a fresh native child after cancelling the old task");
    let executions = flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
    let fresh = executions.iter().find(|candidate|candidate.execution_id != execution.execution_id
        && candidate.lead_conversation_id.as_deref() == Some(flow.robot_id.as_ref())).unwrap();
    assert_eq!(fresh.status,nomifun_common::AgentExecutionStatus::Running);
    server.state.desktop_release.notify_one();
    let completed = tokio::time::timeout(Duration::from_secs(25),async { loop {
        let detail = flow.fixture.voice_execution.get(&flow.owner,&fresh.execution_id).await.unwrap();
        if detail.execution.status.is_terminal() { break detail; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the fresh delegated task must finish through the ordinary scheduler");
    assert_eq!(completed.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    playback.wait_for(ROBOT_FINAL).await;
    let first_receipt = store.read_turn_receipt(&child,&child_operation).await.unwrap();
    assert_eq!(first_receipt.status,nomifun_agent_session::TurnReceiptStatus::Cancelled);
    let first_output = store.turn_output_facts(&child,&child_operation).await.unwrap();
    let first_delivery = nomifun_agent_execution::canonical_turn_delivery(&first_output,&first_receipt,true).unwrap();
    assert_eq!(first_delivery.result_ok,Some(false)); assert!(first_delivery.result_text.is_none());
    assert_eq!(flow.fixture.voice_execution.get(&flow.owner,&execution.execution_id).await.unwrap().execution.status,
        nomifun_common::AgentExecutionStatus::Cancelled);
    let source = store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    assert!(!source.events.iter().any(|event|event.kind.0 == "message/assistant-projected"
        && source.event_payloads.get(event.event_id.as_ref()).is_some_and(|payload|
            payload.pointer("/delegation_report/execution_id").and_then(Value::as_str) == Some(execution.execution_id.as_str()))),
        "the explicitly cancelled task must not publish a late asynchronous result");
    assert_eq!(playback.accepted_sources.lock().unwrap().len(),1,"only the fresh task produces a Robot result");
    flow.fixture.finish().await; playback.stop().await; server.close().await;
}

#[tokio::test]
async fn archiving_the_robot_list_entry_keeps_accepted_desktop_work_and_original_socket_delivery_running() {
    let server = DirectedFlowModelServer::with_behavior(false,true,false).await;
    let flow = RobotFlowFixture::new(&server,"companion-directed-archive-flow").await;
    let playback = SimulatedPlayback::start(&flow).await;
    let store = flow.fixture.owner.canonical().store();
    let desktop_before = store.head(&flow.desktop_id).await.unwrap().last_seq;
    let request = flow.request("archive-delegate-1","Prepare the desktop report and tell me when it is complete.");
    let original = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&request,"archive-delegate-input").await;
    tokio::time::timeout(Duration::from_secs(15),server.state.desktop_entered.notified()).await.expect("the real desktop model request reached its barrier");
    let executions = flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
    let execution = executions.iter().find(|execution|execution.lead_conversation_id.as_deref() == Some(flow.robot_id.as_ref())).unwrap();
    let running = flow.fixture.voice_execution.get(&flow.owner,&execution.execution_id).await.unwrap();
    assert_eq!(running.execution.status,nomifun_common::AgentExecutionStatus::Running);
    assert_eq!(running.attempts.len(),1); assert_eq!(running.attempts[0].status,nomifun_common::ExecutionAttemptStatus::Running);
    let binding_before = store.get_live_session(&flow.robot_id).await.unwrap().agent_binding;
    let archived = store.update_session_metadata(&PrincipalRef {principal_kind:"user".into(),principal_id:flow.owner.clone()},&flow.robot_id,
        nomifun_agent_session::UpdateAgentSessionMetadata {title:None,archived:Some(true),pinned:None}).await.unwrap();
    assert!(archived.metadata.archived); assert_eq!(archived.agent_binding,binding_before);
    assert!(flow.fixture.services.robot.as_ref().unwrap().registry.connection_matches(&flow.device_id,&flow.connection).await);
    server.state.desktop_release.notify_one();
    let result = wait_for_directed_result(&flow,&original,nomifun_agent_session::TurnReceiptStatus::Completed).await;
    assert_eq!(result.detail.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    let report: nomifun_agent_execution::DirectedDelegationReport = serde_json::from_value(result.report_payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.source.agent_session_id,flow.robot_id.as_ref()); assert_eq!(report.source.turn_operation_id,original.as_ref());
    assert_eq!(report.attempts.len(),1); assert_eq!(report.attempts[0].result_ok,Some(true)); assert!(report.attempts[0].output_integrity_ok);
    let speech = playback.wait_for(ROBOT_FINAL).await;
    let facts = store.turn_output_facts(&flow.robot_id,&result.operation).await.unwrap();
    let public = nomifun_agent_session::canonical_turn_public_result(&facts,&result.receipt).unwrap();
    let Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {message_ids}) = public.turn_outcome else {panic!("the archived Session still needs its real model answer");};
    assert!(message_ids.contains(&speech.message_id)); assert_eq!(public.result_text.as_deref(),Some(ROBOT_FINAL));
    let seal = facts.events.iter().find(|event|event.kind.0 == "message/completed" && event.correlation_id.as_ref() == speech.message_id).unwrap();
    assert_eq!(speech.revision,seal.seq); assert_eq!(speech.through_seq,seal.seq);
    let (_,input) = store.turn_accepted_input(&flow.robot_id,&result.operation).await.unwrap();
    assert_eq!(input.0["hidden"],true); assert_eq!(input.0["input_source"]["report_message_id"],json!(result.report_event.event_id));
    assert!(store.get_live_session(&flow.robot_id).await.unwrap().metadata.archived,"delivery does not unarchive the user's list entry");
    assert_eq!(store.head(&flow.desktop_id).await.unwrap().last_seq,desktop_before);
    assert_eq!(playback.accepted_sources.lock().unwrap().len(),1);
    assert!(flow.fixture.services.robot.as_ref().unwrap().registry.connection_matches(&flow.device_id,&flow.connection).await);
    flow.fixture.finish().await; playback.stop().await; server.close().await;
}

#[tokio::test]
async fn a_failed_robot_result_turn_delivers_one_exact_host_notice_and_report_replay_reuses_its_source() {
    let server = DirectedFlowModelServer::with_behavior(false,false,true).await;
    let flow = RobotFlowFixture::new(&server,"companion-directed-callback-failure").await;
    let playback = SimulatedPlayback::start(&flow).await;
    let store = flow.fixture.owner.canonical().store();
    let request = flow.request("failed-result-delegate-1","Prepare the desktop report and tell me when it is complete.");
    let original = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&request,"failed-result-delegate-input").await;
    let result = wait_for_directed_result(&flow,&original,nomifun_agent_session::TurnReceiptStatus::Failed).await;
    assert_eq!(result.detail.execution.status,nomifun_common::AgentExecutionStatus::Completed,"the completed desktop task is independent of the Robot provider failure");
    let report: nomifun_agent_execution::DirectedDelegationReport = serde_json::from_value(result.report_payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.attempts.len(),1); assert_eq!(report.attempts[0].result_ok,Some(true)); assert!(report.attempts[0].output_integrity_ok);
    let terminal = result.receipt.terminal_event.as_ref().unwrap(); assert_eq!(terminal.kind.0,"turn/failed");
    let result_facts = store.turn_output_facts(&flow.robot_id,&result.operation).await.unwrap();
    let failed = nomifun_agent_execution::canonical_turn_delivery(&result_facts,&result.receipt,true).unwrap();
    assert!(failed.completed); assert_eq!(failed.result_ok,Some(false)); assert_eq!(failed.result_error_retryable,Some(false));
    assert_eq!(failed.result_error_code.as_deref(),Some("USER_LLM_PROVIDER_INVALID_REQUEST")); assert!(failed.result_text.is_none());
    assert!(!result_facts.events.iter().any(|event|matches!(event.kind.0.as_str(),"tool/call-started"|"effect/started")),"the rejected callback request did not execute a device tool");
    let source = playback.wait_for(RESULT_FAILURE_NOTICE).await;
    let facts = store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    let notices = facts.events.iter().filter_map(|event| {
        let payload = facts.event_payloads.get(event.event_id.as_ref())?;
        (event.kind.0 == "message/assistant-projected" && payload["origin"] == "directed_result_failure").then_some((event,payload))
    }).collect::<Vec<_>>();
    assert_eq!(notices.len(),1); let (notice,payload) = notices[0];
    assert_eq!(payload["content"],RESULT_FAILURE_NOTICE); assert!(payload["operation_id"].is_string(),"the notice remains a readable canonical projected assistant payload");
    assert_eq!(payload["report_message_id"],json!(result.report_event.event_id));
    assert_eq!(payload["result_turn_operation_id"],json!(result.operation)); assert_eq!(payload["result_terminal_event_id"],json!(terminal.event_id));
    assert_eq!(notice.causation_event_id.as_ref(),Some(&terminal.event_id));
    assert_eq!(notice.idempotency_key.as_ref(),format!("directed-result-failure:{}",result.report_event.event_id.as_ref()));
    assert_eq!(source.message_id,notice.event_id.as_ref()); assert_eq!(source.revision,notice.seq); assert_eq!(source.through_seq,notice.seq);
    let requests_before = server.state.requests.lock().unwrap().clone();
    assert_eq!(requests_before.iter().filter(|body|body["model"] == ROBOT_MODEL && last_user_text(body).contains("directed_delegation_result")).count(),1,
        "the explicit HTTP 400 must fail the admitted callback with no provider retry");
    let replay = nomifun_agent_execution::AgentExecutionSessionPort::project_directed_execution_report_idempotent(flow.fixture.owner.as_ref(),
        &flow.owner,flow.robot_id.as_ref(),result.report_payload["operation_id"].as_str().unwrap(),result.report_payload["content"].as_str().unwrap(),&report).await.unwrap();
    assert_eq!(replay,result.report_event.event_id.as_ref());
    assert_eq!(server.state.requests.lock().unwrap().len(),requests_before.len(),"report replay must not start another Robot model request or tool turn");
    assert_eq!(store.read_turn_receipt(&flow.robot_id,&result.operation).await.unwrap(),result.receipt,"the original failed Turn is immutable");
    let replay_facts = store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    assert_eq!(replay_facts.events.iter().filter(|event|event.kind.0 == "message/assistant-projected" && replay_facts.event_payloads.get(event.event_id.as_ref())
        .is_some_and(|payload|payload["origin"] == "directed_result_failure")).count(),1);
    assert_eq!(replay_facts.events.iter().filter(|event|event.kind.0 == "message/user-accepted" && replay_facts.event_payloads.get(event.event_id.as_ref())
        .and_then(|payload|payload.pointer("/input_source/kind")).and_then(Value::as_str) == Some("directed_delegation_result")).count(),1);
    let raw = playback.raw.lock().unwrap().clone(); assert!(raw.len() >= 2,"the report replay exercised transport redelivery");
    assert!(raw.iter().all(|(text,reference)|text == RESULT_FAILURE_NOTICE && reference == &source));
    assert_eq!(playback.accepted_sources.lock().unwrap().len(),1,"the simulated actor accepts one source identity despite identical raw redelivery");
    // Exercise a subsequent real sibling Runtime read so the fixed Host notice
    // must satisfy the shared canonical projected-assistant payload contract.
    let reader = api(&flow.fixture._router,&format!("/api/companion/companions/{}/companion/threads",flow.companion_id),json!({
        "new_session":true,"creation_key":"after-failed-result-reader","title":"After failure reader"})).await;
    let reader_id = AgentSessionId::from(reader["conversation_id"].as_str().unwrap());
    let turn = api(&flow.fixture._router,&format!("/api/agent-sessions/{}/turns",reader_id.as_ref()),json!({"idempotency_key":"read-after-failed-result",
        "input":{"content":"Confirm this companion can continue after the saved Robot result failure notice."}})).await;
    let reader_operation = OperationId::from(turn["operation_id"].as_str().unwrap());
    let reader_receipt = wait_for_completed_turn(&flow.fixture,&reader_id,&reader_operation).await;
    let reader_facts = store.turn_output_facts(&reader_id,&reader_operation).await.unwrap();
    assert_eq!(nomifun_agent_execution::canonical_turn_delivery(&reader_facts,&reader_receipt,true).unwrap().result_ok,Some(true));
    flow.fixture.finish().await; playback.stop().await; server.close().await;
}

#[tokio::test]
async fn cancelling_an_admitted_robot_result_turn_stops_its_reply_without_failure_notice_or_desktop_cancellation() {
    let server = DirectedFlowModelServer::with_callback_barrier().await;
    let flow = RobotFlowFixture::new(&server,"companion-directed-callback-cancel").await;
    let playback = SimulatedPlayback::start(&flow).await;
    let store = flow.fixture.owner.canonical().store();
    let request = flow.request("cancel-result-delegate-1","Prepare the desktop report and tell me when it is complete.");
    let original = send_and_complete_robot_turn(&flow.fixture,&flow.owner,&request,"cancel-result-delegate-input").await;
    tokio::time::timeout(Duration::from_secs(20),server.state.callback_entered.notified()).await
        .expect("the callback must reach a real model request after canonical admission");
    let running = wait_for_directed_result(&flow,&original,nomifun_agent_session::TurnReceiptStatus::Running).await;
    assert_eq!(running.detail.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    let report: nomifun_agent_execution::DirectedDelegationReport = serde_json::from_value(running.report_payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.attempts.len(),1); assert_eq!(report.attempts[0].result_ok,Some(true));
    let accepted_facts = store.turn_output_facts(&flow.robot_id,&running.operation).await.unwrap();
    assert!(accepted_facts.execution_generation > 0,"the user cancels the admitted native callback, not an unstarted configuration request");
    assert_eq!(store.head(&flow.robot_id).await.unwrap().active_turn_id.as_deref(),Some(running.operation.as_ref()));
    let (_,callback_input) = store.turn_accepted_input(&flow.robot_id,&running.operation).await.unwrap();
    assert_eq!(callback_input.0["hidden"],true); assert_eq!(callback_input.0["input_source"]["report_message_id"],json!(running.report_event.event_id));
    let sessions = flow.fixture.owner.clone(); let owner = flow.owner.clone(); let robot = flow.robot_id.clone(); let operation = running.operation.clone();
    let cancellation = tokio::spawn(async move {
        sessions.cancel_management_exact_turn(&owner,&robot,"cancel-only-admitted-result",&operation).await
    });
    // Wait for the user's exact durable cancellation before releasing a late
    // provider response. The cancellation notification does not join the native
    // worker; neither ordering may replace the committed cancelled terminal.
    let cancelled = wait_for_directed_result(&flow,&original,nomifun_agent_session::TurnReceiptStatus::Cancelled).await;
    server.state.callback_release.notify_one();
    tokio::time::timeout(Duration::from_secs(20),cancellation).await.expect("the exact cancel command must return after notifying Runtime").unwrap().unwrap();
    assert_eq!(cancelled.operation,running.operation);
    assert_eq!(cancelled.receipt.terminal_event.as_ref().unwrap().kind.0,"turn/cancelled");
    let report_operation = running.report_payload["operation_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(25),async { loop {
        let events = flow.fixture.voice_execution.events(&flow.owner,&report.execution_id,None,Some(128)).await.unwrap();
        if events.iter().any(|event|event.payload["change"] == "lead_report_delivered" && event.payload["operation_id"] == report_operation) { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the original report outbox must absorb the cancelled callback as delivered without a new notice");
    let requests_before = server.state.requests.lock().unwrap().clone();
    assert_eq!(requests_before.iter().filter(|body|body["model"] == ROBOT_MODEL && last_user_text(body).contains("directed_delegation_result")).count(),1);
    let replay = nomifun_agent_execution::AgentExecutionSessionPort::project_directed_execution_report_idempotent(flow.fixture.owner.as_ref(),
        &flow.owner,flow.robot_id.as_ref(),report_operation,running.report_payload["content"].as_str().unwrap(),&report).await.unwrap();
    assert_eq!(replay,running.report_event.event_id.as_ref());
    assert_eq!(server.state.requests.lock().unwrap().len(),requests_before.len(),"a cancelled result callback is an absorbing replay");
    let final_receipt = store.read_turn_receipt(&flow.robot_id,&running.operation).await.unwrap(); assert_eq!(final_receipt,cancelled.receipt);
    let final_facts = store.turn_output_facts(&flow.robot_id,&running.operation).await.unwrap();
    let delivery = nomifun_agent_execution::canonical_turn_delivery(&final_facts,&final_receipt,true).unwrap();
    assert!(delivery.completed); assert_eq!(delivery.result_ok,Some(false)); assert!(delivery.result_text.is_none());
    let source = store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    assert_eq!(source.events.iter().filter(|event|event.kind.0 == "message/assistant-projected" && source.event_payloads.get(event.event_id.as_ref())
        .is_some_and(|payload|payload["origin"] == "directed_result_failure")).count(),0,"user cancellation is not a new failure announcement");
    assert_eq!(source.events.iter().filter(|event|event.kind.0 == "message/user-accepted" && source.event_payloads.get(event.event_id.as_ref())
        .and_then(|payload|payload.pointer("/input_source/kind")).and_then(Value::as_str) == Some("directed_delegation_result")).count(),1);
    assert_eq!(source.events.iter().filter(|event|event.kind.0 == "message/assistant-projected" && source.event_payloads.get(event.event_id.as_ref())
        .is_some_and(|payload|payload.get("delegation_report").is_some())).count(),1);
    assert!(playback.raw.lock().unwrap().is_empty(),"neither the cancelled reply nor a fallback is sent to the connected transport");
    assert!(playback.accepted_sources.lock().unwrap().is_empty());
    let completed = flow.fixture.voice_execution.get(&flow.owner,&report.execution_id).await.unwrap();
    assert_eq!(completed.execution.status,nomifun_common::AgentExecutionStatus::Completed);
    assert_eq!(completed.steps.len(),1); assert_eq!(completed.steps[0].status,nomifun_common::ExecutionStepStatus::Completed);
    assert_eq!(completed.attempts.len(),1); assert_eq!(completed.attempts[0].status,nomifun_common::ExecutionAttemptStatus::Completed);
    let desktop = &report.attempts[0]; let child_id = AgentSessionId::from(desktop.agent_session_id.clone()); let child_operation = OperationId::from(desktop.turn_operation_id.clone());
    let child_receipt = store.read_turn_receipt(&child_id,&child_operation).await.unwrap();
    assert_eq!(child_receipt.status,nomifun_agent_session::TurnReceiptStatus::Completed);
    assert_eq!(child_receipt.terminal_event.as_ref().unwrap().event_id.as_ref(),desktop.terminal_event_id);
    let child_facts = store.turn_output_facts(&child_id,&child_operation).await.unwrap();
    assert!(nomifun_agent_session::canonical_turn_public_result(&child_facts,&child_receipt).unwrap().result_text.unwrap().contains(DESKTOP_FINAL_TOKEN));
    flow.fixture.finish().await; playback.stop().await; server.close().await;
}

#[tokio::test]
async fn installed_companion_memory_create_query_cas_and_stale_revision_use_real_turns_and_absorb_input_replay() {
    let server = DirectedFlowModelServer::new().await;
    let fixture = Fixture::with_input_model("companion-memory-sdk-crud",&server.url).await;
    let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
    nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir,nomifun_skill_library::builtin_skills_corpus(),&corpus_version).await.unwrap();
    let owner = fixture.services.authoritative_user_id.to_string(); let router = &fixture._router;
    let selected_model = model(router,&server,DESKTOP_MODEL).await; let preset = desktop_preset(router,&selected_model).await;
    let companion = api(router,"/api/companion/companions",json!({"name":"Real SDK memory CRUD","character":"ink"})).await;
    let companion_id = companion["companion_id"].as_str().unwrap().to_owned();
    request(router,"PATCH",&format!("/api/companion/companions/{companion_id}"),json!({"model":selected_model})).await;
    request(router,"PUT",&format!("/api/product-agent-bindings/companion/{companion_id}"),json!({
        "selection":{"kind":"preset","preset_id":preset},"model":selected_model,"resource_selections":[]})).await;
    let desktop = fixture.services.companion_service.ensure_session_for_role(&companion_id,nomifun_companion::CompanionRole::Companion,None).await.unwrap();
    let session = AgentSessionId::from(desktop.conversation_id); let store = fixture.owner.canonical().store();
    let commands = [("memory-create-turn",MEMORY_CREATE_INPUT),("memory-query-turn",MEMORY_RECALL_INPUT),
        ("memory-correct-turn",MEMORY_CORRECT_INPUT),("memory-stale-turn",MEMORY_STALE_INPUT)];
    let mut turns = Vec::new(); let mut creation_snapshot = None;
    for (index,(key,content)) in commands.into_iter().enumerate() {
        let accepted = api(router,&format!("/api/agent-sessions/{}/turns",session.as_ref()),json!({"idempotency_key":key,"input":{"content":content}})).await;
        let operation = OperationId::from(accepted["operation_id"].as_str().unwrap());
        let receipt = wait_for_completed_turn(&fixture,&session,&operation).await;
        let (_,input) = store.turn_accepted_input(&session,&operation).await.unwrap(); assert_eq!(input.0["content"],content);
        if index == 0 {
            let snapshots = fixture.services.companion_service.list_memories(&nomifun_companion::store::MemoryFilter {
                companion_id:Some(companion_id.clone()),q:Some(MEMORY_QUERY.into()),limit:20,..Default::default()}).await.unwrap();
            assert_eq!(snapshots.len(),1); creation_snapshot = Some(snapshots[0].clone());
        }
        turns.push((operation,receipt));
    }
    let requests = server.state.requests.lock().unwrap().clone();
    let created_tool = requests.iter().find_map(|body|tool_result(body,"memory-create-write")).expect("the SDK returned the real create receipt");
    let created_model: Value = serde_json::from_str(created_tool["content"].as_str().unwrap()).unwrap();
    let created: nomifun_companion::store::CompanionMemory = creation_snapshot.expect("capture the full durable row immediately after the real create Turn");
    let mut expected_created_model = serde_json::to_value(&created).unwrap();
    for field in ["source_event_ids","source_message_refs"] {
        assert!(created_model.get(field).is_none(),"full lineage stays outside the Agent tool result");
        expected_created_model.as_object_mut().unwrap().remove(field);
    }
    assert_eq!(created_model,expected_created_model,"the Agent projection preserves every ordinary field, including the primary source and CAS identity");
    assert_eq!(created.content,MEMORY_INITIAL); assert_eq!(created.revision,1); assert_eq!(created.companion_id.as_deref(),Some(companion_id.as_str()));
    let queried_tool = requests.iter().find_map(|body|tool_result(body,"memory-query-recall")).expect("the next real Turn executed facts query recall");
    let queried: Value = serde_json::from_str(queried_tool["content"].as_str().unwrap()).unwrap(); assert_eq!(queried["scope"],"facts");
    let hits = queried["hits"].as_array().unwrap(); assert_eq!(hits.len(),1);
    for field in ["source_event_ids","source_message_refs"] { assert!(hits[0]["memory"].get(field).is_none()); }
    let queried_memory: nomifun_companion::store::CompanionMemory = serde_json::from_value(hits[0]["memory"].clone()).unwrap();
    assert_eq!(queried_memory.memory_id,created.memory_id); assert_eq!(queried_memory.revision,created.revision); assert_eq!(queried_memory.content,MEMORY_INITIAL);
    assert_eq!(queried_memory.source_session_id,created.source_session_id); assert_eq!(queried_memory.source_operation_id,created.source_operation_id);
    assert_eq!(server.state.memory_target.lock().unwrap().as_ref(),Some(&(queried_memory.memory_id.clone(),queried_memory.revision)),
        "the scripted model obtains its CAS target only from that actual tool result");
    let corrected_tool = requests.iter().find_map(|body|tool_result(body,"memory-correct-write")).expect("the SDK returned the real CAS correction receipt");
    let corrected_model: Value = serde_json::from_str(corrected_tool["content"].as_str().unwrap()).unwrap();
    for field in ["source_event_ids","source_message_refs"] { assert!(corrected_model.get(field).is_none()); }
    let corrected: nomifun_companion::store::CompanionMemory = serde_json::from_value(corrected_model.clone()).unwrap();
    assert_eq!(corrected.memory_id,created.memory_id); assert_eq!(corrected.revision,created.revision+1); assert_eq!(corrected.content,MEMORY_CORRECTED);
    let stale_tool = requests.iter().find_map(|body|tool_result(body,"memory-stale-write")).expect("a later real Turn attempted the old revision");
    let stale_feedback = stale_tool["content"].to_string();
    assert!(stale_feedback.contains("COMPANION_MEMORY_REVISION_CONFLICT"),
        "stale CAS is rejected by the real owner: {stale_tool}");
    let filter = nomifun_companion::store::MemoryFilter {companion_id:Some(companion_id.clone()),q:Some(MEMORY_QUERY.into()),limit:20,..Default::default()};
    let rows = fixture.services.companion_service.list_memories(&filter).await.unwrap(); assert_eq!(rows.len(),1); let current = &rows[0];
    assert_eq!(current.memory_id,created.memory_id); assert_eq!(current.revision,corrected.revision); assert_eq!(current.content,MEMORY_CORRECTED);
    let mut expected_corrected_model = serde_json::to_value(current).unwrap();
    for field in ["source_event_ids","source_message_refs"] { expected_corrected_model.as_object_mut().unwrap().remove(field); }
    assert_eq!(corrected_model,expected_corrected_model);
    assert_ne!(current.content,MEMORY_STALE);
    let effects = store.list_effects(&session).await.unwrap();
    let writes = effects.iter().filter(|effect|effect.capability_module.as_ref() == "companion.memory" && effect.action_id.as_ref() == "companion.memory/write").collect::<Vec<_>>();
    assert_eq!(writes.len(),3,"create, correction and rejected stale correction each have their own exact canonical Effect");
    let creation = writes.iter().find(|effect|effect.turn_id == turns[0].0).unwrap();
    assert_eq!(creation.state,nomifun_agent_session::AgentEffectState::Returned);
    assert_eq!(created.source_session_id.as_deref(),Some(session.as_ref()));
    assert_eq!(created.source_operation_id.as_deref(),Some(creation.operation_id.as_ref()));
    assert_ne!(creation.operation_id,turns[0].0,"tool operation and parent Turn are different canonical identities");
    let (source_root,source_input) = store.turn_accepted_input(&session,&creation.turn_id).await.unwrap();
    assert_eq!(source_root.kind.0,"message/user-accepted"); assert_eq!(source_input.0["content"],MEMORY_CREATE_INPUT);
    assert_eq!(source_root.correlation_id.as_ref(),source_root.event_id.as_ref(),"the exact accepted root addresses the canonical user message");
    assert_ne!(created.source_operation_id.as_deref(),Some(source_root.event_id.as_ref()));
    assert!(created.source_event_ids.iter().all(|event|event != source_root.event_id.as_ref()),"collector event references cannot masquerade as canonical Agent events");
    assert!(created.source_message_refs.iter().any(|reference|reference.session_id == session.as_ref() && reference.message_id == source_root.event_id.as_ref()),
        "the actual SDK Host attaches the creating human message, rather than reconstructing it from an opaque operation");
    for reference in &created.source_message_refs {
        assert_eq!(reference.session_id,session.as_ref()); assert_eq!(reference.message_id,source_root.event_id.as_ref());
    }
    assert_eq!(created.source_message_refs.len(),1);
    let correction = writes.iter().find(|effect|effect.turn_id == turns[2].0).unwrap();
    assert_eq!(correction.state,nomifun_agent_session::AgentEffectState::Returned); assert_ne!(correction.operation_id,creation.operation_id);
    let (correction_root,correction_input) = store.turn_accepted_input(&session,&correction.turn_id).await.unwrap();
    assert_eq!(correction_input.0["content"],MEMORY_CORRECT_INPUT);
    assert_eq!(correction_root.correlation_id.as_ref(),correction_root.event_id.as_ref());
    let expected_sources = [nomifun_companion::store::CompanionMemoryMessageSource {session_id:session.as_ref().into(),message_id:source_root.event_id.as_ref().into()},
        nomifun_companion::store::CompanionMemoryMessageSource {session_id:session.as_ref().into(),message_id:correction_root.event_id.as_ref().into()}]
        .into_iter().collect::<BTreeSet<_>>();
    assert_eq!(corrected.source_session_id,created.source_session_id); assert_eq!(corrected.source_operation_id,created.source_operation_id);
    assert_eq!(current.source_session_id,created.source_session_id); assert_eq!(current.source_operation_id,created.source_operation_id);
    assert_eq!(current.source_event_ids,created.source_event_ids);
    assert_eq!(current.source_message_refs.iter().cloned().collect::<BTreeSet<_>>(),expected_sources,
        "the full durable row preserves creator provenance and unions the real correction root so later deletion can retract both sources");
    let current_turn_request = requests.iter().find(|body|last_user_text(body).contains("MEMORY_CRUD_STALE")).unwrap();
    let current_system = current_turn_request["messages"].as_array().unwrap().iter().filter(|message|message["role"] == "system")
        .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).collect::<Vec<_>>().join("\n");
    assert!(current_system.contains(MEMORY_CORRECTED),"a later model call reads the corrected current shared fact");
    assert!(!current_system.contains(MEMORY_INITIAL),"the current-fact slot cannot retain the superseded value");
    let calls = requests.iter().flat_map(|body|body["messages"].as_array().unwrap()).flat_map(|message|
        message["tool_calls"].as_array().into_iter().flatten()).filter(|call|matches!(call["id"].as_str(),Some("memory-create-write"|"memory-correct-write"|"memory-stale-write"))).collect::<Vec<_>>();
    assert!(!calls.is_empty());
    for call in calls {
        let arguments: Value = serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
        for host_field in ["source_session_id","source_operation_id","source_event_ids","source_message_refs"] { assert!(arguments.get(host_field).is_none()); }
        if call["id"] != "memory-create-write" { assert_eq!(arguments["memory_id"],created.memory_id); assert_eq!(arguments["expected_revision"],created.revision); }
    }
    let request_count = requests.len();
    for index in [0_usize,2,3] {
        let (key,content) = commands[index];
        let replay = api(router,&format!("/api/agent-sessions/{}/turns",session.as_ref()),json!({"idempotency_key":key,"input":{"content":content}})).await;
        assert_eq!(replay["operation_id"],json!(turns[index].0));
        assert_eq!(store.read_turn_receipt(&session,&turns[index].0).await.unwrap(),turns[index].1);
    }
    assert_eq!(server.state.requests.lock().unwrap().len(),request_count,"replayed roots do not re-enter the SDK or repeat a memory mutation");
    let after = fixture.services.companion_service.list_memories(&filter).await.unwrap(); assert_eq!(after,rows);
    let after_effects = store.list_effects(&session).await.unwrap();
    assert_eq!(after_effects.iter().filter(|effect|effect.action_id.as_ref() == "companion.memory/write").count(),3);
    let facts = store.chat_causality_facts(&session,&turns[0].0).await.unwrap();
    assert_eq!(facts.events.iter().filter(|event|event.kind.0 == "message/user-accepted").count(),4);
    let corrected_reader = api(router,&format!("/api/companion/companions/{companion_id}/companion/threads"),json!({
        "new_session":true,"creation_key":"memory-after-correction-reader","title":"Corrected preference reader"})).await;
    let corrected_reader_id = AgentSessionId::from(corrected_reader["conversation_id"].as_str().unwrap());
    assert_ne!(corrected_reader_id,session);
    let corrected_probe = "MEMORY_SIBLING_AFTER_CORRECTION: Continue with this companion's current saved preference.";
    let corrected_read_turn = api(router,&format!("/api/agent-sessions/{}/turns",corrected_reader_id.as_ref()),json!({
        "idempotency_key":"memory-read-after-correction","input":{"content":corrected_probe}})).await;
    wait_for_completed_turn(&fixture,&corrected_reader_id,&corrected_read_turn["operation_id"].as_str().unwrap().into()).await;
    let reader_requests = server.state.requests.lock().unwrap().clone();
    let corrected_request = reader_requests.iter().find(|body|last_user_text(body).contains("MEMORY_SIBLING_AFTER_CORRECTION")).unwrap();
    let corrected_system = corrected_request["messages"].as_array().unwrap().iter().filter(|message|message["role"] == "system")
        .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).collect::<Vec<_>>().join("\n");
    assert!(corrected_system.contains(MEMORY_CORRECTED)); assert!(!corrected_system.contains(MEMORY_INITIAL));
    assert!(!corrected_system.contains(MEMORY_CREATE_INPUT),"a sibling's automatic raw context cannot reintroduce the source of a corrected fact");
    request(router,"DELETE",&format!("/api/companion/memories/{}?companion_id={companion_id}&expected_revision={}",current.memory_id,current.revision),json!({})).await;
    assert!(fixture.services.companion_service.list_memories(&filter).await.unwrap().is_empty());
    let deleted_reader = api(router,&format!("/api/companion/companions/{companion_id}/companion/threads"),json!({
        "new_session":true,"creation_key":"memory-after-deletion-reader","title":"Deleted preference reader"})).await;
    let deleted_reader_id = AgentSessionId::from(deleted_reader["conversation_id"].as_str().unwrap());
    assert_ne!(deleted_reader_id,session); assert_ne!(deleted_reader_id,corrected_reader_id);
    let deleted_probe = "MEMORY_SIBLING_AFTER_DELETION: Continue after removing the saved preference, without restoring withdrawn facts.";
    let deleted_read_turn = api(router,&format!("/api/agent-sessions/{}/turns",deleted_reader_id.as_ref()),json!({
        "idempotency_key":"memory-read-after-deletion","input":{"content":deleted_probe}})).await;
    wait_for_completed_turn(&fixture,&deleted_reader_id,&deleted_read_turn["operation_id"].as_str().unwrap().into()).await;
    let deleted_requests = server.state.requests.lock().unwrap().clone();
    let deleted_request = deleted_requests.iter().find(|body|last_user_text(body).contains("MEMORY_SIBLING_AFTER_DELETION")).unwrap();
    let deleted_system = deleted_request["messages"].as_array().unwrap().iter().filter(|message|message["role"] == "system")
        .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).collect::<Vec<_>>().join("\n");
    for withdrawn in [MEMORY_INITIAL,MEMORY_CORRECTED,MEMORY_CREATE_INPUT,MEMORY_CORRECT_INPUT] {
        assert!(!deleted_system.contains(withdrawn),"a fresh sibling cannot restore deleted memory through automatic raw context: {withdrawn}");
    }
    assert_eq!(store.turn_accepted_input(&session,&turns[0].0).await.unwrap().1.0["content"],MEMORY_CREATE_INPUT);
    assert_eq!(store.turn_accepted_input(&session,&turns[2].0).await.unwrap().1.0["content"],MEMORY_CORRECT_INPUT,
        "forgetting shared memory preserves the original Session's real conversation record");
    let requests_before_deleted_replay = server.state.requests.lock().unwrap().len();
    let deleted_replay = api(router,&format!("/api/agent-sessions/{}/turns",session.as_ref()),json!({
        "idempotency_key":commands[0].0,"input":{"content":commands[0].1}})).await;
    assert_eq!(deleted_replay["operation_id"],json!(turns[0].0));
    assert_eq!(server.state.requests.lock().unwrap().len(),requests_before_deleted_replay);
    assert!(fixture.services.companion_service.list_memories(&filter).await.unwrap().is_empty(),"absorbing the original root cannot recreate a deleted memory");
    assert_eq!(fixture.services.authoritative_user_id.as_ref(),owner);
    fixture.finish().await; server.close().await;
}

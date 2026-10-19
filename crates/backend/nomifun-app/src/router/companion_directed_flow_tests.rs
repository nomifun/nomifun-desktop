//! One Companion Agent through independent desktop and physical-entry Sessions.
//! Installed Runtime, Kernel, real controlled HTTP research, canonical Effects
//! and source-bound background reports. Simulated motion proves transport
//! acceptance, never firmware completion or audible playback.
use super::*;
use nomifun_agent_contracts::OperationId;
use nomifun_api_types::SendMessageRequest;
use axum::response::IntoResponse;
#[path = "companion_binding_evolution_tests.rs"]
mod companion_binding_evolution_tests;
#[path = "companion_voice_contract_evolution_tests.rs"]
mod companion_voice_contract_evolution_tests;
#[path = "robot_background_control_tests.rs"]
mod robot_background_control_tests;
#[path = "companion_execution_question_tests.rs"]
mod companion_execution_question_tests;
#[path = "robot_request_playback_tests.rs"]
mod robot_request_playback_tests;
const COMPANION_MODEL:&str="companion-shared-fixture";
const SEARCH_MODEL:&str="configured-research-fixture";
const WEATHER_QUERY:&str="北京今天的天气怎么样";
const WEATHER_RESULT:&str="受控天气数据：北京今天晴，18～26℃，数据标识 WEATHER-HTTP-73921。";
const WEATHER_SOURCE:&str="https://weather.example.test/beijing/today";
const SILENT_ACTION_QUERY:&str="坐下";
const SILENT_ACTION_PAIR_QUERY:&str="先坐下，再站起来";
const HELD_INPUT:&str="HELD_INTERACTIVE_WORK";
const BACKGROUND_INPUT:&str="BACKGROUND_REQUEST: Run an isolated background report and let me continue chatting. SOURCE-CONSTRAINT-48173: preserve existing files and ask before sending the report.";
const BACKGROUND_SOURCE_CONSTRAINT:&str="SOURCE-CONSTRAINT-48173";
const BACKGROUND_GOAL:&str="BACKGROUND_WORK: Prepare the report in isolation and return the verified result.";
const BACKGROUND_RESULT:&str="BACKGROUND-CANONICAL-73921: The isolated report is ready.";
const FOLLOWUP_INPUT:&str="FOLLOWUP_WHILE_BACKGROUND: Answer this independent message.";
const FILE_INPUT:&str="FILE_ROUNDTRIP: Write the exact random marker and read it back from our workspace.";
const PROCESS_INPUT:&str="NATIVE_PROCESS_OUTPUT: Run the authorized local echo program and verify its real output.";
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
    requests:Mutex<Vec<Value>>, searches:Mutex<Vec<Value>>, hold_work:bool, question_mode:bool,
    hold_research:std::sync::atomic::AtomicBool,
    held_entered:tokio::sync::Notify, held_release:tokio::sync::Notify,
    memory_target:Mutex<Option<(String,i64)>>, file_marker:String,
}
struct CompanionFlowModelServer {url:String,state:Arc<ModelState>,task:tokio::task::JoinHandle<()>}
fn tool_available(body:&Value,name:&str)->bool {
    body["tools"].as_array().is_some_and(|tools|tools.iter().any(|tool|tool["function"]["name"]==name))
}
fn tool_result<'a>(body:&'a Value,id:&str)->Option<&'a Value> {
    let messages=body["messages"].as_array()?;
    let root=messages.iter().rposition(|message|message["role"]=="user")?;
    messages[root+1..].iter().rev().find(|message|message["role"]=="tool"&&message["tool_call_id"]==id)
}
fn last_user_text(body:&Value)->String {
    body["messages"].as_array().unwrap().iter().rev().find(|message|message["role"]=="user")
        .map(|message|message["content"].as_str().map(str::to_owned).unwrap_or_else(||message["content"].to_string())).unwrap_or_default()
}
fn tool_call(name:&str,id:&str,arguments:Value)->(Value,&'static str) {
    (json!({"role":"assistant","tool_calls":[{"index":0,"id":id,"type":"function",
        "function":{"name":name,"arguments":arguments.to_string()}}]}),"tool_calls")
}
fn selected_tool(body:&Value,name:&str,id:&str,action:&str,arguments:Value)->(Value,&'static str) {
    if tool_available(body,name) {tool_call(name,id,arguments)}
    else {tool_call("ToolSearch",&format!("{id}-find"),json!({"query":action}))}
}
fn native_echo_launch(marker:&str)->Value {
    #[cfg(windows)]
    let (command,args)={
        let command=std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows provides its system root"))
            .join("System32").join("cmd.exe");
        assert!(command.is_file(),"the process fixture uses the actual Windows command interpreter");
        (command,json!(["/d","/c","echo",marker]))
    };
    #[cfg(unix)]
    let (command,args)=(std::path::PathBuf::from("/bin/sh"),json!(["-c",format!("printf '%s' '{marker}'")]));
    #[cfg(not(any(windows,unix)))]
    let (command,args)=(std::path::PathBuf::from("unsupported-platform"),json!([]));
    json!({"command":command,"args":args,"execution_identity":"current_user","tty":false,
        "wait_ms":0,"timeout_ms":5000})
}
fn record_fixture_cost(case:&str,requests:&[Value],flow:&PhysicalEntryFixture) {
    let calls=requests.iter().enumerate().map(|(index,body)|json!({
        "call":index+1,
        "scope":if last_user_text(body).contains(BACKGROUND_GOAL) {"isolated_attempt"}else {"interactive_session"},
        "visible_tools":body["tools"].as_array().map_or(0,Vec::len),
        "tools_json_bytes":serde_json::to_vec(&body["tools"]).unwrap().len(),
        "messages_json_bytes":serde_json::to_vec(&body["messages"]).unwrap().len(),
        "request_json_bytes":serde_json::to_vec(body).unwrap().len(),
    })).collect::<Vec<_>>();
    eprintln!("{}",json!({"schema":"nomifun.unified-agent-fixture-cost.v1","case":case,
        "evidence":"local-controlled-http-fixture","model_calls":requests.len(),"calls":calls,
        "research_owner":if flow.fixture.services.test_wave1_search.is_some() {
            "controlled host SearchProvider through the formal Wave1 Kernel; not installed Browser acceptance"
        } else {"no controlled research owner installed for this case"},
        "canonical_session_open_us":flow.canonical_session_open_us,
        "desktop_canonical_session_open_us":flow.desktop_canonical_session_open_us,
        "session_open_measurement":"monotonic wall time around each fresh ensure_session_for_role call; excludes fixture App startup",
        "runtime_preparation_us":null,"runtime_preparation_measurement":"not measured separately",
        "hardware_speech_latency":"not measured; no utterance or audible sink clock in this fixture"}));
}
impl CompanionFlowModelServer {
    async fn new()->Self {Self::build(false,false).await}
    async fn with_held_work()->Self {Self::build(true,false).await}
    async fn with_question_mode()->Self {Self::build(false,true).await}
    async fn build(hold_work:bool,question_mode:bool)->Self {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}/v1",listener.local_addr().unwrap());
        let state=Arc::new(ModelState {hold_work,question_mode,file_marker:format!("WORKSPACE-{}",uuid::Uuid::now_v7()),..Default::default()});
        let router=axum::Router::new()
            .route("/v1/models",axum::routing::get(||async {axum::Json(json!({"object":"list","data":[]}))}))
            .route("/v1/responses",axum::routing::post(|
                axum::extract::State(state):axum::extract::State<Arc<ModelState>>,axum::Json(body):axum::Json<Value>,
            |async move {
                assert_eq!(body["model"],SEARCH_MODEL);assert_eq!(body["input"],WEATHER_QUERY);
                assert_eq!(body["tools"],json!([{"type":"web_search"}]));state.searches.lock().unwrap().push(body);
                if state.hold_research.load(std::sync::atomic::Ordering::SeqCst) {
                    state.held_entered.notify_one();state.held_release.notified().await;
                }
                axum::Json(json!({"output":[
                    {"type":"web_search_call","action":{"sources":[{"id":"weather-source","url":WEATHER_SOURCE,"title":"受控天气来源"}]}},
                    {"type":"message","content":[{"type":"output_text","text":WEATHER_RESULT}]}
                ]}))
            }))
            .route("/v1/chat/completions",axum::routing::post(|
                axum::extract::State(state):axum::extract::State<Arc<ModelState>>,axum::Json(body):axum::Json<Value>,
            |async move {
                assert_eq!(body["model"],COMPANION_MODEL,"every entry and Attempt uses the one selected main model");
                let ordinal={let mut requests=state.requests.lock().unwrap();requests.push(body.clone());requests.len()};
                let user=last_user_text(&body);
                if state.hold_work&&(user.contains(HELD_INPUT)||user.contains(BACKGROUND_GOAL)) {
                    state.held_entered.notify_one();state.held_release.notified().await;
                }
                let (delta,reason)=if let Some(step)=memory_crud_step(&body,&state,&user) {step}
                else if let Some(step)=companion_execution_question_tests::model_step(&body,&state,&user) {step}
                else if let Some(step)=robot_background_control_tests::correction_step(&body,&state,&user) {step}
                else if user.contains(WEATHER_QUERY) {
                    if let Some(result)=tool_result(&body,"weather-search") {
                        let content=result["content"].as_str().expect("the search observation must reach the model as tool text");
                        assert_ne!(result.get("is_error").and_then(Value::as_bool),Some(true),
                            "weather-search returned an actual failed tool observation: {content}");
                        // Chat wire tool messages do not carry the Runtime's
                        // is_error flag. A plain Kernel failure is not a JSON
                        // search answer; preserve that diagnostic explicitly.
                        let result:Value=serde_json::from_str(content).unwrap_or_else(|error|
                            panic!("weather-search did not return successful canonical search JSON: {content}; JSON error: {error}"));
                        assert_eq!(result["answer"],WEATHER_RESULT,"the real research HTTP response supplies the answer");
                        assert_eq!(result["results"][0]["url"],WEATHER_SOURCE);
                        let citation=result["results"][0]["citation_markdown"].as_str().unwrap();
                        (json!({"role":"assistant","content":format!("{WEATHER_RESULT} 来源：{citation}")}),"stop")
                    } else {
                        let mut call=selected_tool(&body,"web_search","weather-search","web.research/search",json!({"query":WEATHER_QUERY,"limit":1}));
                        if state.hold_research.load(std::sync::atomic::Ordering::SeqCst)
                            &&call.0.pointer("/tool_calls/0/function/name").and_then(Value::as_str)==Some("web_search") {
                            call.0["content"]=json!("正在查询原始天气来源。");
                        }
                        call
                    }
                } else if user.contains(FILE_INPUT) {
                    if let Some(result)=tool_result(&body,"file-read") {
                        let observed:Value=serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
                        assert_eq!(observed["content"],state.file_marker,"read_file returns the real owner's exact bytes");
                        assert_eq!(observed["path"],"physical-proof.txt");
                        assert_eq!(observed["eof"],true);
                        assert!(tool_available(&body,"report_completion"),"the Runtime advertises formal completion after the real read");
                        tool_call("report_completion","file-completion",json!({
                            "summary":format!("The workspace contains {}",state.file_marker),
                            "criteria":[{"disposition":"supported","evidence_paths":["physical-proof.txt"],
                                "rationale":"The original workspace read returned the exact marker after its write."}]
                        }))
                    } else if tool_result(&body,"file-write").is_some() {
                        selected_tool(&body,"read_file","file-read","workspace.files/read",json!({"path":"physical-proof.txt"}))
                    } else {selected_tool(&body,"write_file","file-write","workspace.files/write",json!({"path":"physical-proof.txt","content":state.file_marker}))}
                } else if user.contains(PROCESS_INPUT) {
                    if let Some(result)=tool_result(&body,"native-process-poll") {
                        let output:Value=serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
                        assert_eq!(output["state"],"exited");assert_eq!(output["success"],true);
                        assert_eq!(output["exit_code"],0);assert_eq!(output["cleanup"]["reaped"],true);
                        assert_eq!(output["output"]["text"].as_str().unwrap().trim(),state.file_marker);
                        assert!(tool_available(&body,"report_completion"),"the Runtime advertises formal completion after the original process settles");
                        tool_call("report_completion","native-process-completion",json!({
                            "summary":format!("The real native process output is verified: {}",state.file_marker),
                            "criteria":[{"disposition":"supported","evidence_call_ids":["native-process-poll"],
                                "rationale":"The original process poll returned the exact output, exit code zero, and proven child cleanup."}]
                        }))
                    } else if let Some(result)=tool_result(&body,"native-process-start") {
                        let started:Value=serde_json::from_str(result["content"].as_str().unwrap()).unwrap();
                        let process=started["process_id"].as_str().expect("the actual process owner supplies the opaque identity");
                        selected_tool(&body,"poll_process","native-process-poll","workspace.process/poll",
                            json!({"process_id":process,"cursor":0,"wait_ms":30000}))
                    } else {selected_tool(&body,"start_process","native-process-start","workspace.process/start",native_echo_launch(&state.file_marker))}
                } else if user.contains(BACKGROUND_GOAL) {
                    (json!({"role":"assistant","content":BACKGROUND_RESULT}),"stop")
                } else if user.contains(BACKGROUND_INPUT) {
                    if tool_result(&body,"background-fork").is_some() {(json!({"role":"assistant","content":"The work is accepted. You can continue here."}),"stop")}
                    else {selected_tool(&body,"fork_agent","background-fork","agent/fork",json!({"goal":BACKGROUND_GOAL}))}
                } else if matches!(user.trim(),SILENT_ACTION_QUERY|SILENT_ACTION_PAIR_QUERY) {
                    assert!(tool_available(&body,"robot_dog_action"));assert!(tool_available(&body,"finish_without_reply"));
                    let mut calls=vec![json!({"index":0,"id":"silent-motion-sit","type":"function",
                        "function":{"name":"robot_dog_action","arguments":json!({"action":"sitdown"}).to_string()}})];
                    if user.trim()==SILENT_ACTION_PAIR_QUERY {calls.push(json!({"index":1,"id":"silent-motion-stand","type":"function",
                        "function":{"name":"robot_dog_action","arguments":json!({"action":"standup"}).to_string()}}));}
                    calls.push(json!({"index":calls.len(),"id":"silent-motion-finish","type":"function",
                        "function":{"name":"finish_without_reply","arguments":"{}"}}));
                    (json!({"role":"assistant","tool_calls":calls}),"tool_calls")
                } else {(json!({"role":"assistant","content":if user.contains(BACKGROUND_GOAL) {BACKGROUND_RESULT}else {"The independent message is answered."}}),"stop")};
                let chunk=json!({"id":format!("shared-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":COMPANION_MODEL,
                    "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                let end=json!({"id":format!("shared-{ordinal}"),"object":"chat.completion.chunk","created":1,"model":COMPANION_MODEL,
                    "choices":[{"index":0,"delta":{},"finish_reason":reason}]});
                ([(axum::http::header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")).into_response()
            })).with_state(state.clone());
        let task=tokio::spawn(async move {axum::serve(listener,router).await.unwrap();});Self {url,state,task}
    }
    async fn close(self) {self.task.abort();let _=self.task.await;}
}
fn memory_crud_step(body: &Value, state: &ModelState, user: &str) -> Option<(Value,&'static str)> {
    if body["model"] != COMPANION_MODEL || !user.contains("MEMORY_CRUD_") { return None; }
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

async fn request(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", "cleanup-fixture-local-trust").header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{method} {path}: {status}: {value}"); value["data"].clone()
}

async fn model(router:&axum::Router,server:&CompanionFlowModelServer)->Value {
    let provider=api(router,"/api/providers",json!({"platform":"custom","name":"One Companion main model","base_url":server.url,
        "auth_scheme":"bearer","credentials":{"api_keys":["local-shared-fixture"]},"enabled":true,
        "initial_model":{"model":COMPANION_MODEL,"enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.chat_text",
            "connection_role":"default","output_limit":4096}]}})).await;
    json!({"provider_id":provider["provider_id"],"model":COMPANION_MODEL})
}
async fn shared_preset(router:&axum::Router,model:&Value,research:bool)->String {
    shared_preset_with_process(router,model,research,false).await
}
async fn shared_preset_with_process(router:&axum::Router,model:&Value,research:bool,process:bool)->String {
    let editor=api(router,"/api/agent-presets/from-template/chat.minimal",json!({"reuse_existing":false,
        "display_name":"Unified Companion fixture","model":model})).await;
    let preset=editor["preset"]["preset_id"].as_str().unwrap().to_owned();let mut draft=editor["draft"].clone();
    draft["document"]["enabled_capabilities"]=json!([
        {"capability":{"id":"companion"},"action_allowlist":["companion/evolve","companion/learn"]},
        {"capability":{"id":"companion.memory"},"action_allowlist":["companion.memory/recall","companion.memory/write"]},
        {"capability":{"id":"workspace.files"},"action_allowlist":["workspace.files/read","workspace.files/write"]},
        {"capability":{"id":"robot"},"action_allowlist":["robot/device","robot/display","robot/motion","robot/vision"]},
        {"capability":{"id":"session.management"},"action_allowlist":["session.management/execution_answer_decision"]},
        {"capability":{"id":"agent.collaboration"},"action_allowlist":["agent/delegate","agent/fork","agent/request_user_decision"]}
    ]);
    if research {
        draft["document"]["enabled_capabilities"].as_array_mut().unwrap().retain(|item|item["capability"]["id"]!="agent.collaboration");
        draft["document"]["enabled_capabilities"].as_array_mut().unwrap().push(json!({"capability":{"id":"web.research"},"action_allowlist":["web.research/search","web.research/fetch"]}));
    }
    if process {
        draft["document"]["enabled_capabilities"].as_array_mut().unwrap().retain(|item|item["capability"]["id"]!="agent.collaboration");
        draft["document"]["enabled_capabilities"].as_array_mut().unwrap().push(json!({"capability":{"id":"workspace.process"},
            "action_allowlist":["workspace.process/start","workspace.process/poll"]}));
    }
    api(router,&format!("/api/agent-presets/{preset}/revisions"),json!({"expected_current_revision":draft["current_revision"],"draft":draft,
        "reason":"one authored Companion Agent for every entry"})).await;preset
}
async fn install_search_provider(router:&axum::Router,server:&CompanionFlowModelServer) {
    api(router,"/api/providers",json!({"platform":"openai","name":"Controlled research HTTP owner","base_url":server.url,
        "auth_scheme":"bearer","credentials":{"api_keys":["research-fixture-only"]},"enabled":true,
        "initial_model":{"model":SEARCH_MODEL,"enabled":true,"capabilities":[{"task":"chat","traits":[],"protocol":"openai.responses",
            "connection_role":"default","endpoint":"/v1/responses","provider_params":{}}]}})).await;
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

struct PhysicalEntryFixture {
    fixture: Fixture,
    owner: String,
    companion_id: String,
    robot_id: AgentSessionId,
    desktop_id: AgentSessionId,
    device_id: String,
    connection: String,
    canonical_session_open_us: u128,
    desktop_canonical_session_open_us: u128,
    _device_rx: Option<tokio::sync::mpsc::Receiver<nomifun_robot::link::Frame>>,
    motion_requests: Arc<Mutex<Vec<Value>>>,
    motion_responder: Option<tokio::task::JoinHandle<()>>,
}

impl PhysicalEntryFixture {
    async fn new(server: &CompanionFlowModelServer, scenario: &str) -> Self {
        Self::with_research(server, scenario, false).await
    }

    async fn with_research(server: &CompanionFlowModelServer, scenario: &str, research: bool) -> Self {
        Self::with_options(server,scenario,research,false).await
    }

    async fn with_motion(server: &CompanionFlowModelServer, scenario: &str) -> Self {
        Self::with_options(server,scenario,false,true).await
    }

    async fn with_process(server:&CompanionFlowModelServer,scenario:&str)->Self {
        Self::with_process_options(server,scenario,false,false,true).await
    }

    async fn with_options(server: &CompanionFlowModelServer, scenario: &str, research: bool, motion: bool) -> Self {
        Self::with_process_options(server,scenario,research,motion,false).await
    }

    async fn with_process_options(server:&CompanionFlowModelServer,scenario:&str,research:bool,motion:bool,process:bool)->Self {
        let fixture = if research {
            let search=nomifun_ai_agent::web_search::OpenAiResponsesSearchProvider::new(
                &format!("{}/responses",server.url),"controlled-search-fixture-only".into(),SEARCH_MODEL.into()).unwrap();
            Fixture::with_input_model_and_search(scenario,&server.url,Arc::new(search)).await
        } else {Fixture::with_input_model(scenario,&server.url).await};
        let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
        nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir,nomifun_skill_library::builtin_skills_corpus(),&corpus_version).await.unwrap();
        assert!(fixture.services.skill_paths.builtin_skills_dir.is_dir(),
            "this full product fixture must materialize its actual shared builtin Skill owner before either entry is created");
        let owner = fixture.services.authoritative_user_id.to_string(); let router = &fixture._router;
        let shared_model = model(router,server).await;
        let preset = if process {shared_preset_with_process(router,&shared_model,research,true).await}
            else {shared_preset(router,&shared_model,research).await};
        let companion = api(router,"/api/companion/companions",json!({"name":scenario,"character":"ink"})).await;
        let companion_id = companion["companion_id"].as_str().unwrap().to_owned();
        request(router,"PATCH",&format!("/api/companion/companions/{companion_id}"),json!({"model":shared_model})).await;
        request(router,"PUT",&format!("/api/product-agent-bindings/companion/{companion_id}"),json!({
            "selection":{"kind":"preset","preset_id":preset},"model":shared_model,"resource_selections":[]})).await;
        let desktop_open_started=std::time::Instant::now();
        let desktop = fixture.services.companion_service.ensure_session_for_role(&companion_id,nomifun_companion::CompanionRole::Companion,None).await.unwrap();
        let desktop_canonical_session_open_us=desktop_open_started.elapsed().as_micros();
        let robots = fixture.services.robot.as_ref().unwrap();
        let (device,_) = robots.registry.upsert_on_report(nomifun_robot::registry::RobotReport {robot_id:"shared-entry-device".into(),
            client_id:"shared-entry-client".into(),board:"fixture-board".into(),firmware_version:"test".into()},1).await.unwrap();
        robots.registry.claim(device.activation_code.as_deref().unwrap(),&companion_id).await.unwrap();
        let connection = "shared-entry-original-connection".to_owned();
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
        let physical_open_started=std::time::Instant::now();
        let robot = fixture.services.companion_service.ensure_session_for_role(&companion_id,nomifun_companion::CompanionRole::Robot,Some(&device.robot_id)).await.unwrap();
        let canonical_session_open_us=physical_open_started.elapsed().as_micros();
        Self { fixture,owner,companion_id,robot_id:robot.conversation_id.into(),desktop_id:desktop.conversation_id.into(),
            device_id:device.robot_id,connection,canonical_session_open_us,desktop_canonical_session_open_us,
            _device_rx:device_rx,motion_requests,motion_responder }
    }

    fn request(&self, key: &str, text: &str) -> nomifun_robot::services::RobotTurnRequest {
        nomifun_robot::services::RobotTurnRequest {robot_id:self.device_id.clone(),companion_id:self.companion_id.clone(),
            conversation_id:self.robot_id.as_ref().into(),connection_id:self.connection.clone(),request_id:key.into(),text:text.into(),presented_question_id:None,captured_context:None}
    }

    async fn finish(self) {
        self.fixture.finish().await;
        if let Some(task)=self.motion_responder {task.abort();let _=task.await;}
    }
}


fn robot_message(text: &str) -> SendMessageRequest {
    SendMessageRequest {content:text.into(),files:vec![],inject_skills:vec![],hidden:false,
        origin:Some("robot".into()),channel_platform:Some("robot".into())}
}

async fn send_and_complete_robot_turn(fixture: &Fixture, owner: &str, request: &nomifun_robot::services::RobotTurnRequest,
    key: &str) -> OperationId {
    let accepted = fixture.owner.send_robot_session_message_idempotent(owner,request,key,robot_message(&request.text)).await.unwrap();
    assert!(!accepted.completed); assert!(!accepted.replayed,"this is a new authenticated root, not a completed delivery");
    let session=AgentSessionId::from(request.conversation_id.clone());
    let root=nomifun_agent_contracts::EventId::from(accepted.message_id);
    let store=fixture.owner.canonical().store();
    let started=tokio::time::Instant::now();
    let mut observed_delivery=None;
    let operation=loop {
        if let Some(claim)=store.session_input_claim(&session,&root).await.unwrap() {break claim.operation_id;}
        let delivery=store.session_input_delivery(&session,&root).await.unwrap();
        if observed_delivery.as_ref()!=Some(&delivery) {
            eprintln!("SESSION_INPUT_DELIVERY session={} input={} delivery={delivery}",session.as_ref(),root.as_ref());
            observed_delivery=Some(delivery.clone());
        }
        assert_ne!(delivery["state"],"blocked",
            "accepted physical input was blocked before Turn admission: session={}, input={}, delivery={delivery}",session.as_ref(),root.as_ref());
        assert!(started.elapsed()<Duration::from_secs(15),
            "accepted physical input has no canonical Turn claim: session={}, input={}, delivery={delivery}",session.as_ref(),root.as_ref());
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    wait_for_completed_turn(fixture,&session,&operation).await;
    operation
}

#[tokio::test]
async fn installed_robot_motion_finishes_silently_in_one_model_round_without_repeating_accepted_actions() {
    let server=CompanionFlowModelServer::new().await;
    let flow=PhysicalEntryFixture::with_motion(&server,"robot-silent-motion-flow").await;
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
async fn physical_weather_query_executes_real_http_research_in_its_original_session_without_delegation() {
    let server=CompanionFlowModelServer::new().await;
    let flow=PhysicalEntryFixture::with_research(&server,"unified-physical-weather",true).await;
    install_search_provider(&flow.fixture._router,&server).await;
    let store=flow.fixture.owner.canonical().store();
    let desktop_before=store.head(&flow.desktop_id).await.unwrap().last_seq;
    let sessions_before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(flow.fixture.pool()).await.unwrap();
    let projection=flow.fixture.owner.get_session(&flow.owner,flow.robot_id.as_ref()).await.unwrap();
    assert!(!projection.agent_snapshot.as_ref().unwrap().enabled_capabilities.iter().any(|item|item=="agent.collaboration"),
        "ordinary research works without any delegation or fork grant");
    let utterance=flow.request("weather-request",WEATHER_QUERY);
    let operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&utterance,"weather-input").await;
    let receipt=store.read_turn_receipt(&flow.robot_id,&operation).await.unwrap();
    let facts=store.turn_output_facts(&flow.robot_id,&operation).await.unwrap();
    let public=nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).unwrap();
    assert!(matches!(public.turn_outcome,Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {..})));
    let answer=public.result_text.unwrap();assert!(answer.contains(WEATHER_RESULT));assert!(answer.contains(WEATHER_SOURCE));
    let effects=store.list_effects(&flow.robot_id).await.unwrap();
    assert!(facts.event_payloads.values().any(|payload|matches!(
        serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(payload["event"].clone()),
        Ok(nomifun_agent_runtime::AgentEngineEvent::ToolCompleted {result,..})
            if result.call_id.as_ref()=="weather-search"&&!result.is_error)),
        "the read-only research result is a real canonical tool receipt, not an invented mutation Effect");
    assert!(effects.iter().all(|effect|effect.capability_module.as_ref()!="agent.collaboration"));
    assert_eq!(server.state.searches.lock().unwrap().len(),1,"one actual HTTP search request supplies the tool result");
    let requests=server.state.requests.lock().unwrap().clone();
    record_fixture_cost("physical_weather_direct",&requests,&flow);
    assert!(tool_available(&requests[0],"web_search"),"ordinary research is ready on the first model request");
    assert_eq!(requests.len(),2,"one tool proposal and one model consumption of the real search result");
    assert!(requests.iter().any(|body|tool_result(body,"weather-search").is_some()),"the same main model consumes the actual tool response");
    assert!(requests.iter().all(|body|body["model"]==COMPANION_MODEL));
    for body in &requests {
        assert!(!tool_available(body,"delegate_task")&&!tool_available(body,"fork_agent"));
        assert!(!body["tools"].to_string().contains("target_role"));
    }
    assert!(flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap().is_empty());
    assert_eq!(store.head(&flow.desktop_id).await.unwrap().last_seq,desktop_before);
    let sessions_after:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(sessions_after,sessions_before,"ordinary research creates neither an Attempt nor a hidden result Session");
    let request_count=requests.len();
    let replay=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&utterance,"weather-input",robot_message(WEATHER_QUERY)).await.unwrap();
    assert!(replay.replayed&&replay.completed);assert_eq!(replay.result_text.as_deref(),Some(answer.as_str()));
    assert_eq!(server.state.requests.lock().unwrap().len(),request_count);assert_eq!(server.state.searches.lock().unwrap().len(),1);
    flow.finish().await;server.close().await;
}

#[tokio::test]
async fn physical_entry_writes_and_reads_real_workspace_bytes_with_exact_effects() {
    let server=CompanionFlowModelServer::new().await;
    let flow=PhysicalEntryFixture::new(&server,"unified-physical-files").await;
    let utterance=flow.request("file-roundtrip",FILE_INPUT);
    let operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&utterance,"file-roundtrip-input").await;
    let store=flow.fixture.owner.canonical().store();
    let receipt=store.read_turn_receipt(&flow.robot_id,&operation).await.unwrap();
    let facts=store.turn_output_facts(&flow.robot_id,&operation).await.unwrap();
    let public=nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).unwrap();
    assert!(public.result_text.unwrap().contains(&server.state.file_marker));
    let projection=flow.fixture.owner.get_session(&flow.owner,flow.robot_id.as_ref()).await.unwrap();
    let root=projection.agent_snapshot.as_ref().unwrap().canonical_binding.as_ref().unwrap().typed_resource_bindings.iter()
        .find(|resource|resource.resource_kind=="workspace").unwrap().typed_parameters.get("workspace_root").unwrap();
    assert_eq!(std::fs::read_to_string(std::path::Path::new(root).join("physical-proof.txt")).unwrap(),server.state.file_marker);
    let effects=store.list_effects(&flow.robot_id).await.unwrap();
    let written=effects.iter().filter(|effect|effect.turn_id==operation&&effect.action_id.as_ref()=="workspace.files/write").collect::<Vec<_>>();
    assert_eq!(written.len(),1);assert_eq!(written[0].state,nomifun_agent_session::AgentEffectState::Returned);
    assert!(facts.event_payloads.values().any(|payload|matches!(
        serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(payload["event"].clone()),
        Ok(nomifun_agent_runtime::AgentEngineEvent::ToolCompleted {result,..})
            if result.call_id.as_ref()=="file-read"&&!result.is_error)),
        "the read uses its canonical ToolCompleted observation without manufacturing a durable mutation Effect");
    assert!(flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap().is_empty());
    let requests=server.state.requests.lock().unwrap().clone();
    record_fixture_cost("physical_file_roundtrip",&requests,&flow);
    assert!(tool_available(&requests[0],"write_file")&&tool_available(&requests[0],"read_file"));
    assert_eq!(requests.len(),3,"write, read and the answer each use one main model request");
    let calls=requests.len();
    let replay=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&utterance,"file-roundtrip-input",robot_message(FILE_INPUT)).await.unwrap();
    assert!(replay.replayed&&replay.completed);assert_eq!(server.state.requests.lock().unwrap().len(),calls);
    assert_eq!(store.list_effects(&flow.robot_id).await.unwrap().len(),effects.len());
    flow.finish().await;server.close().await;
}

#[tokio::test]
#[cfg(any(windows,unix))]
async fn physical_entry_executes_an_authorized_native_process_and_consumes_its_real_output() {
    let server=CompanionFlowModelServer::new().await;
    let flow=PhysicalEntryFixture::with_process(&server,"unified-physical-native-process").await;
    let store=flow.fixture.owner.canonical().store();
    let before=flow.fixture.owner.get_session(&flow.owner,flow.robot_id.as_ref()).await.unwrap();
    let snapshot=before.agent_snapshot.as_ref().unwrap();
    assert_eq!(snapshot.enabled_capability_actions["workspace.process"],BTreeSet::from([
        "workspace.process/start".to_owned(),"workspace.process/poll".to_owned()]));
    assert!(!snapshot.enabled_capabilities.iter().any(|capability|capability=="agent.collaboration"));
    let binding=snapshot.canonical_binding.as_ref().unwrap();
    let process=binding.typed_resource_bindings.iter().find(|resource|resource.resource_kind=="process_session").unwrap();
    let workspace=binding.typed_resource_bindings.iter().find(|resource|resource.resource_kind=="workspace").unwrap();
    assert_eq!(process.owner_id,flow.owner);assert!(process.operations.contains("execute"));
    let process_root=process.typed_parameters.get("workspace_root").expect("the process resource has an exact root");
    assert_eq!(Some(process_root),workspace.typed_parameters.get("workspace_root"),
        "the native program runs through this source Session's exact admitted process/workspace owner");
    let sessions_before:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(flow.fixture.pool()).await.unwrap();
    let desktop_before=store.head(&flow.desktop_id).await.unwrap().last_seq;
    let input=flow.request("native-output-request",PROCESS_INPUT);
    let operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&input,"native-output-input").await;
    let receipt=store.read_turn_receipt(&flow.robot_id,&operation).await.unwrap();
    let facts=store.turn_output_facts(&flow.robot_id,&operation).await.unwrap();
    let answer=nomifun_agent_session::canonical_turn_public_result(&facts,&receipt).unwrap();
    assert!(answer.result_text.unwrap().contains(&server.state.file_marker));
    let requests=server.state.requests.lock().unwrap().clone();
    assert_eq!(requests.len(),3,"start, exact process poll, and consuming the real terminal output use the same model");
    assert!(tool_available(&requests[0],"start_process")&&tool_available(&requests[0],"poll_process"));
    assert!(requests.iter().all(|request|request["model"]==COMPANION_MODEL));
    let started=tool_result(&requests[1],"native-process-start").unwrap();
    let started:Value=serde_json::from_str(started["content"].as_str().unwrap()).unwrap();
    let observed=tool_result(&requests[2],"native-process-poll").unwrap();
    let observed:Value=serde_json::from_str(observed["content"].as_str().unwrap()).unwrap();
    assert_eq!(observed["process_id"],started["process_id"],"poll must use the actual opaque process identity");
    assert_eq!(observed["state"],"exited");assert_eq!(observed["exit_code"],0);
    assert_eq!(observed["cleanup"]["reaped"],true,"a real native child exit is proved independently of the model reply");
    assert_eq!(observed["output"]["text"].as_str().unwrap().trim(),server.state.file_marker);
    let effects=store.list_effects(&flow.robot_id).await.unwrap();
    let starts=effects.iter().filter(|effect|effect.turn_id==operation&&effect.action_id.as_ref()=="workspace.process/start").collect::<Vec<_>>();
    assert_eq!(starts.len(),1);assert_eq!(starts[0].state,nomifun_agent_session::AgentEffectState::Returned);
    assert!(facts.event_payloads.values().any(|payload|matches!(
        serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(payload["event"].clone()),
        Ok(nomifun_agent_runtime::AgentEngineEvent::ToolCompleted {result,..})
            if result.call_id.as_ref()=="native-process-poll"&&!result.is_error)),
        "poll is the original native owner's canonical read observation, not another launch Effect");
    assert!(flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap().is_empty());
    assert_eq!(store.head(&flow.desktop_id).await.unwrap().last_seq,desktop_before);
    let sessions_after:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(sessions_after,sessions_before);
    let turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(turns,1,"native work stays in the original device Session without a hidden result model Turn");
    let replay=flow.fixture.owner.send_robot_session_message_idempotent(&flow.owner,&input,"native-output-input",robot_message(PROCESS_INPUT)).await.unwrap();
    assert!(replay.replayed&&replay.completed);
    assert_eq!(server.state.requests.lock().unwrap().len(),requests.len());
    assert_eq!(store.list_effects(&flow.robot_id).await.unwrap().len(),effects.len(),"replay never launches the native program twice");
    record_fixture_cost("physical_native_process_output",&requests,&flow);
    flow.finish().await;server.close().await;
}

#[tokio::test]
async fn background_execution_keeps_exact_source_grants_and_projects_once_without_a_result_model_turn() {
    let server=CompanionFlowModelServer::with_held_work().await;
    let flow=PhysicalEntryFixture::new(&server,"unified-background-source").await;
    let store=flow.fixture.owner.canonical().store();
    let original_binding=store.get_live_session(&flow.robot_id).await.unwrap().agent_binding;
    let request=flow.request("background-request",BACKGROUND_INPUT);
    let original=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&request,"background-input").await;
    tokio::time::timeout(Duration::from_secs(15),server.state.held_entered.notified()).await.unwrap();
    let executions=flow.fixture.voice_execution.list(&flow.owner,Some(20),Some(0)).await.unwrap();
    assert_eq!(executions.len(),1);let execution=&executions[0];
    let running=flow.fixture.voice_execution.get(&flow.owner,&execution.execution_id).await.unwrap();
    assert_eq!(running.attempts.len(),1,"explicit isolation opens one Attempt without a planning model");
    let child=AgentSessionId::from(running.attempts[0].conversation_id.clone().unwrap());
    assert_ne!(child,flow.robot_id);assert_ne!(child,flow.desktop_id);
    let child_binding=store.get_live_session(&child).await.unwrap().agent_binding;
    assert_eq!(child_binding.preset_revision_ref,original_binding.preset_revision_ref);
    assert_eq!(child_binding.typed_resource_bindings,original_binding.typed_resource_bindings,
        "the exact owned device and work resources survive isolation");
    let followup=flow.request("background-followup",FOLLOWUP_INPUT);
    let followup_operation=send_and_complete_robot_turn(&flow.fixture,&flow.owner,&followup,"background-followup-input").await;
    assert_ne!(followup_operation,original,"the source Session remains available while its isolated work is held");
    server.state.held_release.notify_one();
    let (event,payload)=tokio::time::timeout(Duration::from_secs(25),async {loop {
        let facts=store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
        if let Some((event,payload))=facts.events.iter().find_map(|event|facts.event_payloads.get(event.event_id.as_ref())
            .filter(|payload|event.kind.0=="message/assistant-projected"&&payload["origin"]=="agent_execution_report")
            .map(|payload|(event.clone(),payload.clone()))) {break (event,payload);}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.expect("the canonical execution event directly publishes its report");
    assert!(payload["content"].as_str().unwrap().contains(BACKGROUND_RESULT));
    let report:nomifun_agent_execution::AgentExecutionReport=serde_json::from_value(payload["delegation_report"].clone()).unwrap();
    assert_eq!(report.source.agent_session_id,flow.robot_id.as_ref());assert_eq!(report.source.turn_operation_id,original.as_ref());
    assert_eq!(report.execution_id,execution.execution_id);assert_eq!(report.attempts.len(),1);
    assert_eq!(report.attempts[0].agent_session_id,child.as_ref());
    let captured = server.state.requests.lock().unwrap().clone();
    let worker = captured.iter().find(|body| last_user_text(body).contains(BACKGROUND_GOAL))
        .expect("the isolated Attempt issued an actual main-model request");
    let instructions = worker["messages"].as_array().unwrap().iter()
        .filter(|message| matches!(message["role"].as_str(), Some("system" | "developer")))
        .map(|message| message["content"].to_string()).collect::<Vec<_>>().join("\n");
    assert!(instructions.contains(BACKGROUND_SOURCE_CONSTRAINT) && instructions.contains(original.as_ref()),
        "the child consumes the exact canonical original constraint and source Turn, even when its goal omits them");
    let effect=store.read_effect(&flow.robot_id,&report.source.effect_id).await.unwrap().unwrap();
    assert_eq!(effect.operation_id.as_ref(),report.source.tool_operation_id);assert_eq!(effect.turn_id,original);
    assert_eq!(effect.action_id.as_ref(),"agent/fork");assert_eq!(effect.state,nomifun_agent_session::AgentEffectState::Returned);
    let source_turns:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_turns WHERE session_id=?")
        .bind(flow.robot_id.as_ref()).fetch_one(flow.fixture.pool()).await.unwrap();
    assert_eq!(source_turns,2,"a report is a typed assistant projection, never a third hidden model Turn");
    let calls=server.state.requests.lock().unwrap().len();
    let replay=nomifun_agent_execution::AgentExecutionSessionPort::project_execution_report_idempotent(flow.fixture.owner.as_ref(),
        &flow.owner,flow.robot_id.as_ref(),payload["operation_id"].as_str().unwrap(),payload["content"].as_str().unwrap(),&report).await.unwrap();
    assert_eq!(replay,event.event_id.as_ref());assert_eq!(server.state.requests.lock().unwrap().len(),calls);
    let source=store.chat_causality_facts(&flow.robot_id,&original).await.unwrap();
    assert_eq!(source.events.iter().filter(|event|event.kind.0=="message/assistant-projected").count(),1);
    assert!(server.state.requests.lock().unwrap().iter().all(|body|body["model"]==COMPANION_MODEL));
    record_fixture_cost("physical_background_source",&server.state.requests.lock().unwrap(),&flow);
    flow.finish().await;server.close().await;
}

async fn start_held_interactive_turn(flow: &PhysicalEntryFixture, server: &CompanionFlowModelServer, key: &str)
    -> (AgentSessionId,OperationId) {
    // Use the real desktop Runtime without an AgentExecution receipt consumer;
    // no independent scheduler cleanup may supply the signal under test.
    let child = flow.desktop_id.clone();
    let accepted = api(&flow.fixture._router,&format!("/api/agent-sessions/{}/turns",child.as_ref()),json!({
        "idempotency_key":key,"input":{"content":HELD_INPUT},
    })).await;
    let target = OperationId::from(accepted["operation_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(15),server.state.held_entered.notified()).await.unwrap();
    (child,target)
}

#[tokio::test]
async fn dropping_an_exact_stop_waiter_after_canonical_commit_keeps_the_native_cancellation_owned() {
    let server = CompanionFlowModelServer::with_held_work().await;
    let flow = PhysicalEntryFixture::new(&server,"companion-shared-stop-waiter-drop").await;
    let (child,target) = start_held_interactive_turn(&flow,&server,"drop-stop-input").await;
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
    server.state.held_release.notify_one();
    flow.fixture.finish().await; server.close().await;
}

#[tokio::test]
async fn execution_cleanup_retries_a_canonical_cancelled_turn_that_still_has_a_live_native_generation() {
    let server = CompanionFlowModelServer::with_held_work().await;
    let flow = PhysicalEntryFixture::new(&server,"companion-shared-closed-native-cleanup").await;
    let (session,operation) = start_held_interactive_turn(&flow,&server,"closed-native-cleanup-input").await;
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
        match nomifun_agent_execution::AgentExecutionSessionPort::cancel_attempt_for_execution(flow.fixture.owner.as_ref(),
            &flow.owner,session.as_ref(),"fixture-attempt",operation.as_ref()).await {
            Ok(()) => break,
            Err(nomifun_common::AppError::Conflict(_)) => tokio::time::sleep(Duration::from_millis(10)).await,
            Err(error) => panic!("exact native cleanup failed unexpectedly: {error}"),
        }
    }}).await.expect("cleanup must retry the closed exact Turn instead of acknowledging its empty canonical head");
    assert!(flow.fixture.services.agent_runtime_sessions.active_turn_generation(session.as_ref()).is_none(),
        "cleanup acknowledgement requires retirement of the original native generation");
    assert!(flow.fixture.services.agent_runtime_sessions.get_runtime(session.as_ref()).is_none(),
        "a completed Attempt cleanup must retire the exact Runtime without closing its history");
    assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Cancelled);
    server.state.held_release.notify_one();
    flow.fixture.finish().await; server.close().await;
}

#[tokio::test]
async fn retired_attempt_cleanup_never_selects_the_successor_turn() {
    let server = CompanionFlowModelServer::with_held_work().await;
    let flow = PhysicalEntryFixture::new(&server,"companion-exact-cleanup-successor").await;
    let original_key = "k".repeat(nomifun_common::MAX_IDEMPOTENCY_KEY_LEN);
    let (session, original) = start_held_interactive_turn(&flow,&server,&original_key).await;
    let port = flow.fixture.owner.as_ref();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match nomifun_agent_execution::AgentExecutionSessionPort::cancel_attempt_for_execution(
                port,&flow.owner,session.as_ref(),"original-attempt",original.as_ref()).await {
                Ok(()) => break,
                Err(nomifun_common::AppError::Conflict(_)) => tokio::task::yield_now().await,
                Err(error) => panic!("original cleanup failed: {error}"),
            }
        }
    }).await.unwrap();
    server.state.held_release.notify_one();
    let (_, successor) = start_held_interactive_turn(&flow,&server,"cleanup-successor").await;
    let successor_generation = flow.fixture.services.agent_runtime_sessions.active_turn_generation(session.as_ref());
    assert!(successor_generation.is_some());
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_attempt_for_execution(
        port,&flow.owner,session.as_ref(),"original-attempt",original.as_ref()).await.unwrap();
    let store = flow.fixture.owner.canonical().store();
    assert_eq!(store.head(&session).await.unwrap().active_turn_id.as_deref(),Some(successor.as_ref()));
    assert_eq!(flow.fixture.services.agent_runtime_sessions.active_turn_generation(session.as_ref()),successor_generation);
    assert_eq!(store.read_turn_receipt(&session,&original).await.unwrap().status,nomifun_agent_session::TurnReceiptStatus::Cancelled);
    nomifun_agent_execution::AgentExecutionSessionPort::cancel_turn_for_execution(
        port,&flow.owner,session.as_ref(),"successor-fixture-cleanup",successor.as_ref()).await.unwrap();
    server.state.held_release.notify_one();
    flow.fixture.finish().await; server.close().await;
}

#[tokio::test]
async fn installed_companion_memory_create_query_cas_and_stale_revision_use_real_turns_and_absorb_input_replay() {
    let server = CompanionFlowModelServer::new().await;
    let fixture = Fixture::with_input_model("companion-memory-sdk-crud",&server.url).await;
    let corpus_version = nomifun_skill_library::builtin_skills_materialize_version(env!("CARGO_PKG_VERSION"));
    nomifun_skill_library::materialize_if_needed(&fixture.services.data_dir,nomifun_skill_library::builtin_skills_corpus(),&corpus_version).await.unwrap();
    let owner = fixture.services.authoritative_user_id.to_string(); let router = &fixture._router;
    let selected_model = model(router,&server).await; let preset = shared_preset(router,&selected_model,false).await;
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

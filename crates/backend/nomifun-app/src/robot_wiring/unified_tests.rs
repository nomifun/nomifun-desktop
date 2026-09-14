//! Real AppRobotBackend + ConversationService + Nomi engine against a local
//! model endpoint and a physical-protocol double. No paid provider is called.
use super::*;
use crate::{AppConfig, services::AppServices};
use axum::{body::Body, http::Request};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};
use nomifun_robot::wiring::RobotConversationBackend;

const TRUST: &str = "isolated-companion-test";

struct ExternalMcp;
impl Respond for ExternalMcp {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let Some(id) = body.get("id") else { return ResponseTemplate::new(202); };
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => serde_json::json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"companion-external","version":"1"}}),
            "tools/list" => serde_json::json!({"tools":[{"name":"echo","description":"Echo companion data","inputSchema":{"type":"object","properties":{}}}]}),
            "tools/call" => serde_json::json!({"content":[{"type":"text","text":"EXTERNAL_TOOL_OK"}]}),
            "resources/read" => serde_json::json!({"contents":[{"uri":"test://shared","mimeType":"text/plain","text":"EXTERNAL_RESOURCE_OK"}]}),
            "resources/list" => serde_json::json!({"resources":[{"uri":"test://shared","name":"shared"}]}),
            _ => serde_json::json!({}),
        };
        ResponseTemplate::new(200).set_body_json(serde_json::json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}

#[tokio::test]
async fn companion_mcp_selection_preserves_skills_and_serves_desktop_and_robot() {
    let harness = Harness::new().await;
    let external = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST")).respond_with(ExternalMcp).mount(&external).await;
    let before = harness.api("GET", &format!("/api/conversations/{}", harness.conversation_id), Value::Null, None).await;
    assert_eq!(before["data"]["extra"]["mcp_server_ids"], serde_json::json!([]));
    let server = harness.api("POST", "/api/mcp/servers", serde_json::json!({"name":"companion-external","transport":{"type":"http","url":format!("{}/mcp",external.uri())}}), None).await;
    let id = server["data"]["mcp_server_id"].as_str().unwrap();
    harness.api("POST", &format!("/api/mcp/servers/{id}/toggle"), serde_json::json!({}), None).await;
    let selected = harness.api("PUT", &format!("/api/agent-sessions/{}/mcp-selection", harness.conversation_id), serde_json::json!({"mcp_server_ids":[id]}), None).await;
    assert_eq!(selected["data"]["extra"]["skills"], before["data"]["extra"]["skills"]);
    harness.api("POST", &format!("/api/conversations/{}/warmup", harness.conversation_id), serde_json::json!({}), None).await;
    assert!(external.received_requests().await.unwrap().is_empty(), "MCP must not connect before a model tool call");
    harness.desktop("EXTERNAL_MCP desktop", "external-mcp-desktop").await;
    let mcp_calls = || async { external.received_requests().await.unwrap().into_iter().filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok()).collect::<Vec<_>>() };
    let calls = mcp_calls().await;
    assert!(calls.iter().any(|r| r["method"] == "tools/call"), "desktop must reach the selected MCP tool");
    assert!(calls.iter().any(|r| r["method"] == "resources/read"));
    harness.device("robot-mcp").await;
    harness.robot("robot-mcp", "EXTERNAL_MCP robot").await;
    assert_eq!(mcp_calls().await.iter().filter(|r| r["method"] == "tools/call").count(), 2);
    harness.api("PUT", &format!("/api/agent-sessions/{}/mcp-selection", harness.conversation_id), serde_json::json!({"mcp_server_ids":[]}), None).await;
    harness.desktop("after MCP removal", "external-mcp-removed").await;
    assert_eq!(mcp_calls().await.iter().filter(|r| r["method"] == "tools/call").count(), 2);
    harness.shutdown().await;
}

struct Model { requests: Arc<Mutex<Vec<Value>>> }
impl Respond for Model {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let mut captured = body.clone();
        captured["_test_path"] = serde_json::json!(request.url.path());
        self.requests.lock().unwrap().push(captured);
        let messages = body["messages"].as_array().unwrap();
        let last_user = messages.iter().rev().find(|message| message["role"] == "user")
            .map(|message| message["content"].to_string()).unwrap_or_default();
        if last_user.contains("FALLBACK") && request.url.path().starts_with("/primary") {
            return ResponseTemplate::new(503).set_body_json(serde_json::json!({"error":{"message":"test provider unavailable","type":"server_error"}}));
        }
        let text = if last_user.contains("LOOK") { "red image" }
            else if messages.iter().any(|message| message["content"].to_string().contains("ALPHA")) { "ALPHA" } else { "reply" };
        let needs_tool = ["MOVE", "LOOK", "SAVE_MEMORY", "RECALL_MEMORY"].iter().any(|word| last_user.contains(word))
            && messages.last().is_some_and(|message| message["role"] != "tool");
        let tool_name = if last_user.contains("SAVE_MEMORY") { "save_memory" }
            else if last_user.contains("RECALL_MEMORY") { "recall_memories" }
            else if last_user.contains("LOOK") { "robot_camera_take_photo" } else { "robot_gimbal_look" };
        let tool = body["tools"].as_array().into_iter().flatten()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .find(|name| name.contains(tool_name));
        let arguments = if last_user.contains("SAVE_MEMORY") { r#"{"kind":"preference","content":"用户喜欢乌龙茶。","tags":["drink"]}"# }
            else if last_user.contains("RECALL_MEMORY") { r#"{"query":"乌龙茶"}"# } else { r#"{"direction":"left"}"# };
        let (mut delta, mut finish) = if needs_tool && tool.is_some() {
            (serde_json::json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_move","type":"function",
                "function":{"name":tool.unwrap(),"arguments":arguments}}]}), "tool_calls")
        } else { (serde_json::json!({"role":"assistant","content":text}), "stop") };
        if last_user.contains("EXTERNAL_MCP") {
            let start = messages.iter().rposition(|m| m["role"] == "user").unwrap();
            let called = messages[start..].iter().flat_map(|m| m["tool_calls"].as_array().into_iter().flatten())
                .filter_map(|call| call["function"]["name"].as_str()).collect::<Vec<_>>();
            if let Some(next) = ["mcp_connect", "mcp_tool_proxy", "mcp_resource_read"].into_iter().find(|name| !called.contains(name)) {
                let visible = messages[start..].iter().flat_map(|m| m["tool_calls"].as_array().into_iter().flatten())
                    .any(|call| call["function"]["name"] == "ToolSearch" && call["function"]["arguments"].as_str()
                        .and_then(|args| serde_json::from_str::<Value>(args).ok()).is_some_and(|args| args["query"] == next));
                let name = if visible { next } else { "ToolSearch" };
                let args = if !visible { serde_json::json!({"query":next}) }
                    else if next == "mcp_connect" { serde_json::json!({}) }
                    else if next == "mcp_tool_proxy" { serde_json::json!({"server":"companion-external","tool":"echo","arguments":{}}) }
                    else { serde_json::json!({"server":"companion-external","uri":"test://shared"}) };
                delta = serde_json::json!({"role":"assistant","tool_calls":[{"index":0,"id":format!("call_{}", called.len()),"type":"function","function":{"name":name,"arguments":args.to_string()}}]});
                finish = "tool_calls";
            }
        }
        if needs_tool && last_user.contains("PREAMBLE") { delta["content"] = serde_json::json!("I will check the device first."); }
        let event = serde_json::json!({"id":"test","object":"chat.completion.chunk","created":1,
            "model":"step-3.7-flash","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let terminal = serde_json::json!({"id":"test","object":"chat.completion.chunk","created":1,
            "model":"step-3.7-flash","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
        let response = ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {event}\n\ndata: {terminal}\n\ndata: [DONE]\n\n"))
            ;
        if last_user.contains("SLOW") { response.set_delay(std::time::Duration::from_millis(600)) } else { response }
    }
}

struct TestSpeech;
#[async_trait::async_trait]
impl SpeechServices for TestSpeech {
    async fn transcribe(&self, _: &nomifun_robot::services::SpeechContext, _: Vec<u8>) -> anyhow::Result<String> { Ok("hello".to_owned()) }
    async fn synthesize(&self, _: &nomifun_robot::services::SpeechContext, _: &str) -> anyhow::Result<nomifun_robot::audio::AudioBuffer> {
        Ok(nomifun_robot::audio::AudioBuffer { pcm: vec![0; 1440], sample_rate: 24000 })
    }
    async fn explain_image(&self, _: &nomifun_robot::services::SpeechContext, _: Vec<u8>, _: &str) -> anyhow::Result<String> { Ok("red image".to_owned()) }
}

fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8).unwrap();
    bytes
}

struct Harness {
    router: axum::Router,
    services: AppServices,
    backend: Arc<AppRobotBackend>,
    requests: Arc<Mutex<Vec<Value>>>,
    _upstream: MockServer,
    _root: tempfile::TempDir,
    companion_id: String,
    conversation_id: String,
    provider_id: String,
}

impl Harness {
    async fn api(&self, method: &str, path: &str, body: Value, key: Option<&str>) -> Value {
        let mut request = Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST).header("content-type", "application/json");
        if let Some(key) = key { request = request.header("Idempotency-Key", key); }
        let response = self.router.clone().oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        let result: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({"body":String::from_utf8_lossy(&bytes)}));
        assert!(status.is_success(), "{method} {path}: {status}: {result}");
        result
    }

    async fn new() -> Self {
        let _ = tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "error".to_owned()))
            .with_test_writer().try_init();
        let root = tempfile::tempdir().unwrap();
        let mut services = AppServices::from_config(nomifun_db::init_database_memory().await.unwrap(), &AppConfig {
            data_dir: root.path().join("data"), work_dir: root.path().join("work"),
            auth_policy: nomifun_auth::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from(TRUST)), ..Default::default()
        }).await.unwrap();
        Arc::get_mut(services.robot.as_mut().unwrap()).expect("unmounted RobotServices has one owner").speech = Arc::new(TestSpeech);
        let router = crate::router::create_router(&services).await;
        let backend = services.robot.as_ref().unwrap().backend.get().unwrap().clone();
        let upstream = MockServer::start().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        Mock::given(wiremock::matchers::method("POST")).respond_with(Model { requests: requests.clone() }).mount(&upstream).await;
        let mut harness = Self { router, services, backend, requests, _upstream: upstream, _root: root,
            companion_id: String::new(), conversation_id: String::new(), provider_id: String::new() };
        harness.provider_id = harness.provider("primary").await;
        let created = harness.api("POST", "/api/companion/companions", serde_json::json!({"name":"Unified companion","character":"ink"}), None).await;
        harness.companion_id = created["data"]["companion_id"].as_str().unwrap().to_owned();
        harness.api("PATCH", &format!("/api/companion/companions/{}", harness.companion_id),
            serde_json::json!({"model":{"provider_id":harness.provider_id,"model":"step-3.7-flash"}}), None).await;
        harness.conversation_id = harness.backend.ensure_companion_session(&harness.companion_id).await.unwrap();
        harness
    }

    async fn provider(&self, name: &str) -> String {
        let value = self.api("POST", "/api/providers", serde_json::json!({
            "platform":"stepfun-plan", "name":name, "base_url":format!("{}/{name}/v1", self._upstream.uri()),
            "auth_scheme":"bearer", "credentials":{"api_keys":["test-only"]},"enabled":true,
            "initial_model":{"model":"step-3.7-flash","enabled":true,"capabilities":[{
                "task":"chat","traits":["function_calling","reasoning","streaming"],
                "protocol":"openai.chat_text","connection_role":"default","provider_params":{}
            }]}
        }), None).await;
        value["data"]["provider_id"].as_str().unwrap().to_owned()
    }

    async fn device(&self, robot_id: &str) -> Arc<Mutex<Vec<Value>>> {
        let (record, token) = self.backend.registry.upsert_on_report(nomifun_robot::registry::RobotReport {
            robot_id: robot_id.to_owned(), client_id: "test-device".to_owned(), board: "test".to_owned(), firmware_version: "test".to_owned(),
        }, 1).await.unwrap();
        self.backend.registry.claim(record.activation_code.as_deref().unwrap(), &self.companion_id).await.unwrap();
        let mut permissions = nomifun_robot::registry::RobotPermissions::default();
        permissions.motion = true;
        permissions.vision = true;
        self.backend.registry.set_permissions(robot_id, permissions).await.unwrap();
        self.backend.registry.connect(robot_id, &self.companion_id, "socket-1").await.unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let client = Arc::new(nomifun_robot::mcp_bridge::RobotMcpClient::new(tx, "socket-1".to_owned()));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let responder = client.clone();
        let router = self.router.clone();
        tokio::spawn(async move {
            while let Some(nomifun_robot::link::Frame::Text(frame)) = rx.recv().await {
                let value: Value = serde_json::from_str(&frame).unwrap();
                recorded.lock().unwrap().push(value.clone());
                let text = if value["payload"]["params"]["name"] == "self.camera.take_photo" {
                    let mut body = b"--picture\r\nContent-Disposition: form-data; name=\"question\"\r\n\r\nWhat is here?\r\n--picture\r\nContent-Disposition: form-data; name=\"file\"; filename=\"camera.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n".to_vec();
                    body.extend(jpeg()); body.extend(b"\r\n--picture--\r\n");
                    let response = router.clone().oneshot(Request::post("/robot/vision/explain")
                        .header("Authorization", format!("Bearer {token}"))
                        .header("content-type", "multipart/form-data; boundary=picture")
                        .body(Body::from(body)).unwrap()).await.unwrap();
                    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                    let result: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(result["success"], true, "{result}");
                    result["result"].as_str().unwrap().to_owned()
                } else { "moved".to_owned() };
                responder.handle_incoming(serde_json::json!({"jsonrpc":"2.0","id":value["payload"]["id"],
                    "result":{"content":[{"type":"text","text":text}],"isError":false}})).await;
            }
        });
        self.services.robot.as_ref().unwrap().tools.attach(robot_id, client, vec![nomifun_robot::mcp_bridge::RobotToolDescriptor {
            device_name:"self.gimbal.look".to_owned(), exposed_name:"robot_gimbal_look".to_owned(), description:"Look in a direction".to_owned(),
            input_schema:serde_json::json!({"type":"object","properties":{"direction":{"type":"string"}},"required":["direction"]}),
        }, nomifun_robot::mcp_bridge::RobotToolDescriptor {
            device_name:"self.camera.take_photo".to_owned(), exposed_name:"robot_camera_take_photo".to_owned(), description:"Take a photo".to_owned(), input_schema:serde_json::json!({"type":"object","properties":{}}),
        }]).await;
        calls
    }

    async fn desktop(&self, text: &str, key: &str) {
        self.api("POST", &format!("/api/conversations/{}/messages", self.conversation_id), serde_json::json!({"content":text}), Some(key)).await;
        self.idle().await;
    }

    async fn idle(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            while self.backend.conversations.runtime_summary_for(&self.conversation_id).await.is_processing {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }).await.expect("conversation did not settle");
    }

    async fn robot(&self, id: &str, text: &str) -> String {
        let mut stream = self.backend.dispatch(nomifun_robot::services::RobotTurnRequest {
            robot_id:id.to_owned(),companion_id:self.companion_id.clone(),conversation_id:self.conversation_id.clone(),
            connection_id:"socket-1".to_owned(),request_id:uuid::Uuid::now_v7().to_string(),text:text.to_owned(),
        }).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut text = String::new();
            while let Some(event) = stream.recv().await {
                match event { TurnEvent::Text(part) => text.push_str(&part), TurnEvent::Done => return text,
                    TurnEvent::Failed { message, .. } => panic!("robot turn failed: {message}") }
            }
            panic!("robot stream closed without completion");
        }).await.unwrap()
    }

    async fn shutdown(&self) {
        self.services.agent_runtime_registry.terminate_all();
        self.services.shutdown_background_tasks(std::time::Duration::from_secs(5)).await;
    }
}

#[tokio::test]
async fn desktop_and_robot_share_history_and_actual_device_tools() {
    let harness = Harness::new().await;
    harness.desktop("Remember ALPHA", "desktop-first").await;
    let calls = harness.device("robot-1").await;
    assert_eq!(harness.robot("robot-1", "What did I say?").await, "ALPHA");
    harness.desktop("MOVE the head", "desktop-move").await;
    assert_eq!(calls.lock().unwrap().len(), 1, "desktop model must reach the real device transport");
    assert_eq!(harness.backend.ensure_companion_session(&harness.companion_id).await.unwrap(), harness.conversation_id);
    let requests = harness.requests.lock().unwrap();
    assert!(requests.iter().any(|request| request["messages"].to_string().contains("不超过 3 句")), "voice prompt reaches model");
    assert!(requests.iter().any(|request| request["messages"].to_string().contains("回复不会自动播报")), "desktop keeps full text output");
    drop(requests);
    let second = harness.device("robot-2").await;
    harness.desktop("MOVE without a target", "ambiguous-target").await;
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert!(second.lock().unwrap().is_empty(), "multiple devices require an explicit target");
    harness.api("PATCH", &format!("/api/companion/companions/{}", harness.companion_id),
        serde_json::json!({"control_robot_id":"robot-2"}), None).await;
    harness.desktop("MOVE selected device", "selected-target").await;
    assert_eq!(second.lock().unwrap().len(), 1);
    harness.desktop("MOVE selected device", "selected-target").await;
    assert_eq!(second.lock().unwrap().len(), 1, "public replay must not execute another device action");
    harness.backend.registry.disconnect("robot-2", "socket-1").await;
    harness.backend.registry.connect("robot-2", &harness.companion_id, "socket-2").await.unwrap();
    assert_eq!(harness.backend.ensure_companion_session(&harness.companion_id).await.unwrap(), harness.conversation_id);
    let alternate = harness.provider("alternate").await;
    harness.api("PATCH", &format!("/api/companion/companions/{}", harness.companion_id),
        serde_json::json!({"model":{"provider_id":alternate,"model":"step-3.7-flash"}}), None).await;
    assert_eq!(harness.robot("robot-1", "What did I say after the model change?").await, "ALPHA");
    assert!(harness.requests.lock().unwrap().last().unwrap()["_test_path"].as_str().unwrap().starts_with("/alternate"));
    harness.api("PUT", &format!("/api/product-agent-bindings/companion/{}", harness.companion_id),
        serde_json::json!({"selection":{"kind":"template","template_key":"chat.minimal"},"conversation_id":harness.conversation_id}), None).await;
    harness.desktop("MOVE with minimal Agent", "minimal-desktop").await;
    harness.robot("robot-1", "MOVE with the same minimal Agent").await;
    assert_eq!(calls.lock().unwrap().len(), 1);
    assert_eq!(second.lock().unwrap().len(), 1, "the one Agent setting governs both entry points");
    harness.shutdown().await;
}

#[tokio::test]
async fn fallback_stays_in_one_turn_without_rewriting_the_companion_model() {
    let harness = Harness::new().await;
    harness.device("robot-1").await;
    let fallback = harness.provider("backup").await;
    harness.api("PATCH", &format!("/api/companion/companions/{}", harness.companion_id),
        serde_json::json!({"fallback_model":{"provider_id":fallback,"model":"step-3.7-flash"}}), None).await;
    assert!(harness.backend.companions.get_companion(&harness.companion_id).await.unwrap().fallback_model.is_some());
    assert_eq!(harness.robot("robot-1", "FALLBACK please").await, "reply");
    let conversation = harness.backend.conversations.get(&harness.backend.owner_user_id, &harness.conversation_id).await.unwrap();
    assert_eq!(conversation.model.unwrap().provider_id, harness.provider_id);
    let count: i64 = nomifun_db::sqlx::query_scalar("SELECT count(*) FROM messages WHERE conversation_id = ? AND position = 'right' AND json_extract(content, '$.content') = 'FALLBACK please'")
        .bind(&harness.conversation_id).fetch_one(harness.services.database.pool()).await.unwrap();
    assert_eq!(count, 1);
    harness.desktop("After recovery", "post-fallback").await;
    let requests = harness.requests.lock().unwrap();
    assert!(requests.last().unwrap()["_test_path"].as_str().unwrap().starts_with("/primary"));
    let fallback_request = requests.iter().find(|request| request["_test_path"].as_str().unwrap().starts_with("/backup")).unwrap();
    assert_eq!(fallback_request["messages"].as_array().unwrap().iter().filter(|message|
        message["role"] == "user" && message["content"].to_string().contains("FALLBACK please")).count(), 1,
        "the model history must not duplicate the user input either");
    drop(requests);
    harness.shutdown().await;
}

#[tokio::test]
async fn camera_tool_stores_its_image_and_observation_on_the_initiating_message() {
    let harness = Harness::new().await;
    let calls = harness.device("robot-1").await;
    harness.desktop("LOOK at the camera", "camera-source").await;
    assert_eq!(calls.lock().unwrap().len(), 1);
    let content: String = nomifun_db::sqlx::query_scalar("SELECT content FROM messages WHERE conversation_id = ? AND position = 'right' AND json_extract(content, '$.content') = 'LOOK at the camera'")
        .bind(&harness.conversation_id).fetch_one(harness.services.database.pool()).await.unwrap();
    let content: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(content["observations"][0]["answer"], "red image");
    let path = content["observations"][0]["image"]["path"].as_str().unwrap();
    assert!(std::path::Path::new(path).is_file());
    assert_eq!(content["interaction"]["kind"], "desktop");
    assert!(harness.backend.vision_observations.active_turn("robot-1", &harness.companion_id).is_none());
    harness.shutdown().await;
}

#[tokio::test]
async fn concurrent_devices_queue_against_the_same_history() {
    let harness = Harness::new().await;
    harness.device("robot-1").await;
    harness.device("robot-2").await;
    let (first, second) = tokio::join!(harness.robot("robot-1", "SLOW remember ALPHA"), async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        harness.robot("robot-2", "What did I say?").await
    });
    assert_eq!(first, "ALPHA");
    assert_eq!(second, "ALPHA");
    harness.shutdown().await;
}

#[tokio::test]
async fn desktop_memory_writes_are_recalled_from_the_robot_without_another_store() {
    let harness = Harness::new().await;
    harness.device("robot-1").await;
    harness.desktop("SAVE_MEMORY", "save-preference").await;
    let memories = harness.api("GET", &format!("/api/companion/memories?companion_id={}", harness.companion_id), serde_json::json!({}), None).await;
    assert!(memories.to_string().contains("乌龙茶"), "{memories}");
    harness.robot("robot-1", "RECALL_MEMORY").await;
    let requests = harness.requests.lock().unwrap();
    assert!(requests.iter().any(|request| request["messages"].as_array().unwrap().iter().any(|message|
        message["role"] == "tool" && message["content"].to_string().contains("乌龙茶"))), "robot must receive the actual memory tool result");
    drop(requests);
    harness.shutdown().await;
}

#[tokio::test]
async fn explicit_playback_reads_the_existing_reply_without_calling_the_model() {
    let harness = Harness::new().await;
    harness.device("robot-1").await;
    harness.desktop("Remember ALPHA", "playback-source").await;
    let before = harness.requests.lock().unwrap().len();
    let mut permissions = harness.backend.registry.get("robot-1").await.unwrap().permissions;
    permissions.proactive_speech = true;
    harness.backend.registry.set_permissions("robot-1", permissions).await.unwrap();
    let mut commands = harness.backend.registry.open_playback("robot-1", "socket-1").await.unwrap();
    let receiver = tokio::spawn(async move {
        let command = commands.recv().await.unwrap();
        assert_eq!(command.text, "ALPHA");
        let _ = command.accepted.send(Ok(()));
    });
    let response = harness.api("POST", "/api/robots/robot-1/speak", serde_json::json!({"conversation_id":harness.conversation_id}), None).await;
    assert_eq!(response["accepted"], true);
    receiver.await.unwrap();
    assert_eq!(harness.requests.lock().unwrap().len(), before);
    harness.shutdown().await;
}

#[tokio::test]
async fn permission_revocation_after_model_start_prevents_the_physical_action() {
    let harness = Harness::new().await;
    let calls = harness.device("robot-1").await;
    let turn = harness.desktop("SLOW MOVE", "revoked-motion");
    tokio::pin!(turn);
    tokio::select! {
        _ = &mut turn => panic!("test model must still be preparing its tool call"),
        _ = async {
            while harness.requests.lock().unwrap().is_empty() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
        } => {}
    }
    let mut permissions = harness.backend.registry.get("robot-1").await.unwrap().permissions;
    permissions.motion = false;
    harness.backend.registry.set_permissions("robot-1", permissions).await.unwrap();
    turn.await;
    assert!(calls.lock().unwrap().is_empty(), "a discovered tool must lose authority immediately when permissions change");
    harness.shutdown().await;
}

#[tokio::test]
async fn cancelling_a_queued_device_utterance_does_not_cancel_desktop_work() {
    let harness = Harness::new().await;
    harness.device("robot-1").await;
    let desktop = harness.desktop("SLOW remember ALPHA", "desktop-busy");
    tokio::pin!(desktop);
    tokio::select! {
        _ = &mut desktop => panic!("the slow desktop turn must remain active"),
        _ = async {
            while harness.requests.lock().unwrap().is_empty() { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
        } => {}
    }
    let request = nomifun_robot::services::RobotTurnRequest { robot_id:"robot-1".to_owned(),
        companion_id:harness.companion_id.clone(), conversation_id:harness.conversation_id.clone(),
        connection_id:"socket-1".to_owned(), request_id:"cancel-queued-utterance".to_owned(), text:"MUST_NOT_RUN".to_owned() };
    let mut queued = harness.backend.dispatch(request.clone()).await.unwrap();
    harness.backend.cancel(&request).await.unwrap();
    desktop.await;
    assert!(tokio::time::timeout(std::time::Duration::from_secs(3), queued.recv()).await.unwrap().is_none());
    assert!(!harness.requests.lock().unwrap().iter().any(|request| request["messages"].to_string().contains("MUST_NOT_RUN")));
    assert_eq!(harness.robot("robot-1", "What did I say?").await, "ALPHA");
    harness.backend.cancel(&request).await.unwrap();
    harness.shutdown().await;
}

#[tokio::test]
async fn fast_device_completion_speaks_only_the_final_answer_after_tools() {
    let harness = Harness::new().await;
    harness.device("robot-1").await;
    assert_eq!(harness.robot("robot-1", "MOVE PREAMBLE").await, "reply");
    harness.shutdown().await;
}

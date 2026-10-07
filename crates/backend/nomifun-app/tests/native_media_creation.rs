//! Deterministic product regressions for text-model Creation Actions. These
//! fixtures use the production Session, model protocol, Runtime, Kernel,
//! Creation worker and Workshop asset bridge. Only the provider HTTP is mocked.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use nomifun_app::compatibility::{AppServices, create_router};
use nomifun_app::{AppConfig, AuthPolicy};
use nomifun_db::{IClientPreferenceRepository, SqliteClientPreferenceRepository};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TRUST: &str = "native-media-creation-regression";
const CHAT_MODEL: &str = "step-3.7-flash";
const TEXT_PRODUCT: &str = "A short independently generated text artifact.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Media {
    Image,
    ImageEdit,
    Video,
    Audio,
    Music,
    Text,
}

impl Media {
    const ALL: [Self; 6] = [
        Self::Image,
        Self::ImageEdit,
        Self::Video,
        Self::Audio,
        Self::Music,
        Self::Text,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::ImageEdit => "image_edit",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Music => "music",
            Self::Text => "text",
        }
    }

    fn action(self) -> String {
        format!("creation.media/{}", self.label())
    }

    fn request(self) -> String {
        let request = match self {
            Self::Image => "生成一张小猫咪图片",
            Self::ImageEdit => "把附件照片的背景换成蓝色",
            Self::Video => "生成一段海浪视频",
            Self::Audio => "请把你好世界这段话朗读出来",
            Self::Music => "创作一首轻快的纯音乐",
            Self::Text => "使用已授权的文本创作动作保存一份简短文稿",
        };
        format!("{request}。MEDIA_CASE=[{}]", self.label())
    }

    fn arguments(self, reference: &str) -> Value {
        match self {
            Self::Image => json!({"prompt":"a cat", "count":1, "size":"1024x1024"}),
            Self::ImageEdit => json!({
                "prompt":"a cat with a blue background", "count":1,
                "inputs":[{"asset_id":reference,"role":"reference"}]
            }),
            Self::Video => json!({"prompt":"ocean waves", "seconds":4, "count":1}),
            Self::Audio => json!({"text":"你好世界", "voice":"alloy", "format":"mp3"}),
            Self::Music => json!({"prompt":"calm acoustic guitar", "instrumental":true}),
            Self::Text => json!({"prompt":"MEDIA_TEXT_ARTIFACT", "max_tokens":1024}),
        }
    }

    fn capability(self) -> &'static str {
        match self {
            Self::Image => "t2i",
            Self::ImageEdit => "i2i",
            Self::Video => "t2v",
            Self::Audio => "tts",
            Self::Music => "music",
            Self::Text => "text",
        }
    }

    fn product(self) -> Vec<u8> {
        match self {
            Self::Image | Self::ImageEdit => png(),
            Self::Video => mp4(),
            Self::Audio | Self::Music => mp3(),
            Self::Text => TEXT_PRODUCT.as_bytes().to_vec(),
        }
    }
}

async fn dispatch(router: &axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .header("idempotency-key", uuid::Uuid::now_v7().to_string())
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn call(router: &axum::Router, method: &str, path: &str, body: Value) -> Value {
    let (status, envelope) = dispatch(router, method, path, body).await;
    assert!(status.is_success(), "{path}: HTTP {status}: {envelope}");
    assert_eq!(envelope["success"], true, "{path}: {envelope}");
    envelope["data"].clone()
}

fn stream(tool: Option<(&str, &str, Value)>, text: &str) -> ResponseTemplate {
    let (delta, reason) = match tool {
        Some((id, name, arguments)) => (
            json!({"tool_calls":[{"index":0,"id":id,"type":"function",
                "function":{"name":name,"arguments":arguments.to_string()}}]}),
            "tool_calls",
        ),
        None => (json!({"content":text}), "stop"),
    };
    let data = json!({"id":"media-regression","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let done = json!({"id":"media-regression","choices":[{"index":0,"delta":{},"finish_reason":reason}]});
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {data}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
}

struct Fixture {
    _root: tempfile::TempDir,
    services: AppServices,
    router: axum::Router,
    upstream: MockServer,
    chat: Value,
    routes: BTreeMap<String, Value>,
    tool_names: Arc<Mutex<BTreeMap<String, String>>>,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(TRUST.into()),
            ..Default::default()
        };
        std::fs::create_dir_all(&config.data_dir).unwrap();
        std::fs::create_dir_all(&config.work_dir).unwrap();
        let database = nomifun_db::init_database_memory().await.unwrap();
        #[allow(unused_mut)]
        let mut services = AppServices::from_config(database, &config).await.unwrap();
        #[cfg(all(feature = "browser-use", feature = "computer-use"))]
        {
            // The General preset needs an attachable browser resource, not a
            // browser process or network access to a real page.
            services.attached_chrome = Some(nomifun_app::AttachedChromeProviderService::new());
        }
        let router = create_router(&services).await;
        let upstream = MockServer::start().await;
        let mut fixture = Self {
            _root: root,
            services,
            router,
            upstream,
            chat: Value::Null,
            routes: BTreeMap::new(),
            tool_names: Arc::new(Mutex::new(BTreeMap::new())),
        };
        fixture.chat = fixture
            .provider("stepfun-plan", CHAT_MODEL, "chat", "openai.chat_text", "/step_plan/v1")
            .await;
        fixture.routes.insert("text".into(), fixture.chat.clone());
        for (case, platform, model, task, protocol, prefix) in [
            ("image", "stepfun-plan", "step-image-edit-2", "image_generation", "stepfun.images", "/step_plan/v1"),
            ("image_edit", "agnes", "agnes-image-2.1-flash", "image_edit", "agnes.images", "/agnes/v1"),
            ("video", "openai", "sora-2", "video_generation", "openai.videos", "/video/v1"),
            ("audio", "openai", "tts-1", "speech_synthesis", "openai.audio_speech", "/speech/v1"),
            ("music", "minimax", "music-3.0", "music_generation", "minimax.music", "/music/v1"),
        ] {
            let route = fixture.provider(platform, model, task, protocol, prefix).await;
            fixture.routes.insert(case.into(), route);
        }
        let preferences = SqliteClientPreferenceRepository::new(fixture.services.database.pool().clone());
        for (case, key) in [
            ("text", "nomi.defaultModel"),
            ("image", "models.default.imageGeneration"),
            ("image_edit", "models.default.imageEdit"),
            ("video", "models.default.videoGeneration"),
            ("audio", "models.default.speechSynthesis"),
            ("music", "models.default.musicGeneration"),
        ] {
            preferences.upsert_batch(&[(key, fixture.routes[case].to_string().as_str())]).await.unwrap();
        }
        fixture.mount_media().await;
        fixture
    }

    async fn provider(&self, platform: &str, model: &str, task: &str, protocol: &str, prefix: &str) -> Value {
        let provider = call(&self.router, "POST", "/api/providers", json!({
            "platform":platform, "name":format!("media fixture {task}"),
            "base_url":format!("{}{prefix}",self.upstream.uri()),
            "auth_scheme":"bearer", "credentials":{"api_keys":["fixture-only"]}, "enabled":true,
            "initial_model":{"model":model,"enabled":true,"capabilities":[{
                "task":task,"traits":[],"protocol":protocol,"connection_role":"default","provider_params":{}
            }]}
        })).await;
        json!({"provider_id":provider["provider_id"],"model":model})
    }

    async fn mount_media(&self) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(png());
        for endpoint in ["/step_plan/v1/images/generations", "/agnes/v1/images/generations"] {
            Mock::given(method("POST")).and(path(endpoint))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data":[{"b64_json":encoded,"finish_reason":"success"}]
                })))
                .mount(&self.upstream).await;
        }
        Mock::given(method("POST")).and(path("/video/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"media-video","status":"queued"})))
            .mount(&self.upstream).await;
        Mock::given(method("GET")).and(path("/video/v1/videos/media-video"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"media-video","status":"completed"})))
            .mount(&self.upstream).await;
        Mock::given(method("GET")).and(path("/video/v1/videos/media-video/content"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type","video/mp4").set_body_bytes(mp4()))
            .mount(&self.upstream).await;
        Mock::given(method("POST")).and(path("/speech/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type","audio/mpeg").set_body_bytes(mp3()))
            .mount(&self.upstream).await;
        let hex = mp3().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        Mock::given(method("POST")).and(path("/music/v1/music_generation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp":{"status_code":0},"data":{"status":2,"audio":hex}
            })))
            .mount(&self.upstream).await;
    }

    async fn mount_chat(&self) {
        let reference = self.services.workshop_service.upload_asset(
            nomifun_workshop::service::NewAssetUpload {
                file_name: "reference.png".into(), content_type: Some("image/png".into()),
                bytes: png(), title: None, collection: None, tags: None, in_library: Some(false),
            }
        ).await.unwrap().asset_id;
        let names = self.tool_names.clone();
        Mock::given(method("POST")).and(path("/step_plan/v1/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(body["model"], CHAT_MODEL, "creation must not replace the Session text model");
                let messages = body["messages"].as_array().unwrap();
                let input = messages.iter().rev().find(|message|message["role"] == "user")
                    .expect("accepted user input")["content"].to_string();
                if input.contains("MEDIA_TEXT_ARTIFACT") {
                    assert!(body["tools"].as_array().is_none_or(Vec::is_empty), "the text artifact executor does not recursively expose tools");
                    return stream(None, TEXT_PRODUCT);
                }
                let case = Media::ALL.into_iter().find(|case|input.contains(&format!("MEDIA_CASE=[{}]", case.label())))
                    .expect("recognized media request");
                let action = case.action();
                let empty_tools = Vec::new();
                let tools = body["tools"].as_array().unwrap_or(&empty_tools);
                let tool = tools.iter().find(|tool|tool["function"]["description"].as_str()
                    .is_some_and(|description|description.contains(&format!("Action: {action}."))));
                if input.contains("MEDIA_DENIED") {
                    assert!(tool.is_none(), "ungranted action must not be model-visible");
                    if messages.iter().any(|message|message["role"] == "tool" && message["tool_call_id"] == "denied-image") {
                        let result = messages.iter().find(|message|message["role"] == "tool" && message["tool_call_id"] == "denied-image").unwrap();
                        assert!(result["content"].to_string().contains("Not executed"), "ungranted call must be refused: {result}");
                        return stream(None, "The selected Agent does not grant image creation.");
                    }
                    let name = names.lock().unwrap().get("image").expect("captured authorized model name").clone();
                    return stream(Some(("denied-image", &name, case.arguments(&reference))), "");
                }
                let tool = tool.unwrap_or_else(||panic!("{action} missing from the model tool surface: {tools:?}"));
                let name = tool["function"]["name"].as_str().unwrap();
                names.lock().unwrap().insert(case.label().into(), name.to_owned());
                let schema = &tool["function"]["parameters"];
                assert!(schema["properties"].get("target").is_none(), "Session/Turn ownership belongs to the host");
                assert!(!schema["required"].as_array().unwrap().iter().any(|key|key == "target"));
                if input.contains("MEDIA_PROBE") {
                    return stream(None, "The authorized image schema is available.");
                }
                if input.contains("MEDIA_GENERAL") {
                    assert!(tools.iter().any(|tool|tool["function"]["name"] == "write_file"), "automatic creation retains other authorized tools");
                    assert!(!tools.iter().any(|tool|tool["function"]["description"].as_str()
                        .is_some_and(|description|description.contains("Action: creation.media/image_edit."))),
                        "the General catalog must remain deferred to exercise presentation changes");
                }
                let call_id = format!("create-{}", case.label());
                if messages.iter().any(|message|message["role"] == "tool" && message["tool_call_id"] == call_id) {
                    let result = messages.iter().find(|message|message["role"] == "tool" && message["tool_call_id"] == call_id).unwrap();
                    assert!(result["content"].to_string().contains("creation_task_id"), "Creation effect was refused before the host: {result}");
                    stream(None, "The generation task was accepted.")
                } else {
                    stream(Some((&call_id, name, case.arguments(&reference))), "")
                }
            }).mount(&self.upstream).await;
    }

    async fn session(&self, general: bool, actions: &[String]) -> (String, String) {
        let template = if general { "assistant.general" } else { "chat.minimal" };
        let preset = call(&self.router, "POST", &format!("/api/agent-presets/from-template/{template}"), json!({
            "reuse_existing":false,"display_name":"media regression","model":self.chat
        })).await;
        let preset_id = preset["preset"]["preset_id"].as_str().unwrap().to_owned();
        if !general {
            let mut draft = preset["draft"].clone();
            draft["document"]["enabled_capabilities"] = if actions.is_empty() { json!([]) } else {
                json!([{"capability":{"id":"creation.media"},"action_allowlist":actions}])
            };
            call(&self.router, "POST", &format!("/api/agent-presets/{preset_id}/revisions"), json!({
                "expected_current_revision":draft["current_revision"],"draft":draft,
                "reason":"exercise only the explicitly granted media actions"
            })).await;
        }
        let resources = if general {
            json!([
                {"resource_kind":"computer","resource_id":"local-desktop"},
                {"resource_kind":"process_session","resource_id":"managed-process-session"},
                {"resource_kind":"project_memory","resource_id":"default-project-memory"},
                {"resource_kind":"scheduler","resource_id":"installation-scheduler"},
                {"resource_kind":"workspace","resource_id":"default-workspace"}
            ])
        } else { json!([]) };
        let session = call(&self.router, "POST", "/api/agent-sessions", json!({
            "preset_id":preset_id,"model":self.chat,"resource_selections":resources
        })).await;
        assert_eq!(session["state"], "ready");
        let session_id = session["agent_session_id"].as_str().unwrap().to_owned();
        let projection = call(&self.router, "GET", &format!("/api/agent-sessions/{session_id}/projection"), Value::Null).await;
        let snapshot = &projection["agent_snapshot"];
        assert!(!snapshot["enabled_capabilities"].as_array().unwrap().iter().any(|capability|capability == "creative.workshop"),
            "media creation needs no creative.workshop grant");
        if !general {
            let capabilities = snapshot["enabled_capabilities"].as_array().unwrap();
            assert_eq!(capabilities.len(), usize::from(!actions.is_empty()));
            if !actions.is_empty() {
                assert_eq!(capabilities[0], "creation.media");
                assert_eq!(snapshot["enabled_capability_actions"]["creation.media"], json!(actions));
            }
        }
        (session_id, preset_id)
    }

    async fn turn(&self, session: &str, content: String) -> String {
        let turn = call(&self.router, "POST", &format!("/api/agent-sessions/{session}/turns"), json!({
            "idempotency_key":uuid::Uuid::now_v7().to_string(),"input":{"content":content}
        })).await;
        let operation = turn["operation_id"].as_str().unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let receipt: (String, Option<String>, String) = nomifun_db::sqlx::query_as(
                    "SELECT state,error_json,source_message_id FROM agent_turns WHERE session_id=? AND operation_id=?"
                ).bind(session).bind(operation).fetch_one(self.services.database.pool()).await.unwrap();
                match receipt.0.as_str() {
                    "completed" => break receipt.2,
                    "failed" | "canceled" => panic!("media turn failed: {receipt:?}"),
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        }).await.expect("the canonical Turn must settle")
    }

    async fn assert_product(&self, session: &str, message: &str, case: Media) {
        let task = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let page = call(&self.router, "GET", &format!("/api/agent-sessions/{session}/creation-tasks"), Value::Null).await;
                let tasks = page["items"].as_array().unwrap();
                assert_eq!(tasks.len(), 1, "one admitted call must persist one task: {page}");
                let task = &tasks[0];
                match task["status"].as_str() {
                    Some("succeeded") => break task.clone(),
                    Some("failed" | "canceled") => panic!("Creation task failed: {task}"),
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        }).await.expect("the Creation worker must persist a terminal product");
        assert_eq!(task["owner"]["kind"], "conversation_turn");
        assert_eq!(task["owner"]["conversation_id"], session);
        assert_eq!(task["owner"]["message_id"], message, "host must inject the admitted canonical user Turn");
        assert_eq!(task["provider_id"], self.routes[case.label()]["provider_id"]);
        assert_eq!(task["model"], self.routes[case.label()]["model"]);
        assert_eq!(task["capability"], case.capability());
        assert!(task["owner"].get("canvas_id").is_none());
        assert!(task["owner"].get("template_id").is_none());
        let assets = task["result_asset_ids"].as_array().unwrap();
        assert_eq!(assets.len(), 1);
        let asset = assets[0].as_str().unwrap();
        let row: (String, i64, String) = nomifun_db::sqlx::query_as(
            "SELECT kind,in_library,origin FROM workshop_assets WHERE asset_id=? AND deleted_at IS NULL"
        ).bind(asset).fetch_one(self.services.database.pool()).await.unwrap();
        assert_eq!(row.1, 1, "generated products are stored in the asset library without granting the Agent its management tools");
        let origin: Value = serde_json::from_str(&row.2).unwrap();
        assert_eq!(origin["creation_task_id"], task["creation_task_id"]);
        assert_eq!(origin["conversation_id"], session);
        assert_eq!(origin["message_id"], message);
        let response = self.router.clone().oneshot(Request::builder()
            .uri(format!("/api/creative-studio/files/{asset}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
        assert_eq!(bytes.as_ref(), case.product().as_slice(), "the generated artifact must survive the real asset bridge");
        if matches!(case, Media::Image | Media::ImageEdit) {
            let image = image::load_from_memory(&bytes).unwrap();
            assert_eq!((image.width(), image.height()), (1, 1));
        }
        let failures: i64 = nomifun_db::sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind IN ('effect/failed','turn/failed')"
        ).bind(session).fetch_one(self.services.database.pool()).await.unwrap();
        assert_eq!(failures, 0, "schema presentation must not invalidate an authorized effect");
    }

    async fn close(self) {
        self.services.shutdown_browser_platform().await.unwrap();
        self.services.database.close().await;
    }
}

#[cfg(all(feature = "browser-use", feature = "computer-use"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn general_agent_executes_hinted_creation_from_its_deferred_catalog() {
    let fixture = Fixture::new().await;
    fixture.mount_chat().await;
    for case in [Media::Image, Media::Video, Media::Audio, Media::Music] {
        let (session, _) = fixture.session(true, &[]).await;
        let message = fixture.turn(&session, format!("{} MEDIA_GENERAL", case.request())).await;
        fixture.assert_product(&session, &message, case).await;
    }
    let requests = fixture.upstream.received_requests().await.unwrap();
    assert_eq!(requests.iter().filter(|request|request.method == "POST" && request.url.path() == "/step_plan/v1/chat/completions").count(), 8,
        "each hint executes once and settles without a schema rejection/retry loop");
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_media_action_works_with_only_its_own_agent_grant_and_no_canvas_resource() {
    let fixture = Fixture::new().await;
    fixture.mount_chat().await;
    for case in Media::ALL {
        let (session, _) = fixture.session(false, &[case.action()]).await;
        let message = fixture.turn(&session, case.request()).await;
        fixture.assert_product(&session, &message, case).await;
    }
    let requests = fixture.upstream.received_requests().await.unwrap();
    assert_eq!(requests.iter().filter(|request|request.method == "POST" && request.url.path() == "/step_plan/v1/chat/completions").count(), 13,
        "six canonical media calls settle once each, plus the isolated text artifact completion");
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn absent_media_module_and_ungranted_action_reject_text_model_creation_calls() {
    let fixture = Fixture::new().await;
    fixture.mount_chat().await;
    let (probe, _) = fixture.session(false, &[Media::Image.action()]).await;
    fixture.turn(&probe, format!("{} MEDIA_PROBE", Media::Image.request())).await;
    assert!(fixture.tool_names.lock().unwrap().contains_key("image"));
    for actions in [vec![], vec![Media::Audio.action()]] {
        let (session, preset_id) = fixture.session(false, &actions).await;
        fixture.turn(&session, format!("{} MEDIA_DENIED", Media::Image.request())).await;
        let route = &fixture.routes["image"];
        let (status, refusal) = dispatch(&fixture.router, "POST", &format!("/api/agent-sessions/{session}/creation-tasks"), json!({
            "preset_id":preset_id,"provider_id":route["provider_id"],"model":route["model"],
            "capability":"t2i","params":{"prompt":"a cat","count":1}
        })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refusal}");
        assert_eq!(refusal["code"], "CAPABILITY_ACTION_NOT_GRANTED");
    }
    let count: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks")
        .fetch_one(fixture.services.database.pool()).await.unwrap();
    assert_eq!(count, 0, "neither a malicious model call nor direct submission may bypass the exact Action grant");
    assert!(fixture.upstream.received_requests().await.unwrap().iter().all(|request|
        request.url.path() == "/step_plan/v1/chat/completions"),
        "no ungranted effect reaches a generation provider");
    fixture.close().await;
}

fn png() -> Vec<u8> {
    let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([4, 5, 6, 255])));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn mp3() -> Vec<u8> {
    // Two complete MPEG1 Layer3 frames, accepted by the real artifact validator.
    let mut frame = vec![0; 417];
    frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0]);
    frame[10] = 1;
    [frame.clone(), frame].concat()
}

fn mp4() -> Vec<u8> {
    // Minimal complete sample tables and one video sample. This deliberately
    // exercises the product validator rather than persisting a magic header.
    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        [u32::try_from(payload.len() + 8).unwrap().to_be_bytes().as_slice(), kind, payload].concat()
    }
    let sample = boxed(b"avc1", &[1; 8]);
    let stsd = [[0; 4].as_slice(), &1_u32.to_be_bytes(), &sample].concat();
    let stts = [[0; 4].as_slice(), &1_u32.to_be_bytes(), &1_u32.to_be_bytes(), &1_u32.to_be_bytes()].concat();
    let stsc = [[0; 4].as_slice(), &1_u32.to_be_bytes(), &1_u32.to_be_bytes(), &1_u32.to_be_bytes(), &1_u32.to_be_bytes()].concat();
    let stsz = [[0; 4].as_slice(), &4_u32.to_be_bytes(), &1_u32.to_be_bytes()].concat();
    let stco = [[0; 4].as_slice(), &1_u32.to_be_bytes(), &1_u32.to_be_bytes()].concat();
    let stbl = [boxed(b"stsd", &stsd), boxed(b"stts", &stts), boxed(b"stsc", &stsc), boxed(b"stsz", &stsz), boxed(b"stco", &stco)].concat();
    let mut hdlr = vec![0; 12];
    hdlr[8..12].copy_from_slice(b"vide");
    let mdia = [boxed(b"mdhd", &[1; 8]), boxed(b"hdlr", &hdlr), boxed(b"minf", &boxed(b"stbl", &stbl))].concat();
    let trak = [boxed(b"tkhd", &[1; 8]), boxed(b"mdia", &mdia)].concat();
    let moov = [boxed(b"mvhd", &[1; 8]), boxed(b"trak", &trak)].concat();
    [boxed(b"ftyp", b"isom\0\0\0\0"), boxed(b"moov", &moov), boxed(b"mdat", &[1, 2, 3, 4])].concat()
}

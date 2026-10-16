//! Real catalog resolution and adapters against local HTTP/WebSocket servers.
//! No cloud credentials, downloads, job polling or generated conversations.

use std::{io::Cursor, sync::{Arc, Mutex}, time::Duration};

use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use nomifun_api_types::{HealthStatus, ModelTask};
use nomifun_common::encrypt_string;
use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository, SqliteProviderModelRepository,
    SqliteProviderRepository, init_database_memory};
use nomifun_model_invoke::{AdapterRegistry, ModelInvokeService, ModelRef, default_adapters, protocol_task_descriptor};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_hdr_async, tungstenite::{Message, handshake::server::{Request, Response}}};
use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: [u8; 32] = [0x37; 32];

async fn fixture(base_url: &str, platform: &str, model: &str, task: ModelTask, protocol: &str, params: Value) -> (ModelInvokeService, ModelRef) {
    let database = init_database_memory().await.unwrap();
    let pool = database.pool().clone();
    let descriptor = protocol_task_descriptor(protocol, task).expect("implemented fixture protocol");
    let endpoint = |field: &str| descriptor.endpoints.iter().find(|endpoint| endpoint.field == field).map(|endpoint| endpoint.default_value.as_str());
    let task_name = serde_json::to_value(task).unwrap();
    let params = params.to_string();
    let capabilities = [NewProviderModelCapability {
        task: task_name.as_str().unwrap(), traits: "[]", protocol, connection_role: "default",
        endpoint: endpoint("endpoint"), poll_endpoint: endpoint("poll_endpoint"),
        content_endpoint: endpoint("content_endpoint"), realtime_endpoint: endpoint("realtime_endpoint"),
        provider_params: &params, ..Default::default()
    }];
    let encrypted = encrypt_string(r#"{"api_keys":["probe-test-key"]}"#, &KEY).unwrap();
    let (provider, _) = SqliteProviderRepository::new(pool.clone()).create(CreateProviderParams {
        provider_id: None, platform, name: "Local protocol fixture", base_url, auth_scheme: "bearer",
        credentials_encrypted: &encrypted, enabled: true, bedrock_config: None, sort_order: None,
    }, &NewProviderModel { model, enabled: true, capabilities: &capabilities, ..Default::default() }, &[]).await.unwrap();
    let service = ModelInvokeService::new(Arc::new(SqliteProviderRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelRepository::new(pool.clone())), Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool)), KEY,
        reqwest::Client::builder().no_proxy().build().unwrap(), AdapterRegistry::new(default_adapters()));
    (service, ModelRef { provider_id: provider.provider_id, model: model.to_owned() })
}

fn png() -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([15, 40, 90, 255])))
        .write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn wav() -> Vec<u8> {
    let samples = [1_u8, 0, 255, 255, 2, 0, 254, 255];
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF"); bytes.extend_from_slice(&(36_u32 + samples.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt "); bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24000_u32.to_le_bytes()); bytes.extend_from_slice(&48000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes()); bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data"); bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&samples); bytes
}

#[tokio::test]
async fn image_probe_requires_usable_artifacts_and_never_downloads_a_returned_url() {
    let server = MockServer::start().await;
    let (service, model) = fixture(&format!("{}/v1",server.uri()), "openai", "fixture-image", ModelTask::ImageGeneration, "openai.images", json!({})).await;
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    for (name, reply, expected) in [
        ("empty URL", json!({"data":[{"url":""}]}), HealthStatus::Unhealthy),
        ("bad base64", json!({"data":[{"b64_json":"not base64!"}]}), HealthStatus::Unhealthy),
        ("JSON disguised as image", json!({"data":[{"b64_json":encode(br#"{"error":"not an image"}"#)}]}), HealthStatus::Unhealthy),
        ("decoded PNG", json!({"data":[{"b64_json":encode(&png())}]}), HealthStatus::Healthy),
        ("unverified URL", json!({"data":[{"url":format!("{}/output.png?signature=fixture",server.uri())}]}), HealthStatus::Unknown),
    ] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/images/generations")).and(header("authorization","Bearer probe-test-key"))
            .and(body_partial_json(json!({"model":"fixture-image","prompt":"health check","n":1})))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply)).expect(1).mount(&server).await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).expect(0).mount(&server).await;
        let report = service.probe(&model, ModelTask::ImageGeneration).await.unwrap();
        assert_eq!(report.status, expected, "{name}: {:?}",report.message);
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "{name}: no artifact download");
        server.verify().await;
    }
}

#[tokio::test]
async fn video_probe_rejects_failed_submission_and_keeps_queued_jobs_unknown_without_polling() {
    let server = MockServer::start().await;
    let (service, model) = fixture(&format!("{}/v1",server.uri()), "openai", "fixture-video", ModelTask::VideoGeneration, "openai.videos", json!({})).await;
    for (status, expected) in [("failed",HealthStatus::Unhealthy),("queued",HealthStatus::Unknown)] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/videos")).and(header("authorization","Bearer probe-test-key"))
            .and(body_string_contains("name=\"model\"\r\n\r\nfixture-video"))
            .and(body_string_contains("name=\"prompt\"\r\n\r\nhealth check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"local-video-job","status":status})))
            .expect(1).mount(&server).await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(500)).expect(0).mount(&server).await;
        let report = service.probe(&model, ModelTask::VideoGeneration).await.unwrap();
        assert_eq!(report.status, expected, "{status}: {:?}",report.message);
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "probe must not poll or download the job");
        server.verify().await;
    }
}

#[tokio::test]
async fn speech_probe_rejects_empty_and_json_audio_but_accepts_a_real_wav() {
    let server = MockServer::start().await;
    let (service, model) = fixture(&format!("{}/v1",server.uri()), "openai", "fixture-tts", ModelTask::SpeechSynthesis, "openai.audio_speech",
        json!({"voice":"alloy","response_format":"wav"})).await;
    for (name, bytes, mime, expected) in [
        ("empty audio",vec![],"audio/wav",HealthStatus::Unhealthy),
        ("JSON with audio MIME",br#"{"error":"not audio"}"#.to_vec(),"audio/wav",HealthStatus::Unhealthy),
        ("JSON MIME",br#"{"error":"not audio"}"#.to_vec(),"application/json",HealthStatus::Unhealthy),
        ("complete WAV",wav(),"audio/wav",HealthStatus::Healthy),
    ] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/audio/speech")).and(header("authorization","Bearer probe-test-key"))
            .and(body_partial_json(json!({"model":"fixture-tts","input":"hi","voice":"alloy","response_format":"wav"})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(bytes,mime)).expect(1).mount(&server).await;
        let report = service.probe(&model, ModelTask::SpeechSynthesis).await.unwrap();
        assert_eq!(report.status, expected, "{name}: {:?}",report.message);
        server.verify().await;
    }
}

#[tokio::test]
async fn transcription_probe_distinguishes_missing_text_from_explicit_silence() {
    let server = MockServer::start().await;
    let (service, model) = fixture(&format!("{}/v1",server.uri()), "openai", "fixture-asr", ModelTask::SpeechRecognition, "openai.audio_transcriptions", json!({})).await;
    for (reply, expected) in [(json!({}),HealthStatus::Unhealthy),(json!({"text":""}),HealthStatus::Healthy)] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/audio/transcriptions")).and(header("authorization","Bearer probe-test-key"))
            .and(|request: &wiremock::Request| {
                // The WAV contains arbitrary PCM bytes, so a matcher that
                // requires the entire multipart body to be UTF-8 is invalid.
                let body = String::from_utf8_lossy(&request.body);
                body.contains("name=\"model\"\r\n\r\nfixture-asr")
                    && body.contains("name=\"file\"; filename=\"audio.wav\"")
                    && request.body.windows(4).any(|bytes| bytes == b"RIFF")
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(reply)).expect(1).mount(&server).await;
        let report = service.probe(&model, ModelTask::SpeechRecognition).await.unwrap();
        assert_eq!(report.status, expected, "{:?}",report.message);
        assert_eq!(server.received_requests().await.unwrap().len(),1);
        server.verify().await;
    }
}

#[tokio::test]
async fn embedding_probe_rejects_empty_or_overflowing_vectors_and_accepts_finite_results() {
    let server = MockServer::start().await;
    let (service, model) = fixture(&format!("{}/v1",server.uri()), "openai", "fixture-embedding", ModelTask::Embedding, "openai.embeddings", json!({})).await;
    for (vector, expected) in [(json!([]),HealthStatus::Unhealthy),(json!([1e100]),HealthStatus::Unhealthy),(json!([0,-0.5,1]),HealthStatus::Healthy)] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/embeddings")).and(header("authorization","Bearer probe-test-key"))
            .and(body_partial_json(json!({"model":"fixture-embedding","input":["health check"]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"index":0,"embedding":vector}]})))
            .expect(1).mount(&server).await;
        let report = service.probe(&model, ModelTask::Embedding).await.unwrap();
        assert_eq!(report.status, expected, "{:?}",report.message);
        server.verify().await;
    }
}

#[tokio::test]
async fn rerank_probe_rejects_duplicate_or_foreign_indices_but_allows_negative_scores() {
    let server = MockServer::start().await;
    let (service, model) = fixture(&format!("{}/v1",server.uri()), "custom", "fixture-reranker", ModelTask::Rerank, "generic.rerank", json!({})).await;
    for (results, expected) in [
        (json!([{"index":0,"score":0.9},{"index":0,"score":0.8}]),HealthStatus::Unhealthy),
        (json!([{"index":1,"score":0.9}]),HealthStatus::Unhealthy),
        (json!([{"index":0,"score":-2.5}]),HealthStatus::Healthy),
    ] {
        server.reset().await;
        Mock::given(method("POST")).and(path("/v1/rerank")).and(header("authorization","Bearer probe-test-key"))
            .and(body_partial_json(json!({"model":"fixture-reranker","query":"health check","documents":["health check"],"top_n":1})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":results}))).expect(1).mount(&server).await;
        let report = service.probe(&model, ModelTask::Rerank).await.unwrap();
        assert_eq!(report.status, expected, "{:?}",report.message);
        server.verify().await;
    }
}

#[derive(Clone, Copy, Debug)]
enum RealtimeCase { Updated, MissingId, ProviderError, ConfigurationError, ClosedBeforeUpdate }

async fn realtime_probe(case: RealtimeCase) -> (HealthStatus, Vec<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handshake = Arc::new(Mutex::new(None));
    let captured = Arc::clone(&handshake);
    let worker = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = accept_hdr_async(tcp, move |request: &Request, response: Response| {
            *captured.lock().unwrap() = Some((request.uri().to_string(),request.headers().get("authorization").unwrap().to_str().unwrap().to_owned()));
            Ok(response)
        }).await.unwrap();
        let created = match case {
            RealtimeCase::MissingId => json!({"type":"session.created","session":{"model":"stepaudio-3-realtime-preview"}}),
            RealtimeCase::ProviderError => json!({"type":"error","error":{"type":"invalid_request_error","message":"fixture rejected"}}),
            _ => json!({"type":"session.created","session":{"id":"local-session","model":"stepaudio-3-realtime-preview"}}),
        };
        socket.send(Message::Text(created.to_string().into())).await.unwrap();
        let mut controls = Vec::new();
        while let Ok(Some(Ok(frame))) = tokio::time::timeout(Duration::from_secs(2),socket.next()).await {
            match frame {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(text.as_ref()).unwrap();
                    controls.push(event.clone());
                    assert_eq!(event["type"],"session.update","probe must not create a response or send audio");
                    assert_eq!(event["session"]["modalities"],json!(["text","audio"]));
                    match case {
                        RealtimeCase::Updated => socket.send(Message::Text(json!({"type":"session.updated","session":{"id":"local-session"}}).to_string().into())).await.unwrap(),
                        RealtimeCase::ConfigurationError => {
                            let _ = socket.send(Message::Text(json!({"type":"error","error":{"type":"invalid_request_error","message":"fixture session configuration failed"}}).to_string().into())).await;
                            let _ = socket.close(None).await;
                            break;
                        }
                        _ => { let _ = socket.close(None).await; break; }
                    }
                }
                Message::Close(_) => break,
                Message::Ping(bytes) => { let _ = socket.send(Message::Pong(bytes)).await; }
                _ => panic!("probe sent unexpected binary audio"),
            }
        }
        controls
    });
    let (service, model) = fixture(&format!("http://{address}/v1"), "stepfun", "stepaudio-3-realtime-preview",
        ModelTask::RealtimeConversation, "stepfun.realtime_s2s", json!({})).await;
    let report = tokio::time::timeout(Duration::from_secs(4),service.probe_realtime(&model)).await
        .expect("explicit local failure must not wait for the global probe timeout").unwrap();
    let controls = tokio::time::timeout(Duration::from_secs(3),worker).await.expect("bounded local socket worker").unwrap();
    let (uri, auth) = handshake.lock().unwrap().clone().unwrap();
    assert_eq!(auth,"Bearer probe-test-key");
    assert_eq!(uri,"/v1/realtime?model=stepaudio-3-realtime-preview");
    assert!(controls.len() <= 1,"no repeated configuration or generation requests");
    (report.status,controls)
}

#[tokio::test]
async fn realtime_created_and_configured_session_is_unknown_without_generating_audio() {
    let (status,controls) = realtime_probe(RealtimeCase::Updated).await;
    assert_eq!(status,HealthStatus::Unknown);
    assert_eq!(controls.len(),1);
}

#[tokio::test]
async fn realtime_missing_session_identity_or_provider_error_is_unhealthy() {
    for case in [RealtimeCase::MissingId,RealtimeCase::ProviderError] {
        assert_eq!(realtime_probe(case).await.0,HealthStatus::Unhealthy,"{case:?}");
    }
}

#[tokio::test]
async fn realtime_creation_without_successful_configuration_is_unhealthy() {
    for case in [RealtimeCase::ConfigurationError,RealtimeCase::ClosedBeforeUpdate] {
        assert_eq!(realtime_probe(case).await.0,HealthStatus::Unhealthy,"{case:?}");
    }
}

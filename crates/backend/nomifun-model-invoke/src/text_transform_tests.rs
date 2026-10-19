use super::*;
use crate::{AuthScheme, ResolvedConnection, ResolvedTaskTransport, SingleAttemptFrame};
use futures_util::stream;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn config(protocol: &str, url: &str) -> ResolvedTaskConfig {
    ResolvedTaskConfig {
        provider_id: "fixture".into(), config_revision: 1, platform: "stepfun".into(),
        model: "step-5-preview".into(), task: ModelTask::Chat, traits: vec![],
        protocol: protocol.into(), connection: ResolvedConnection {
            role: "default".into(), base_url: url.into(), auth: AuthMaterial {
                scheme: AuthScheme::Bearer, credentials: json!({"api_keys":["fixture-key","unused-key"]}),
            }, extra: json!({}),
        }, transport: ResolvedTaskTransport { endpoint: Some("/convert".into()), ..Default::default() },
        provider_params: json!({}), context_limit: None, output_limit: Some(4096), bedrock_config: None,
    }
}

fn request() -> TextTransformRequest<'static> {
    TextTransformRequest { system: "只整理原文，不增加事实", body: "明天下午四点开会", max_output_tokens: 2048,
        max_output_utf8_bytes: 4096, timeout: Duration::from_secs(2) }
}

async fn run(
    config: &ResolvedTaskConfig, request: &TextTransformRequest<'_>, cancel: &CancellationToken,
    on_delta: impl FnMut(&str) + Send,
) -> Result<TextTransformResult, InvokeError> {
    validate_request(request)?;
    let started = Instant::now();
    let deadline = started + request.timeout;
    let mut metrics = TextTransformMetrics::default();
    let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let text = within_deadline(cancel, deadline,
        execute(&client, config, request, started, deadline, &mut metrics, on_delta)).await?;
    metrics.total_ms = elapsed_ms(started);
    Ok(TextTransformResult { text, metrics })
}

fn frame(event: &str, data: Value) -> SingleAttemptFrame {
    SingleAttemptFrame { event: event.into(), data, diagnostic: None }
}

fn openai_frames(reason: &str) -> Vec<SingleAttemptFrame> {
    vec![
        frame("message", json!({"id":"r1","choices":[{"index":0,"delta":{"role":"assistant","content":"整理后"},"finish_reason":null}]})),
        frame("message", json!({"id":"r1","choices":[{"index":0,"delta":{},"finish_reason":reason}],
            "usage":{"prompt_tokens":20,"completion_tokens":8,"completion_tokens_details":{"reasoning_tokens":3}}})),
        frame("done", json!({})),
    ]
}

fn anthropic_frames() -> Vec<SingleAttemptFrame> {
    [
        json!({"type":"message_start","message":{"container":null,"content":[],"id":"m1","model":"fixture",
            "role":"assistant","stop_details":null,"stop_reason":null,"stop_sequence":null,"type":"message", "usage":{"input_tokens":20,"output_tokens":1}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"整理后"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":8}}),
        json!({"type":"message_stop"}),
    ].into_iter().map(|mut data| {
        let object = data.as_object_mut().unwrap();
        for key in ["content_block","delta","error","message"] { object.entry(key).or_insert(Value::Null); }
        object.entry("index").or_insert(json!(0));
        frame(data["type"].as_str().unwrap(), data.clone())
    }).collect()
}

fn responses_frames() -> Vec<SingleAttemptFrame> {
    [
        json!({"type":"response.created","response":{"id":"r1","status":"in_progress"}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"m1","type":"message","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.content_part.added","output_index":0,"item_id":"m1","content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"m1","content_index":0,"delta":"整理后"}),
        json!({"type":"response.output_text.done","output_index":0,"item_id":"m1","content_index":0,"text":"整理后"}),
        json!({"type":"response.content_part.done","output_index":0,"item_id":"m1","content_index":0,"part":{"type":"output_text","text":"整理后","annotations":[]}}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"id":"m1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"整理后","annotations":[]}]}}),
        json!({"type":"response.completed","response":{"id":"r1","status":"completed","output":[{"id":"m1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"整理后","annotations":[]}]}],
            "usage":{"input_tokens":20,"output_tokens":8,"output_tokens_details":{"reasoning_tokens":3}}}}),
    ].into_iter().map(|data| frame(data["type"].as_str().unwrap(), data.clone())).collect()
}

fn sse(frames: &[SingleAttemptFrame]) -> String {
    frames.iter().map(|frame| if frame.event == "done" { "data: [DONE]\n\n".into() }
        else { format!("event: {}\ndata: {}\n\n",frame.event,frame.data) }).collect()
}

#[tokio::test]
async fn one_request_contains_only_current_text_and_instructions_without_tools_or_history() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .set_body_raw(sse(&openai_frames("stop")), "text/event-stream"))
        .expect(1).mount(&server).await;
    let result = run(&config("openai.chat_text", &server.uri()), &request(), &CancellationToken::new(), |_| {}).await.unwrap();
    assert_eq!(result.text,"整理后");
    assert_eq!(result.metrics.input_tokens, Some(20));
    assert_eq!(result.metrics.output_tokens, Some(8));
    assert_eq!(result.metrics.reasoning_tokens, Some(3));
    assert!(result.metrics.first_text_ms.is_some());
    assert!(result.metrics.first_text_ms.unwrap() <= result.metrics.total_ms);
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(),1);
    assert_eq!(received[0].headers["authorization"], "Bearer fixture-key");
    let body: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(),2);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["reasoning_effort"],"low");
    assert_eq!(body["max_tokens"],2048);
    for key in ["tools","tool_choice","parallel_tool_calls","previous_response_id","conversation"] { assert!(body.get(key).is_none(),"{key}"); }
    assert_eq!(body["store"],false);
}

#[tokio::test]
async fn gemini_conversion_cannot_attach_a_configured_cached_conversation() {
    let server=MockServer::start().await;
    let response=frame("message",json!({"responseId":"r1",
        "candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"整理后"}]},"finishReason":"STOP"}],
        "usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":8}}));
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .set_body_raw(sse(&[response]),"text/event-stream")).expect(1).mount(&server).await;
    let mut configured=config("gemini.generate_text",&server.uri());
    configured.provider_params=json!({"cachedContent":"cachedContents/old-session","cached_content":"cachedContents/old-alias"});
    crate::manifest::validate_provider_params_for_protocol(&configured.protocol,ModelTask::Chat,&configured.provider_params).unwrap();
    let result=run(&configured,&request(),&CancellationToken::new(),|_|{}).await.unwrap();
    assert_eq!(result.text,"整理后");
    let received=server.received_requests().await.unwrap();
    let body:Value=serde_json::from_slice(&received[0].body).unwrap();
    assert!(body.get("cachedContent").is_none() && body.get("cached_content").is_none());
    assert_eq!(body["contents"].as_array().unwrap().len(),1);
    assert_eq!(body["contents"][0]["parts"][0]["text"],request().body);
    assert!(configured.provider_params.get("cachedContent").is_some(),"persisted settings must remain untouched");
}

#[tokio::test]
async fn failed_http_is_sent_once_without_retry_key_rotation_or_fallback() {
    for status in [302, 401, 429, 503] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(status)
            .insert_header("Location", format!("{}/must-not-follow",server.uri()))
            .set_body_json(json!({"error":{"message":"fixture"}}))).expect(1).mount(&server).await;
        let error = run(&config("openai.chat_text", &server.uri()), &request(), &CancellationToken::new(), |_| {}).await.err().unwrap();
        assert_eq!(error.http_status,Some(status));
        assert_eq!(server.received_requests().await.unwrap().len(),1);
    }
}

#[test]
fn reasoning_effort_requires_exact_documented_model_and_protocol_and_respects_ceiling() {
    let mut configured = config("openai.chat_text","https://fixture.invalid");
    configured.provider_params = json!({"reasoning_effort":"high"});
    let encode = |configured: &ResolvedTaskConfig| encode_standalone_chat_input(
        chat_protocol_for_id(&configured.protocol).unwrap(), &configured.model,
        &text_input(configured,&request()), &configured.provider_params, Some(1024)).unwrap();
    assert_eq!(encode(&configured)["reasoning_effort"],"low");
    assert_eq!(encode(&configured)["max_tokens"],1024);
    configured.platform = "custom".into();
    assert_eq!(encode(&configured)["reasoning_effort"],"high");
    configured.provider_params = json!({});
    assert!(encode(&configured).get("reasoning_effort").is_none());
    configured.platform = "stepfun".into();
    configured.model = "step-unknown".into();
    assert!(encode(&configured).get("reasoning_effort").is_none());
    configured.platform="openai".into();configured.model="gpt-6.1-sol".into();configured.protocol="openai.responses".into();
    configured.provider_params=json!({"reasoning_effort":"high","reasoning":{"effort":"high","summary":"detailed"}});
    assert_eq!(encode(&configured)["reasoning"]["effort"],"low");
    configured.platform="gemini".into();configured.model="gemini-3-flash-preview".into();configured.protocol="gemini.generate_text".into();
    configured.provider_params=json!({"reasoning_effort":"high","generationConfig":{"thinkingConfig":{"thinkingLevel":"high","thinkingBudget":8192}}});
    let body=encode(&configured);
    assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingLevel"],"minimal");
    assert!(body["generationConfig"]["thinkingConfig"].get("thinkingBudget").is_none());
}

#[tokio::test]
async fn all_six_protocols_encode_tool_free_and_complete_with_shared_standalone_decoders() {
    for (id, protocol, frames) in [
        ("openai.chat_text", ChatProtocol::OpenaiChat, openai_frames("stop")),
        ("openai.responses", ChatProtocol::OpenaiResponses, responses_frames()),
        ("anthropic.messages", ChatProtocol::Anthropic, anthropic_frames()),
        ("bedrock.anthropic_messages", ChatProtocol::Bedrock, anthropic_frames()),
        ("vertex.anthropic_messages", ChatProtocol::Vertex, anthropic_frames()),
        ("gemini.generate_text", ChatProtocol::Gemini, vec![frame("message",json!({"responseId":"r1",
            "candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"整理后"}]},"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":8}}))]),
    ] {
        let configured = config(id,"https://fixture.invalid");
        let body = encode_standalone_chat_input(protocol, &configured.model, &text_input(&configured,&request()),
            &configured.provider_params, configured.output_limit).unwrap();
        assert!(body.get("tools").is_none(),"{id}");
        assert!(body.get("previous_response_id").is_none(),"{id}");
        let mut metrics = TextTransformMetrics::default();
        let mut candidate = String::new();
        let result = drain(Box::pin(stream::iter(frames.into_iter().map(Ok))), protocol,4096,Instant::now(),
            &mut metrics,&SingleAttemptResponseBudget::new(8192).unwrap(), &configured.connection.auth.secret_redactor(),
            |delta|candidate.push_str(delta)).await.unwrap_or_else(|error|panic!("{id}: {error}"));
        assert_eq!(result,"整理后","{id}");
        assert_eq!(candidate,result,"{id}");
        assert_eq!(metrics.input_tokens,Some(20),"{id}");
        assert_eq!(metrics.output_tokens,Some(8),"{id}");
    }
}

#[tokio::test]
async fn truncation_missing_terminal_tool_call_and_provider_error_never_succeed() {
    let tool = vec![frame("message",json!({"id":"r1","choices":[{"index":0,"delta":{"role":"assistant",
        "tool_calls":[{"index":0,"id":"call1","type":"function","function":{"name":"forbidden","arguments":"{}"}}]},"finish_reason":null}]}))];
    for frames in [openai_frames("length"), openai_frames("stop")[..1].to_vec(), tool,
        vec![frame("error",json!({"error":{"message":"fixture failure"}}))]] {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_raw(sse(&frames),"text/event-stream"))
            .expect(1).mount(&server).await;
        assert!(run(&config("openai.chat_text",&server.uri()),&request(),&CancellationToken::new(),|_|{}).await.is_err());
    }
}

#[tokio::test]
async fn utf8_budget_is_checked_before_a_candidate_is_delivered() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .set_body_raw(sse(&openai_frames("stop")),"text/event-stream")).expect(1).mount(&server).await;
    let mut request = request(); request.max_output_utf8_bytes = 8;
    let mut delivered = false;
    assert!(run(&config("openai.chat_text",&server.uri()),&request,&CancellationToken::new(),|_|delivered=true).await.is_err());
    assert!(!delivered);
}

/// Send chunked SSE but deliberately keep the HTTP body open. The test owns
/// the release signal, so completing never depends on a provider EOF/sleep.
async fn held_connection(body: String) -> (String, tokio::sync::oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}",listener.local_addr().unwrap());
    let (release, released) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket,_) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        loop {
            let mut bytes = [0;4096];
            let count = socket.read(&mut bytes).await.unwrap();
            if count == 0 { return; }
            received.extend_from_slice(&bytes[..count]);
            let Some(end) = received.windows(4).position(|bytes|bytes==b"\r\n\r\n") else {continue};
            let headers = String::from_utf8_lossy(&received[..end]);
            let length = headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:")
                .and_then(|value|value.trim().parse::<usize>().ok())).unwrap_or(0);
            if received.len() >= end+4+length {break;}
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        socket.write_all(format!("{:x}\r\n{body}\r\n",body.len()).as_bytes()).await.unwrap();
        let _ = released.await;
    });
    (url, release, task)
}

#[tokio::test]
async fn responses_terminal_returns_without_waiting_for_http_eof() {
    let (url,release,task) = held_connection(sse(&responses_frames())).await;
    let mut request = request(); request.timeout=Duration::from_millis(500);
    let result = run(&config("openai.responses",&url),&request,&CancellationToken::new(),|_|{}).await.unwrap();
    assert_eq!(result.text,"整理后");
    assert!(result.metrics.total_ms<500);
    drop(release); task.await.unwrap();
}

#[tokio::test]
async fn cancellation_drops_pending_stream_and_never_delivers_a_late_candidate() {
    let (url,release,task) = held_connection(sse(&openai_frames("stop")[..1])).await;
    let cancel = CancellationToken::new();
    let mut count = 0;
    let result = run(&config("openai.chat_text",&url),&request(),&cancel,|_| {count+=1;cancel.cancel();}).await;
    assert!(result.is_err()); assert_eq!(count,1);
    drop(release); task.await.unwrap();
    let server = MockServer::start().await;
    assert!(run(&config("openai.chat_text",&server.uri()),&request(),&cancel,|_|panic!("cancelled request emitted text")).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    // Cancellation can win while an entire terminal frame is already ready,
    // without another socket await giving select a chance to poll its branch.
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .set_body_raw(sse(&openai_frames("stop")),"text/event-stream")).expect(1).mount(&server).await;
    let ready_cancel=CancellationToken::new();
    assert!(run(&config("openai.chat_text",&server.uri()),&request(),&ready_cancel,|_|ready_cancel.cancel()).await.is_err());
}

#[tokio::test]
async fn absolute_deadline_bounds_a_stream_without_terminal() {
    let (url,release,task) = held_connection(sse(&openai_frames("stop")[..1])).await;
    let mut request=request();request.timeout=Duration::from_millis(80);
    let error=run(&config("openai.chat_text",&url),&request,&CancellationToken::new(),|_|{}).await.err().unwrap();
    assert_eq!(error.kind,InvokeErrorKind::Timeout);
    drop(release);task.await.unwrap();
}

#[tokio::test]
async fn a_ready_future_cannot_commit_after_crossing_the_deadline_in_one_synchronous_poll() {
    for also_cancel in [false,true] {
        let cancel=CancellationToken::new();
        let deadline=Instant::now()+Duration::from_millis(5);
        let mut polled=false;
        let result=within_deadline(&cancel,deadline,async {
            polled=true;
            std::thread::sleep(Duration::from_millis(10));
            if also_cancel {cancel.cancel();}
            Ok::<_,InvokeError>("ready but too late")
        }).await;
        assert!(polled,"the operation must actually cross the deadline inside its poll");
        let error=result.err().expect("an expired synchronous result must not commit");
        assert_eq!(error.kind,if also_cancel {InvokeErrorKind::InvalidParams} else {InvokeErrorKind::Timeout});
    }
}

#[test]
fn bedrock_region_and_model_form_a_bounded_explicit_target() {
    let mut config=config("bedrock.anthropic_messages","https://fixture.invalid");
    config.model="model/name:1".into();config.bedrock_config=Some(json!({"region":"us-west-2","auth_method":"accessKey"}).to_string());
    let (url,region)=request_target(&config,ChatProtocol::Bedrock).unwrap();
    assert_eq!(region.as_deref(),Some("us-west-2"));
    assert_eq!(url,"https://bedrock-runtime.us-west-2.amazonaws.com/model/model%2Fname%3A1/invoke-with-response-stream");
    config.bedrock_config=Some(json!({"region":"bad.example/","auth_method":"accessKey"}).to_string());
    assert!(request_target(&config,ChatProtocol::Bedrock).is_err());
}

fn bedrock_config(auth_method: BedrockAuthMethod, profile: Option<&str>) -> ResolvedTaskConfig {
    let mut configured=config("bedrock.anthropic_messages", "");
    configured.connection.auth=AuthMaterial {scheme:AuthScheme::Bedrock,credentials:json!({})};
    configured.bedrock_config=Some(serde_json::to_string(&BedrockConfig {
        auth_method,region:"us-west-2".into(),profile:profile.map(str::to_owned),
    }).unwrap());
    configured
}

#[tokio::test]
async fn bedrock_sdk_material_is_resolved_once_only_for_selected_profile_or_chain() {
    for (method, profile) in [(BedrockAuthMethod::Profile,Some("fixture-profile")),(BedrockAuthMethod::DefaultChain,None)] {
        let configured=bedrock_config(method,profile);
        let calls=std::sync::atomic::AtomicUsize::new(0);
        let material=resolve_transform_auth(&configured,|metadata| {let calls=&calls;async move {
            calls.fetch_add(1,Ordering::Relaxed);
            assert_eq!(metadata.auth_method,method);
            assert_eq!(metadata.profile.as_deref(),profile);
            assert_eq!(metadata.region,"us-west-2");
            Ok(aws_credential_types::Credentials::new("fixture-access","fixture-secret",Some("fixture-session".into()),None,"test"))
        }}).await.unwrap();
        assert_eq!(calls.load(Ordering::Relaxed),1);
        assert_eq!(material.scheme,AuthScheme::Bedrock);
        assert_eq!(material.credentials,json!({"access_key_id":"fixture-access","secret_access_key":"fixture-secret","session_token":"fixture-session"}));
        assert_eq!(configured.connection.auth.credentials,json!({}));
    }
    let mut explicit=bedrock_config(BedrockAuthMethod::AccessKey,None);
    explicit.connection.auth.credentials=json!({"access_key_id":"fixture-access","secret_access_key":"fixture-secret"});
    let material=resolve_transform_auth(&explicit,|_|async {panic!("explicit key must not initialize the SDK")}).await.unwrap();
    assert_eq!(material.credentials,explicit.connection.auth.credentials);
    let mut ordinary=config("openai.chat_text","https://fixture.invalid");
    ordinary.bedrock_config=Some("invalid, irrelevant metadata".into());
    let material=resolve_transform_auth(&ordinary,|_|async {panic!("ordinary Chat must not initialize the SDK")}).await.unwrap();
    assert_eq!(material.credentials,ordinary.connection.auth.credentials);
}

#[tokio::test]
async fn bedrock_invalid_auth_metadata_fails_before_sdk_initialization() {
    for metadata in [
        json!({"region":"us-west-2","auth_method":"profile"}),
        json!({"region":"us-west-2","auth_method":"profile","profile":" "}),
        json!({"region":"us-west-2","auth_method":"defaultChain","profile":"forbidden"}),
        json!({"region":"us-west-2","auth_method":"accessKey"}),
        json!({"region":"us-west-2","auth_method":"default_chain"}),
        json!({"region":"invalid/host","auth_method":"defaultChain"}),
    ] {
        let mut configured=bedrock_config(BedrockAuthMethod::DefaultChain,None);
        configured.bedrock_config=Some(metadata.to_string());
        assert!(resolve_transform_auth(&configured,|_|async {panic!("invalid configuration must not initialize the SDK")}).await.is_err());
    }
}

#[tokio::test]
async fn cancellation_and_absolute_deadline_also_drop_an_inflight_aws_credential_provider() {
    struct Dropped<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for Dropped<'_> {fn drop(&mut self){self.0.store(true,Ordering::Relaxed);}}
    for cancel_during_provide in [true,false] {
        let configured=bedrock_config(BedrockAuthMethod::Profile,Some("fixture"));
        let dropped=AtomicBool::new(false);
        let cancel=CancellationToken::new();
        let operation=resolve_transform_auth(&configured,|_|async {
            let _guard=Dropped(&dropped);
            if cancel_during_provide {cancel.cancel();}
            std::future::pending::<Result<aws_credential_types::Credentials,InvokeError>>().await
        });
        let result=within_deadline(&cancel,Instant::now()+Duration::from_millis(20),operation).await;
        assert!(result.is_err());
        assert!(dropped.load(Ordering::Relaxed));
        if !cancel_during_provide {assert_eq!(result.err().unwrap().kind,InvokeErrorKind::Timeout);}
    }
}

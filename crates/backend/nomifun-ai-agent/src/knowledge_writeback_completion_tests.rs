use super::*;
use axum::{Router,extract::State,routing::post,Json};
use nomifun_model_invoke::{AuthScheme,ResolvedConnection,ResolvedTaskTransport,SingleAttemptFrame,SingleAttemptResponseBudget};
use serde_json::json;

fn configuration(base_url:&str)->ResolvedTaskConfig {
    ResolvedTaskConfig {
        provider_id:"0190f5fe-7c00-7a00-8000-000000000001".into(),config_revision:3,platform:"custom".into(),
        model:"arbitrary-chat-model".into(),task:ModelTask::Chat,traits:Vec::new(), input_constraints: None,protocol:"openai.chat_text".into(),
        connection:ResolvedConnection {role:"default".into(),base_url:base_url.into(),
            auth:AuthMaterial {scheme:AuthScheme::Bearer,credentials:json!({"api_keys":["local-test-key"]})},extra:json!({})},
        transport:ResolvedTaskTransport {endpoint:Some("/chat/completions".into()),..Default::default()},
        provider_params:json!({}),context_limit:None,output_limit:None,bedrock_config:None,
    }
}

fn request<'a>(limits:KnowledgeWritebackLimits)->KnowledgeWritebackCompletionRequest<'a> {
    KnowledgeWritebackCompletionRequest {system:"Return valid candidate JSON.",user:"A durable completed answer.",model:None,
        expected_configuration:None,cancellation:None,limits}
}

fn frame(delta:Value,finish:Option<&str>)->SingleAttemptFrame {
    SingleAttemptFrame {event:"message".into(),data:json!({"id":"writeback-response","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}),diagnostic:None}
}

fn finished(reason:&str)->SingleAttemptFrame {
    let mut result=frame(json!({}),Some(reason));
    result.data["usage"]=json!({"completion_tokens":8});result
}

fn frames(script:Vec<(Duration,SingleAttemptFrame)>)->nomifun_model_invoke::SingleAttemptStream {
    Box::pin(futures_util::stream::iter(script).then(|(delay,frame)|async move {
        tokio::time::sleep(delay).await;Ok(frame)
    }))
}

async fn decoded(script:Vec<(Duration,SingleAttemptFrame)>,limits:KnowledgeWritebackLimits,cancellation:CancellationToken)
    ->Result<KnowledgeWritebackCompletion,KnowledgeWritebackCompletionError> {
    let mut observation=Observation::new();observation.diagnostic.stage=Stage::FirstFrame;
    drain(ChatProtocol::OpenaiChat,frames(script),SingleAttemptResponseBudget::new(limits.response_max_bytes).unwrap(),
        limits,limits.output_max_tokens,true,cancellation,observation).await
}

#[tokio::test(start_paused=true)]
async fn healthy_reasoning_and_text_after_45_seconds_are_not_cut_off() {
    let result=decoded(vec![
        (Duration::from_secs(50),frame(json!({"reasoning_content":"Reviewing the grounded facts."}),None)),
        (Duration::from_secs(50),frame(json!({"content":"{\"candidates\":[]}"}),None)),
        (Duration::from_secs(1),finished("stop")),
    ],KnowledgeWritebackLimits::default(),CancellationToken::new()).await.unwrap();
    assert_eq!(result.text,"{\"candidates\":[]}");
    assert_eq!(result.diagnostic.elapsed_ms,Some(101_000));
    assert_eq!(result.diagnostic.first_frame_ms,Some(50_000));
    assert_eq!(result.diagnostic.received_frames,Some(3));
    assert_eq!(result.diagnostic.output_bytes,Some(17));
    assert_eq!(result.diagnostic.output_tokens,Some(8));
    assert_eq!(result.diagnostic.code,None);
    result.diagnostic.validate().unwrap();
}

#[tokio::test(start_paused=true)]
async fn empty_frames_do_not_extend_the_semantic_idle_clock() {
    let result=decoded((0..20).map(|_|(Duration::from_secs(20),frame(json!({}),None))).collect(),
        KnowledgeWritebackLimits::default(),CancellationToken::new()).await.err().expect("the response must fail safely");
    assert_eq!(result.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_TIMEOUT"));
    assert_eq!(result.diagnostic.stage,Stage::FirstFrame);
    assert_eq!(result.diagnostic.elapsed_ms,Some(120_000));
    assert_eq!(result.diagnostic.output_bytes,Some(0));
    assert_eq!(result.diagnostic.received_frames,Some(5));
}

#[tokio::test(start_paused=true)]
async fn cancellation_drops_the_owned_stream_and_retains_real_progress() {
    let cancellation=CancellationToken::new();
    let trigger=cancellation.clone();
    let cancelled=tokio::spawn(async move {tokio::time::sleep(Duration::from_secs(60)).await;trigger.cancel();});
    let result=decoded(vec![
        (Duration::from_secs(10),frame(json!({"reasoning_content":"Grounded extraction is in progress."}),None)),
        (Duration::from_secs(90),frame(json!({"content":"{\"candidates\":[]}"}),None)),
    ],KnowledgeWritebackLimits::default(),cancellation).await.err().expect("the response must fail safely");
    cancelled.await.unwrap();
    assert_eq!(result.diagnostic.stage,Stage::Cancelled);
    assert_eq!(result.diagnostic.elapsed_ms,Some(60_000));
    assert_eq!(result.diagnostic.received_frames,Some(1));
    assert_eq!(result.diagnostic.first_frame_ms,Some(10_000));
    assert_eq!(result.diagnostic.output_bytes,Some(0));
}

#[tokio::test]
async fn output_and_terminal_limits_return_shared_errors_without_partial_json_success() {
    let limits=KnowledgeWritebackLimits {output_max_utf8_bytes:4,..Default::default()};
    let too_large=decoded(vec![(Duration::ZERO,frame(json!({"content":"{\"candidates\":[]}"}),None))],
        limits,CancellationToken::new()).await.err().expect("the response must fail safely");
    assert_eq!(too_large.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_INVALID_RESPONSE"));
    let truncated=decoded(vec![(Duration::ZERO,frame(json!({"content":"{\"candidates\":[]}"}),None)),
        (Duration::ZERO,finished("length"))],KnowledgeWritebackLimits::default(),CancellationToken::new()).await.err().expect("the response must fail safely");
    assert_eq!(truncated.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_INVALID_RESPONSE"));
    assert_eq!(truncated.diagnostic.stage,Stage::Decode);
    let missing=decoded(vec![(Duration::ZERO,frame(json!({"content":"{\"candidates\":[]}"}),None))],
        KnowledgeWritebackLimits::default(),CancellationToken::new()).await.err().expect("the response must fail safely");
    assert_eq!(missing.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_STREAM_INTERRUPTED"));
}

struct LocalServer {
    url:String,
    requests:Arc<Mutex<Vec<Value>>>,
    task:tokio::task::JoinHandle<()>,
}

impl LocalServer {
    async fn new(status:axum::http::StatusCode,body:Value)->Self {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url=format!("http://{}",listener.local_addr().unwrap());
        let requests=Arc::new(Mutex::new(Vec::new()));
        let app=Router::new().route("/chat/completions",post({let body=body.clone();move |State(requests):State<Arc<Mutex<Vec<Value>>>>,Json(request):Json<Value>| {
            let body=body.clone();async move {requests.lock().unwrap().push(request);(status,Json(body))}
        }})).with_state(requests.clone());
        let task=tokio::spawn(async move {axum::serve(listener,app).await.unwrap();});
        Self {url,requests,task}
    }

    async fn close(self) {self.task.abort();let _=self.task.await;}
}

#[tokio::test]
async fn declared_non_streaming_uses_the_shared_json_transport_and_unknown_output_limit_stays_absent() {
    let server=LocalServer::new(axum::http::StatusCode::OK,json!({"id":"writeback-json","choices":[{
        "index":0,"message":{"role":"assistant","content":"{\"candidates\":[]}"},"finish_reason":"stop"}],"usage":{"completion_tokens":8}})).await;
    let result=complete_resolved(configuration(&server.url),BTreeSet::from([ModelTechnicalCapability::Streaming]),
        request(KnowledgeWritebackLimits::default()),CancellationToken::new(),Observation::new()).await.unwrap();
    assert_eq!(result.text,"{\"candidates\":[]}");
    let requests=server.requests.lock().unwrap();
    assert_eq!(requests.len(),1);
    assert_eq!(requests[0]["stream"],false);
    assert!(requests[0].get("stream_options").is_none());
    assert!(requests[0].get("max_tokens").is_none());
    assert!(requests[0].get("response_format").is_none());
    assert!(requests[0].get("tools").is_none());
    drop(requests);
    assert!(result.diagnostic.received_bytes.is_some_and(|bytes|bytes>17));
    assert_eq!(result.diagnostic.received_frames,Some(1));
    result.diagnostic.validate().unwrap();
    server.close().await;
}

#[tokio::test]
async fn a_valid_small_declared_output_limit_is_preserved_on_the_wire() {
    let server=LocalServer::new(axum::http::StatusCode::OK,json!({"id":"small-output-response","choices":[{
        "index":0,"message":{"role":"assistant","content":"{\"candidates\":[]}"},"finish_reason":"stop"}],"usage":{"completion_tokens":8}})).await;
    let mut config=configuration(&server.url);config.context_limit=Some(512);config.output_limit=Some(8);
    let result=complete_resolved(config,BTreeSet::from([ModelTechnicalCapability::Streaming]),request(KnowledgeWritebackLimits::default()),
        CancellationToken::new(),Observation::new()).await.unwrap();
    assert_eq!(result.text,"{\"candidates\":[]}");
    assert_eq!(result.diagnostic.output_token_limit,Some(8));
    assert_eq!(result.diagnostic.output_tokens,Some(8));
    assert_eq!(server.requests.lock().unwrap()[0]["max_tokens"],8);
    server.close().await;
}

#[tokio::test]
async fn provider_failure_is_one_attempt_and_wire_budget_is_independent_of_visible_text() {
    let server=LocalServer::new(axum::http::StatusCode::SERVICE_UNAVAILABLE,json!({"error":{"code":"service_unavailable"}})).await;
    let result=complete_resolved(configuration(&server.url),BTreeSet::new(),request(KnowledgeWritebackLimits::default()),
        CancellationToken::new(),Observation::new()).await.err().expect("the response must fail safely");
    assert_eq!(result.diagnostic.stage,Stage::Connect);
    assert_eq!(server.requests.lock().unwrap().len(),1);
    assert_eq!(result.diagnostic.provider_diagnostic.as_ref().unwrap().http_status,Some(503));
    assert_eq!(result.diagnostic.provider_diagnostic.as_ref().unwrap().provider_id.as_deref(),Some("0190f5fe-7c00-7a00-8000-000000000001"));
    assert_eq!(result.diagnostic.provider_diagnostic.as_ref().unwrap().model_name.as_deref(),Some("arbitrary-chat-model"));
    assert_eq!(result.diagnostic.received_bytes,None,"HTTP error-body reads are not metered by the success-stream byte counter");
    server.close().await;
    let large=LocalServer::new(axum::http::StatusCode::OK,json!({"padding":"x".repeat(4096),"choices":[]})).await;
    let result=complete_resolved(configuration(&large.url),BTreeSet::new(),request(KnowledgeWritebackLimits {response_max_bytes:64,..Default::default()}),
        CancellationToken::new(),Observation::new()).await.err().expect("the response must fail safely");
    assert_eq!(result.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_INVALID_RESPONSE"));
    assert!(result.diagnostic.received_bytes.is_some_and(|bytes|bytes>64));
    assert_eq!(result.diagnostic.received_frames,Some(0));
    large.close().await;
}

#[tokio::test]
async fn known_output_limits_use_actual_usage_instead_of_mistaking_utf8_estimates_for_tokens() {
    let text=json!({"candidates":[{"kb_id":"0190f5fe-7c00-7a00-8000-000000000002","rel_path":"notes/a.md", "content":"a".repeat(6000)}]}).to_string();
    assert!(estimate_tokens(text.len())>1000);
    let response=|tokens:u64|json!({"id":"known-usage","choices":[{"index":0,"message":{"role":"assistant","content":text},
        "finish_reason":"stop"}],"usage":{"completion_tokens":tokens}});
    let server=LocalServer::new(axum::http::StatusCode::OK,response(1000)).await;
    let mut config=configuration(&server.url);config.output_limit=Some(1000);
    let result=complete_resolved(config,BTreeSet::from([ModelTechnicalCapability::Streaming]),request(KnowledgeWritebackLimits::default()),
        CancellationToken::new(),Observation::new()).await.unwrap();
    assert_eq!(result.text,text);assert_eq!(result.diagnostic.output_tokens,Some(1000));
    assert_eq!(server.requests.lock().unwrap()[0]["max_tokens"],1000);
    server.close().await;
    let server=LocalServer::new(axum::http::StatusCode::OK,response(1001)).await;
    let mut config=configuration(&server.url);config.output_limit=Some(1000);
    let result=complete_resolved(config,BTreeSet::from([ModelTechnicalCapability::Streaming]),request(KnowledgeWritebackLimits::default()),
        CancellationToken::new(),Observation::new()).await.err().expect("actual reported usage must stay inside the target");
    assert_eq!(result.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_INVALID_RESPONSE"));
    assert_eq!(result.diagnostic.output_tokens,Some(1001));
    assert_eq!(result.diagnostic.output_bytes,Some(text.len() as u64));
    server.close().await;
}

#[tokio::test]
async fn missing_output_usage_remains_unknown_instead_of_becoming_zero() {
    let server=LocalServer::new(axum::http::StatusCode::OK,json!({"id":"input-only-usage","choices":[{"index":0,
        "message":{"role":"assistant","content":"{\"candidates\":[]}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":25}})).await;
    let result=complete_resolved(configuration(&server.url),BTreeSet::from([ModelTechnicalCapability::Streaming]),request(KnowledgeWritebackLimits::default()),
        CancellationToken::new(),Observation::new()).await.unwrap();
    assert_eq!(result.diagnostic.output_tokens,None);
    server.close().await;
}

fn native(event:&str,data:Value)->SingleAttemptFrame {
    SingleAttemptFrame {event:event.into(),data,diagnostic:None}
}

async fn native_decoded(protocol:ChatProtocol,script:Vec<(Duration,SingleAttemptFrame)>)
    ->Result<KnowledgeWritebackCompletion,KnowledgeWritebackCompletionError> {
    let limits=KnowledgeWritebackLimits::default();
    let mut observation=Observation::new();observation.diagnostic.stage=Stage::FirstFrame;
    drain(protocol,frames(script),SingleAttemptResponseBudget::new(limits.response_max_bytes).unwrap(),limits,limits.output_max_tokens,
        true,CancellationToken::new(),observation).await
}

fn anthropic_start()->SingleAttemptFrame {
    native("message_start",json!({"type":"message_start","message":{"id":"native-thinking","type":"message","role":"assistant","content":[],"usage":{"input_tokens":5,"output_tokens":0}}}))
}

fn anthropic_finish()->Vec<(Duration,SingleAttemptFrame)> {
    vec![
        (Duration::ZERO,native("content_block_start",json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}))),
        (Duration::ZERO,native("content_block_delta",json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"{\"candidates\":[]}"}}))),
        (Duration::ZERO,native("content_block_stop",json!({"type":"content_block_stop","index":1}))),
        (Duration::ZERO,native("message_delta",json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":8}}))),
        (Duration::ZERO,native("message_stop",json!({"type":"message_stop"}))),
    ]
}

#[tokio::test(start_paused=true)]
async fn valid_opaque_thinking_blocks_count_as_progress_without_saving_their_data() {
    let mut script=vec![(Duration::ZERO,anthropic_start()),
        (Duration::from_secs(70),native("content_block_start",json!({"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"opaque-thinking-fixture"}}))),
        (Duration::ZERO,native("content_block_stop",json!({"type":"content_block_stop","index":0}))),
    ];
    let mut finish=anthropic_finish();finish[0].0=Duration::from_secs(70);script.extend(finish);
    let result=native_decoded(ChatProtocol::Anthropic,script).await.unwrap();
    assert_eq!(result.diagnostic.elapsed_ms,Some(140_000));assert_eq!(result.text,"{\"candidates\":[]}");
    assert!(!serde_json::to_string(&result.diagnostic).unwrap().contains("opaque-thinking-fixture"));
}

#[tokio::test(start_paused=true)]
async fn deferred_anthropic_thinking_deltas_keep_the_idle_clock_alive_until_the_atomic_block_closes() {
    let mut script=vec![(Duration::ZERO,anthropic_start()),
        (Duration::ZERO,native("content_block_start",json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}))),
        (Duration::from_secs(70),native("content_block_delta",json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"First grounded thought."}}))),
        (Duration::from_secs(70),native("content_block_delta",json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" Second grounded thought."}}))),
        (Duration::ZERO,native("content_block_delta",json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"fixture-signature"}}))),
        (Duration::ZERO,native("content_block_stop",json!({"type":"content_block_stop","index":0}))),
    ];script.extend(anthropic_finish());
    let result=native_decoded(ChatProtocol::Anthropic,script).await.unwrap();
    assert_eq!(result.diagnostic.elapsed_ms,Some(140_000));assert_eq!(result.text,"{\"candidates\":[]}");
}

#[tokio::test(start_paused=true)]
async fn deferred_responses_reasoning_summaries_are_progress_before_atomic_item_delivery() {
    let reasoning=json!({"id":"reason-item","type":"reasoning","status":"completed","summary":[{"type":"summary_text","text":"First fact. Second fact."}]});
    let message=json!({"id":"answer-item","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"{\"candidates\":[]}","annotations":[]}]});
    let script=vec![
        (Duration::ZERO,native("response.created",json!({"type":"response.created","response":{"id":"native-responses","status":"in_progress"}}))),
        (Duration::ZERO,native("response.output_item.added",json!({"type":"response.output_item.added","output_index":0,"item":{"id":"reason-item","type":"reasoning","summary":[]}}))),
        (Duration::from_secs(70),native("response.reasoning_summary_text.delta",json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"reason-item","summary_index":0,"delta":"First fact."}))),
        (Duration::from_secs(70),native("response.reasoning_summary_text.delta",json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"reason-item","summary_index":0,"delta":" Second fact."}))),
        (Duration::ZERO,native("response.output_item.done",json!({"type":"response.output_item.done","output_index":0,"item":reasoning}))),
        (Duration::ZERO,native("response.output_item.added",json!({"type":"response.output_item.added","output_index":1,"item":{"id":"answer-item","type":"message","role":"assistant","content":[]}}))),
        (Duration::ZERO,native("response.output_text.delta",json!({"type":"response.output_text.delta","output_index":1,"item_id":"answer-item","content_index":0,"delta":"{\"candidates\":[]}"}))),
        (Duration::ZERO,native("response.output_item.done",json!({"type":"response.output_item.done","output_index":1,"item":message}))),
        (Duration::ZERO,native("response.completed",json!({"type":"response.completed","response":{"id":"native-responses","status":"completed","output":[reasoning,message],"usage":{"output_tokens":8}}}))),
    ];
    let result=native_decoded(ChatProtocol::OpenaiResponses,script).await.unwrap();
    assert_eq!(result.diagnostic.elapsed_ms,Some(140_000));assert_eq!(result.text,"{\"candidates\":[]}");
}

#[tokio::test]
async fn known_unsupported_reasoning_is_rejected_before_any_provider_request() {
    let mut config=configuration("http://127.0.0.1:9");
    config.provider_params=json!({"reasoning_effort":"high"});
    let result=complete_resolved(config,BTreeSet::from([ModelTechnicalCapability::Reasoning]),request(KnowledgeWritebackLimits::default()),
        CancellationToken::new(),Observation::new()).await.err().expect("known unsupported reasoning must not be sent");
    assert_eq!(result.diagnostic.code.as_deref(),Some("USER_LLM_PROVIDER_UNSUPPORTED_FEATURE"));
    assert_eq!(result.diagnostic.stage,Stage::Validate);
    assert_eq!(result.diagnostic.received_bytes,Some(0));
    assert_eq!(result.diagnostic.received_frames,Some(0));
    assert_eq!(result.diagnostic.first_frame_ms,None);
}

#[test]
fn configuration_pin_compares_exact_invocation_authority_without_rejecting_unrelated_catalog_updates() {
    let original=configuration("http://127.0.0.1:9");
    let mut changed=original.clone();changed.config_revision+=1;
    assert!(original.matches_invocation_configuration(&changed));
    changed.output_limit=Some(2048);assert!(!original.matches_invocation_configuration(&changed));
    changed=original.clone();changed.connection.base_url="http://127.0.0.1:10".into();
    assert!(!original.matches_invocation_configuration(&changed));
    changed=original.clone();changed.connection.auth.credentials=json!({"api_keys":["other-local-test-key"]});
    assert!(!original.matches_invocation_configuration(&changed));
}

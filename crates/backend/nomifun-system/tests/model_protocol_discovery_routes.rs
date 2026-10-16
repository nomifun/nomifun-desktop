//! Local HTTP fixtures exercise production serializers, streaming decoders and
//! the actual harmless tool-result replay; no paid requests or Agent writes.
mod common;
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
use std::time::Duration;
use axum::body::Body;
use axum::http::{Request,StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value,json};
use tower::ServiceExt;
use wiremock::{Mock,MockServer,ResponseTemplate,Respond};
use wiremock::matchers::{header,method,path};
use nomifun_db::{CreateProviderParams,IProviderRepository,NewProviderModel,NewProviderModelCapability,
    SqliteProviderRepository,init_database_memory};
use nomifun_system::{VersionCheckService,system_routes};

const KEY: [u8;32] = [0x49;32];
const SECRET: &str = "setup-only-secret";
const ROUTE: &str = "/api/providers/discover-model-protocols";
const TOOL: &str = "nomifun_protocol_echo";

async fn setup() -> (axum::Router,nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let state = common::build_system_state(&db,KEY,client.clone(),VersionCheckService::new(client,"0.1.0".into()),
        std::env::temp_dir(),std::env::temp_dir(),false);
    (system_routes(state),db)
}
fn capability(protocol: &str) -> Value {json!({"task":"chat","protocol":protocol,"connection_role":"default","provider_params":{},"context_limit":100000,"output_limit":16000})}
fn anonymous(server: &MockServer, protocol: &str) -> Value {anonymous_at(&format!("{}/v1",server.uri()),protocol)}
fn anonymous_at(base_url: &str, protocol: &str) -> Value {json!({"platform":"openai","base_url":base_url,
    "auth_scheme":"bearer","credentials":{"api_keys":[SECRET]},"model":"exact-user-model","capability":capability(protocol)})}
async fn post(router: &axum::Router, uri: &str, body: Value) -> (StatusCode,Value) {
    let response = router.clone().oneshot(Request::builder().method("POST").uri(uri).header("content-type","application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let value: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(!value.to_string().contains(SECRET),"response must redact credentials"); (status,value)
}
fn frames(values: Vec<Value>) -> String {values.into_iter().map(|frame|format!("event: {}\ndata: {frame}\n\n",frame["type"].as_str().unwrap_or("error"))).collect()}
fn chat_round(tool: bool) -> String {
    let delta = if tool {json!({"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":TOOL,"arguments":"{\"value\":\"ok\"}"}}]})}
        else {json!({"content":"OK"})};
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":if tool {"tool_calls"} else {"stop"}}],"usage":{"prompt_tokens":3,"completion_tokens":1}}))
}
fn responses_round(tool: bool) -> String {
    // The production decoder corroborates the terminal output against each
    // item's output_item.done snapshot; real streams always send the item
    // lifecycle, so the fixture must too.
    let id = if tool {"response_1"} else {"response_2"};
    let mut events=vec![json!({"type":"response.created","response":{"id":id,"status":"in_progress","output":[]}})];
    let output = if tool {
        let reasoning=json!({"id":"reasoning_1","type":"reasoning","status":"completed","summary":[],"content":[{"type":"reasoning_text","text":"original reasoning"}]});
        let function=json!({"id":"function_1","call_id":"call_1","type":"function_call","name":TOOL,"arguments":"{\"value\":\"ok\"}","status":"completed"});
        events.extend([
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"reasoning_1","type":"reasoning","summary":[],"content":[]}}),
            json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"id":"function_1","call_id":"call_1","type":"function_call","name":TOOL,"arguments":"","status":"in_progress"}}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"item_id":"function_1","delta":"{\"value\":\"ok\"}"}),
            json!({"type":"response.function_call_arguments.done","output_index":1,"item_id":"function_1","arguments":"{\"value\":\"ok\"}"}),
            json!({"type":"response.output_item.done","output_index":1,"item":function}),
        ]);
        vec![reasoning,function]
    } else {
        let message=json!({"id":"message_1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"OK","annotations":[]}]});
        events.extend([
            json!({"type":"response.output_item.added","output_index":0,"item":{"id":"message_1","type":"message","role":"assistant","status":"in_progress","content":[]}}),
            json!({"type":"response.content_part.added","output_index":0,"item_id":"message_1","content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","output_index":0,"item_id":"message_1","content_index":0,"delta":"OK"}),
            json!({"type":"response.output_text.done","output_index":0,"item_id":"message_1","content_index":0,"text":"OK"}),
            json!({"type":"response.content_part.done","output_index":0,"item_id":"message_1","content_index":0,"part":{"type":"output_text","text":"OK","annotations":[]}}),
            json!({"type":"response.output_item.done","output_index":0,"item":message}),
        ]);
        vec![message]
    };
    events.push(json!({"type":"response.completed","response":{"id":id,"status":"completed","output":output,"usage":{"input_tokens":3,"output_tokens":1}}}));
    frames(events)
}
fn anthropic_round(tool: bool) -> String {
    let block = if tool {json!({"type":"tool_use","id":"call_1","name":TOOL,"input":{}})} else {json!({"type":"text","text":""})};
    let delta = if tool {json!({"type":"input_json_delta","partial_json":"{\"value\":\"ok\"}"})} else {json!({"type":"text_delta","text":"OK"})};
    let mut events=vec![json!({"type":"message_start","message":{"id":"m","type":"message","role":"assistant","content":[],"model":"exact-user-model","usage":{"input_tokens":3,"output_tokens":0}}})];
    if tool {events.extend([
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"original reasoning"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"original-anthropic-"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signature"}}),
        json!({"type":"content_block_stop","index":0}),
    ]);}
    let index=if tool {1} else {0};
    events.extend([json!({"type":"content_block_start","index":index,"content_block":block}),json!({"type":"content_block_delta","index":index,"delta":delta}),
        json!({"type":"content_block_stop","index":index}),json!({"type":"message_delta","delta":{"stop_reason":if tool {"tool_use"} else {"end_turn"}},"usage":{"output_tokens":1}}),json!({"type":"message_stop"})]);
    frames(events)
}
fn gemini_round(tool: bool) -> String {
    let part = if tool {json!({"functionCall":{"id":"call_1","name":TOOL,"args":{"value":"ok"}},"thoughtSignature":"original-thought-signature"})} else {json!({"text":"OK"})};
    format!("data: {}\n\n",json!({"candidates":[{"index":0,"content":{"role":"model","parts":[part]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":1}}))
}
#[derive(Clone)]
struct ToolRounds {protocol: &'static str,count:Arc<AtomicUsize>,delay:Duration}
impl Respond for ToolRounds {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        if self.protocol == "anthropic" && request.headers.get("anthropic-version")
            .and_then(|value| value.to_str().ok()) != Some("2023-06-01") {
            return ResponseTemplate::new(400).set_body_json(json!({"type":"error","error":{
                "type":"invalid_request_error","message":"anthropic-version must be 2023-06-01"}}));
        }
        let tool = self.count.fetch_add(1,Ordering::SeqCst).is_multiple_of(2);
        let stream = match self.protocol {"responses" => responses_round(tool),"anthropic" => anthropic_round(tool),"gemini" => gemini_round(tool),_ => chat_round(tool)};
        ResponseTemplate::new(200).set_delay(self.delay).set_body_raw(stream,"text/event-stream")
    }
}
async fn mount_tools(server: &MockServer, endpoint: &str, protocol: &'static str, auth_header: &str, key: &str, delay:Duration) {
    let value = if auth_header == "authorization" {format!("Bearer {key}")} else {key.into()};
    Mock::given(method("POST")).and(path(endpoint)).and(header(auth_header,value))
        .respond_with(ToolRounds {protocol,count:Arc::new(AtomicUsize::new(0)),delay}).expect(2).mount(server).await;
}
async fn openai_tools(server: &MockServer) {
    mount_tools(server,"/v1/responses","responses","authorization",SECRET,Duration::ZERO).await;
    mount_tools(server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
}
fn candidate<'a>(body: &'a Value, protocol: &str) -> &'a Value {body["data"]["candidates"].as_array().unwrap().iter().find(|candidate| candidate["protocol"] == protocol).unwrap()}

#[tokio::test]
async fn all_available_chat_protocols_are_verified_and_selection_respects_only_available_preferences() {
    for (preferred,selected) in [(None,"openai.responses"),(Some("openai.chat_text"),"openai.chat_text"),(Some("not-registered"),"openai.responses")] {
        let server=MockServer::start().await; openai_tools(&server).await;
        let (router,db)=setup().await;
        let mut draft=anonymous(&server,"openai.chat_text"); if let Some(preferred) = preferred {draft["preferred_protocol"]=json!(preferred);}
        let (status,body)=post(&router,ROUTE,draft).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["selected_protocol"],selected,"{body}");
        assert_eq!(body["data"]["candidates"].as_array().unwrap().len(),2);
        for protocol in ["openai.responses","openai.chat_text"] {
            let candidate=candidate(&body,protocol);
            assert_eq!(candidate["status"],"available","{body}");assert_eq!(candidate["evidence"],"verified_stream");
            assert_eq!(candidate["capability"]["protocol"],protocol);assert_eq!(candidate["capability"]["output_limit"],16000);
        }
        let received=server.received_requests().await.unwrap(); assert_eq!(received.len(),4);
        assert!(received.iter().all(|request| !request.headers.contains_key("anthropic-version")),
            "OpenAI requests must not inherit Anthropic protocol headers");
        let responses_replay: Value=serde_json::from_slice(&received.iter().filter(|request|request.url.path().ends_with("/responses")).nth(1).unwrap().body).unwrap();
        assert_eq!(responses_replay["input"][1]["type"],"reasoning");
        assert_eq!(responses_replay["input"][1]["content"],json!([{"type":"reasoning_text","text":"original reasoning"}]));
        assert!(responses_replay["input"].as_array().unwrap().iter().any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_1"));
        assert!(SqliteProviderRepository::new(db.pool().clone()).list().await.unwrap().is_empty());
    }
}

struct ConcurrentRounds {
    barrier: tokio::sync::Barrier,
    active: AtomicUsize,
    peak: AtomicUsize,
    rounds: [AtomicUsize;4],
    limit_responses: bool,
}
async fn concurrent_round(
    axum::extract::State(state): axum::extract::State<Arc<ConcurrentRounds>>,
    uri: axum::http::Uri,
) -> (StatusCode,[(axum::http::header::HeaderName,&'static str);1],String) {
    let index=match uri.path() {
        "/v1/responses" => 0,
        "/v1/chat/completions" => 1,
        "/v1/messages" => 2,
        "/v1beta/models/exact-user-model:streamGenerateContent" => 3,
        other => panic!("unexpected verification endpoint: {other}"),
    };
    let round=state.rounds[index].fetch_add(1,Ordering::SeqCst);
    if index<3 && round==0 {
        let active=state.active.fetch_add(1,Ordering::SeqCst)+1;
        state.peak.fetch_max(active,Ordering::SeqCst);
        // No first response is released until three distinct protocol calls
        // have arrived. A serial implementation cannot cross this barrier.
        state.barrier.wait().await;
        state.active.fetch_sub(1,Ordering::SeqCst);
    }
    if state.limit_responses && index==0 {
        return (StatusCode::TOO_MANY_REQUESTS,[(axum::http::header::CONTENT_TYPE,"application/json")],
            json!({"error":{"code":"rate_limit_exceeded","message":"fixture limit"}}).to_string());
    }
    let stream=match index {0=>responses_round(round==0),1=>chat_round(round==0),2=>anthropic_round(round==0),_=>gemini_round(round==0)};
    (StatusCode::OK,[(axum::http::header::CONTENT_TYPE,"text/event-stream")],stream)
}

#[tokio::test]
async fn chat_protocol_waves_dispatch_concurrently_preserve_every_result_and_bound_rate_limit_fanout() {
    for limited in [false,true] {
        let state=Arc::new(ConcurrentRounds {barrier:tokio::sync::Barrier::new(3),active:AtomicUsize::new(0),
            peak:AtomicUsize::new(0),rounds:std::array::from_fn(|_|AtomicUsize::new(0)),limit_responses:limited});
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root=format!("http://{}",listener.local_addr().unwrap());
        let fixture=axum::Router::new().fallback(concurrent_round).with_state(state.clone());
        let fixture_task=tokio::spawn(async move {axum::serve(listener,fixture).await.unwrap()});
        let (router,_)=setup().await;
        let mut draft=anonymous_at(&format!("{root}/v1"),"openai.responses");
        draft["platform"]=json!("custom");
        draft["preferred_protocol"]=json!("anthropic.messages");
        draft["connections"]=json!([
            {"role":"claude","base_url":root,"auth_scheme":"header_key:x-api-key","credentials":{"api_keys":[SECRET]}},
            {"role":"gemini","base_url":root,"auth_scheme":"header_key:x-goog-api-key","credentials":{"api_keys":[SECRET]}}
        ]);
        let (status,body)=tokio::time::timeout(Duration::from_secs(5),post(&router,ROUTE,draft)).await
            .expect("the concurrent protocol wave must cross the three-request barrier");
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(state.peak.load(Ordering::SeqCst),3,"all three first-wave protocols must be in flight together");
        let candidates=body["data"]["candidates"].as_array().unwrap();
        assert_eq!(candidates.iter().map(|candidate|candidate["protocol"].as_str().unwrap()).collect::<Vec<_>>(),
            ["openai.chat_text","openai.responses","anthropic.messages","gemini.generate_text"],"completion order must not reorder priorities");
        assert_eq!(body["data"]["selected_protocol"],"anthropic.messages","an available explicit preference remains authoritative");
        assert_eq!(state.rounds[1].load(Ordering::SeqCst),2);
        assert_eq!(state.rounds[2].load(Ordering::SeqCst),2);
        for protocol in ["openai.chat_text","anthropic.messages"] {assert_eq!(candidate(&body,protocol)["status"],"available","{body}");}
        if limited {
            assert_eq!(state.rounds[0].load(Ordering::SeqCst),1,"rate limits do not cause retries");
            assert_eq!(state.rounds[3].load(Ordering::SeqCst),0,"later waves cannot amplify a known account failure");
            assert_eq!(candidate(&body,"openai.responses")["error_kind"],"rate_limited");
            assert_eq!(candidate(&body,"gemini.generate_text")["status"],"blocked");
        } else {
            assert!(candidates.iter().all(|candidate|candidate["status"]=="available"),"{body}");
            assert!(state.rounds.iter().all(|rounds|rounds.load(Ordering::SeqCst)==2));
        }
        fixture_task.abort();
    }
}

#[tokio::test]
async fn verified_stepfun_discovery_preserves_default_limits_as_omitted_fields() {
    for chat_available in [true,false] {
        let server=MockServer::start().await;
        mount_tools(&server,"/v1/responses","responses","authorization",SECRET,Duration::ZERO).await;
        if chat_available {
            mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
        } else {
            Mock::given(method("POST")).and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(405)).expect(1).mount(&server).await;
        }
        let (router,db)=setup().await;
        let mut draft=anonymous(&server,"openai.chat_text");
        draft["platform"]=json!("stepfun");draft["model"]=json!("step-5-preview");
        draft["capability"]["context_limit"]=json!(1000000);
        draft["capability"]["output_limit"]=Value::Null;
        draft["capability"]["compaction_threshold_pct"]=Value::Null;
        let (status,body)=post(&router,ROUTE,draft).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["selected_protocol"],if chat_available {"openai.chat_text"} else {"openai.responses"},"{body}");
        for protocol in ["openai.responses","openai.chat_text"] {
            let candidate=candidate(&body,protocol);
            assert_eq!(candidate["status"],if protocol=="openai.responses" || chat_available {"available"} else {"incompatible"},"{body}");
            assert_eq!(candidate["capability"]["context_limit"],1000000);
            assert!(candidate["capability"].get("output_limit").is_none());
            assert!(candidate["capability"].get("compaction_threshold_pct").is_none());
            assert_eq!(candidate["probe_output_limit"],4096);
        }
        let selected=&body["data"]["selected_capability"];
        assert_eq!(selected["context_limit"],1000000);
        assert!(selected.get("output_limit").is_none());
        assert!(selected.get("compaction_threshold_pct").is_none());
        assert!(SqliteProviderRepository::new(db.pool().clone()).list().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn the_configured_output_ceiling_is_sent_exactly_like_agent_turns_send_it() {
    let server=MockServer::start().await; openai_tools(&server).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"openai.chat_text");
    draft["capability"]["output_limit"]=json!(131072);
    let (status,body)=post(&router,ROUTE,draft).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    for protocol in ["openai.responses","openai.chat_text"] {
        assert_eq!(candidate(&body,protocol)["probe_output_limit"],131072,"{body}");
    }
    for request in &server.received_requests().await.unwrap() {
        let wire:Value=serde_json::from_slice(&request.body).unwrap();
        if request.url.path().ends_with("/responses") {
            assert_eq!(wire["max_output_tokens"],131072,"{wire}");
        } else {
            assert_eq!(wire["max_tokens"],131072,"{wire}");
        }
    }
}

#[tokio::test]
async fn plain_text_success_does_not_claim_agent_support() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses")).respond_with(ResponseTemplate::new(200).set_body_raw(responses_round(false),"text/event-stream")).expect(1).mount(&server).await;
    mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
    let (router,_)=setup().await;let (_,body)=post(&router,ROUTE,anonymous(&server,"openai.responses")).await;
    assert_eq!(candidate(&body,"openai.responses")["status"],"inconclusive");
    assert_eq!(body["data"]["selected_protocol"],"openai.chat_text","{body}");
}

#[tokio::test]
async fn verification_never_executes_or_acknowledges_an_unexpected_tool_proposal() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(responses_round(true).replace(TOOL,"delete_user_files"),"text/event-stream"))
        .expect(1).mount(&server).await;
    mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
    let (router,_)=setup().await;let (_,body)=post(&router,ROUTE,anonymous(&server,"openai.responses")).await;
    assert_eq!(candidate(&body,"openai.responses")["status"],"inconclusive");
    assert_eq!(body["data"]["selected_protocol"],"openai.chat_text");
    let requests=server.received_requests().await.unwrap();
    assert_eq!(requests.iter().filter(|request| request.url.path().ends_with("/responses")).count(),1);
    assert!(requests.iter().all(|request| !String::from_utf8_lossy(&request.body).contains("delete_user_files")));
}

#[tokio::test]
async fn retired_vendor_audit_does_not_disable_a_user_managed_working_relay() {
    let server=MockServer::start().await;openai_tools(&server).await;
    let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.chat_text");
    draft["platform"]=json!("lingyi");draft["preferred_protocol"]=json!("openai.responses");
    let (_,body)=post(&router,ROUTE,draft).await;
    assert_eq!(body["data"]["selected_protocol"],"openai.responses","{body}");
    assert!(body["data"]["candidates"].as_array().unwrap().iter().all(|candidate| candidate["status"]=="available"));
    assert_eq!(server.received_requests().await.unwrap().len(),4);
}

#[tokio::test]
async fn authentication_quota_and_rate_limits_preserve_inflight_results_without_retrying() {
    for (status,code,kind) in [(401,"invalid_api_key","unauthorized"),(403,"forbidden","forbidden"),(400,"insufficient_quota","insufficient_quota"),(429,"rate_limit_exceeded","rate_limited")] {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(status).insert_header("retry-after","0").set_body_json(json!({"error":{"code":code,"message":SECRET}}))).expect(1).mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(status).insert_header("retry-after","0").set_body_json(json!({"error":{"code":code,"message":SECRET}}))).expect(1).mount(&server).await;
        let (router,_)=setup().await;let (_,body)=post(&router,ROUTE,anonymous(&server,"openai.responses")).await;
        assert_eq!(candidate(&body,"openai.responses")["status"],"inconclusive");assert_eq!(candidate(&body,"openai.responses")["error_kind"],kind);
        assert_eq!(candidate(&body,"openai.chat_text")["status"],"inconclusive");
        assert_eq!(candidate(&body,"openai.chat_text")["error_kind"],kind);assert!(body["data"].get("selected_capability").is_none());
        assert_eq!(server.received_requests().await.unwrap().len(),2);
    }
}

#[tokio::test]
async fn authentication_billing_and_rate_limit_html_keep_their_actual_stop_authority() {
    for (status,kind) in [(401,"unauthorized"),(402,"insufficient_quota"),(403,"forbidden"),(429,"rate_limited")] {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(status).set_body_raw("<!doctype html><html>account gateway</html>","text/html")).expect(1).mount(&server).await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(status).set_body_raw("<!doctype html><html>account gateway</html>","text/html")).expect(1).mount(&server).await;
        let (router,_)=setup().await;let (_,body)=post(&router,ROUTE,anonymous(&server,"openai.responses")).await;
        let first=candidate(&body,"openai.responses");assert_eq!(first["status"],"inconclusive");
        assert_eq!(first["http_status"],status);assert_eq!(first["error_kind"],kind);
        let second=candidate(&body,"openai.chat_text");assert_eq!(second["status"],"inconclusive");
        assert_eq!(second["http_status"],status);assert_eq!(second["error_kind"],kind);
        assert_eq!(server.received_requests().await.unwrap().len(),2);
    }
}

#[tokio::test]
async fn protocol_scoped_html_and_server_failures_do_not_block_a_working_chat_candidate() {
    for (status,content_type,body) in [(200,"text/html","<!doctype html><html>gateway app</html>"),(503,"application/json","{\"error\":{\"code\":\"unavailable\"}}")] {
        let server=MockServer::start().await;
        // Verification executes one attempt per request; no transport retries.
        Mock::given(method("POST")).and(path("/v1/responses")).respond_with(ResponseTemplate::new(status).set_body_raw(body,content_type)).expect(1).mount(&server).await;
        mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
        let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.responses");draft["preferred_protocol"]=json!("openai.responses");
        let (_,body)=post(&router,ROUTE,draft).await;
        assert_eq!(candidate(&body,"openai.responses")["status"],"inconclusive");assert_eq!(candidate(&body,"openai.chat_text")["status"],"available");
        assert_eq!(body["data"]["selected_protocol"],"openai.chat_text");
    }
}

#[tokio::test]
async fn a_responses_timeout_does_not_prevent_verifying_the_available_chat_protocol() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(21)).set_body_raw(responses_round(true),"text/event-stream"))
        .expect(1).mount(&server).await;
    mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
    let (router,_)=setup().await;let (_,body)=post(&router,ROUTE,anonymous(&server,"openai.responses")).await;
    assert_eq!(candidate(&body,"openai.responses")["status"],"inconclusive");
    assert_eq!(candidate(&body,"openai.responses")["error_kind"],"timeout");
    assert_eq!(candidate(&body,"openai.chat_text")["status"],"available");
    assert_eq!(body["data"]["selected_protocol"],"openai.chat_text");
    assert_eq!(body["data"]["candidates"].as_array().unwrap().len(),2);
}

#[tokio::test]
async fn structured_sse_rejection_is_distinct_from_transport_failure_and_can_find_an_alternative() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses")).respond_with(ResponseTemplate::new(200).set_body_raw(
        frames(vec![json!({"type":"error","code":"unsupported_tool_calls","message":"Use Chat Completions"})]),"text/event-stream")).expect(1).mount(&server).await;
    mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
    let (router,_)=setup().await;let (_,body)=post(&router,ROUTE,anonymous(&server,"openai.responses")).await;
    assert_eq!(candidate(&body,"openai.responses")["status"],"incompatible");assert_eq!(body["data"]["selected_protocol"],"openai.chat_text","{body}");
}

#[tokio::test]
async fn a_typed_unsupported_parameter_rejection_marks_only_that_protocol_incompatible() {
    let server=MockServer::start().await;
    mount_tools(&server,"/v1/responses","responses","authorization",SECRET,Duration::ZERO).await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":{"code":"unsupported_parameter",
            "type":"invalid_request_error","message":"Function tools with reasoning_effort are not supported for gpt-6-sol in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'."}})))
        .expect(1).mount(&server).await;
    let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.responses");draft["preferred_protocol"]=json!("openai.chat_text");
    let (_,body)=post(&router,ROUTE,draft).await;
    let chat=candidate(&body,"openai.chat_text");
    assert_eq!(chat["status"],"incompatible","{body}");assert_eq!(chat["error_kind"],"invalid_request");assert_eq!(chat["http_status"],400);
    assert_eq!(body["data"]["selected_protocol"],"openai.responses","{body}");
}

#[tokio::test]
async fn equivalent_output_policies_are_tested_and_unmapped_policies_never_silently_disappear() {
    let server=MockServer::start().await;openai_tools(&server).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"openai.chat_text");
    draft["capability"]["provider_params"]=json!({"thinking":{"type":"disabled"},"response_format":{"type":"json_object"},"verbosity":"low","stream_options":{"include_usage":false}});
    let (_,body)=post(&router,ROUTE,draft).await;
    let selected_params=&body["data"]["selected_capability"]["provider_params"];
    assert_eq!(selected_params["thinking"],json!({"type":"disabled"}));
    assert_eq!(selected_params["text"],json!({"format":{"type":"json_object"},"verbosity":"low"}));
    assert!(selected_params.get("response_format").is_none());assert!(selected_params.get("stream_options").is_none());
    let received=server.received_requests().await.unwrap();
    let tested: Value=serde_json::from_slice(&received.iter().find(|request|request.url.path().ends_with("/responses")).unwrap().body).unwrap();
    assert_eq!(tested["thinking"],json!({"type":"disabled"}));assert_eq!(tested["text"],json!({"format":{"type":"json_object"},"verbosity":"low"}));
    assert!(tested.get("response_format").is_none());
    let other=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses")).respond_with(ResponseTemplate::new(405)).expect(1).mount(&other).await;
    let mut draft=anonymous(&other,"openai.responses");draft["capability"]["provider_params"]=json!({"text":{"future_policy":"keep"}});
    let (_,body)=post(&router,ROUTE,draft).await;
    assert_eq!(candidate(&body,"openai.chat_text")["status"],"blocked");
    assert!(candidate(&body,"openai.chat_text")["message"].as_str().unwrap().contains("provider_params.text"));
    assert_eq!(other.received_requests().await.unwrap().len(),1);assert!(body["data"].get("selected_capability").is_none());
}

#[tokio::test]
async fn documented_defaults_and_current_params_remain_scoped_to_the_protocol_actually_verified() {
    for source in ["openai.chat_text","openai.responses"] {
        let server=MockServer::start().await;openai_tools(&server).await;
        let (router,_)=setup().await;let mut draft=anonymous(&server,source);
        draft["platform"]=json!("stepfun");draft["model"]=json!("step-5-preview");
        draft["capability"]["provider_params"]=if source=="openai.chat_text" {
            json!({"reasoning_effort":"high","max_tokens_field":"max_completion_tokens","stream_options":{"include_usage":false},"require_reasoning_content":true,"vendor_option":7})
        } else {json!({"reasoning_effort":"high","chain_rounds":false,"include":null,"vendor_option":7})};
        let (_,body)=post(&router,ROUTE,draft).await;
        for protocol in ["openai.chat_text","openai.responses"] {
            let current=candidate(&body,protocol);assert_eq!(current["status"],"available","{body}");
            assert_eq!(current["capability"]["provider_params"]["reasoning_effort"],"high");
            assert_eq!(current["capability"]["provider_params"]["vendor_option"],7);
            assert_eq!(current["capability"]["output_limit"],16000);
        }
        let chat=&candidate(&body,"openai.chat_text")["capability"]["provider_params"];
        assert!(chat.get("include").is_none());assert!(chat.get("chain_rounds").is_none());
        let responses=&candidate(&body,"openai.responses")["capability"]["provider_params"];
        assert!(responses.get("stream_options").is_none());assert!(responses.get("require_reasoning_content").is_none());
        let requests=server.received_requests().await.unwrap();
        for request in requests {
            let wire:Value=serde_json::from_slice(&request.body).unwrap();
            if request.url.path().ends_with("/responses") {
                assert_eq!(wire["reasoning"]["effort"],"high");assert_eq!(wire["max_output_tokens"],16000);
                assert!(wire.get("stream_options").is_none());assert!(wire.get("include").is_none());
            } else {
                assert_eq!(wire["reasoning_effort"],"high");assert!(wire.get("include").is_none());
                if source=="openai.chat_text" {assert_eq!(wire["max_completion_tokens"],16000);assert_eq!(wire["stream_options"]["include_usage"],false);}
            }
            assert!(wire.get("chain_rounds").is_none());assert!(wire.get("require_reasoning_content").is_none());
        }
    }
}

#[tokio::test]
async fn explicitly_cleared_parameters_stay_empty_while_untouched_drafts_receive_documented_defaults() {
    for apply in [None,Some(true),Some(false)] {
        let server=MockServer::start().await;openai_tools(&server).await;
        let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.responses");
        draft["platform"]=json!("stepfun");draft["model"]=json!("step-5-preview");
        draft["preferred_protocol"]=json!("openai.responses");
        if let Some(apply) = apply {draft["apply_recommended_defaults"]=json!(apply);}
        let (status,body)=post(&router,ROUTE,draft).await;assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["selected_protocol"],"openai.responses","{body}");
        let params=&body["data"]["selected_capability"]["provider_params"];
        if apply==Some(false) {assert_eq!(params,&json!({}));}
        else {assert_eq!(params.get("include"),Some(&Value::Null),"omitting the flag preserves the fresh-draft default");}
    }
}

#[tokio::test]
async fn native_anthropic_uses_its_declared_header_and_real_tool_result_replay() {
    let server=MockServer::start().await;
    mount_tools(&server,"/v1/messages","anthropic","x-api-key",SECRET,Duration::ZERO).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"anthropic.messages");draft["platform"]=json!("anthropic");draft["auth_scheme"]=json!("header_key:x-api-key");draft["base_url"]=json!(server.uri());
    let (_,body)=post(&router,ROUTE,draft).await;
    assert_eq!(body["data"]["selected_protocol"],"anthropic.messages","{body}");assert_eq!(body["data"]["candidates"].as_array().unwrap().len(),1);
    let requests=server.received_requests().await.unwrap();
    assert_eq!(requests.len(),2,"Anthropic discovery must verify both real tool rounds");
    for request in &requests {
        assert_eq!(request.headers.get("anthropic-version").unwrap(),"2023-06-01");
        assert_eq!(request.headers.get_all("anthropic-version").iter().count(),1);
        assert_eq!(request.headers.get("x-api-key").unwrap(),SECRET);
        assert!(!request.headers.contains_key("authorization"));
    }
    let replay:Value=serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(replay["messages"][1]["content"][0]["signature"],"original-anthropic-signature");
    assert_eq!(replay["messages"][1]["content"][1]["type"],"tool_use");assert_eq!(replay["messages"][2]["content"][0]["type"],"tool_result");
}

#[tokio::test]
async fn native_gemini_preserves_the_exact_tool_thought_signature_and_model_endpoint() {
    let server=MockServer::start().await;
    mount_tools(&server,"/v1beta/models/exact-user-model:streamGenerateContent","gemini","x-goog-api-key",SECRET,Duration::ZERO).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"gemini.generate_text");draft["platform"]=json!("gemini");draft["auth_scheme"]=json!("header_key:x-goog-api-key");draft["base_url"]=json!(server.uri());
    let (_,body)=post(&router,ROUTE,draft).await;
    assert_eq!(body["data"]["selected_protocol"],"gemini.generate_text","{body}");assert_eq!(body["data"]["candidates"].as_array().unwrap().len(),1);
    let requests=server.received_requests().await.unwrap();
    assert_eq!(requests.len(),2);
    assert!(requests.iter().all(|request| !request.headers.contains_key("anthropic-version")),
        "Gemini requests must not inherit Anthropic protocol headers");
    let replay:Value=serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(replay["contents"][1]["parts"][0]["thoughtSignature"],"original-thought-signature");
    assert_eq!(replay["contents"][2]["parts"][0]["functionResponse"]["id"],"call_1");
    assert!(!requests[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn non_chat_registry_only_candidates_never_submit_paid_generation_jobs() {
    let server=MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"future-user-image-model"}]}))).expect(1).mount(&server).await;
    let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.images");draft["model"]=json!("future-user-image-model");
    draft["capability"]=json!({"task":"image_generation","protocol":"openai.images","connection_role":"default","provider_params":{}});
    let (status,body)=post(&router,ROUTE,draft).await;assert_eq!(status,StatusCode::OK,"{body}");
    assert!(body["data"].get("selected_capability").is_none(),"{body}");
    assert!(body["data"]["candidates"].as_array().unwrap().iter().all(|candidate| candidate["status"] != "available"));
    assert_eq!(body["data"]["candidates"].as_array().unwrap().len(),1,"unrelated vendor serializers must not be listed");
    assert_eq!(body["data"]["candidates"][0]["protocol"],"openai.images");
    assert!(server.received_requests().await.unwrap().iter().all(|request| request.method.as_str() == "GET"));
}

#[tokio::test]
async fn model_documentation_never_replaces_a_working_relay_root_or_explicit_endpoint() {
    let server=MockServer::start().await;
    Mock::given(method("GET")).and(path("/proxy/api/v3/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]}))).expect(2).mount(&server).await;
    let (router,_)=setup().await;
    for explicit in [false,true] {
        let mut draft=anonymous(&server,"ark.images");draft["platform"]=json!("ArK-Agent-Plan");draft["model"]=json!("doubao-seedream-5-0-pro");
        draft["base_url"]=json!(format!("{}/proxy/api/v3",server.uri()));
        draft["capability"]=json!({"task":"image_generation","protocol":"ark.images","connection_role":"default","provider_params":{"watermark":false}});
        if explicit {draft["capability"]["base_url_override"]=json!(format!("{}/custom/api/v3",server.uri()));draft["capability"]["endpoint"]=json!("/tenant-image");}
        let (status,body)=post(&router,ROUTE,draft.clone()).await;assert_eq!(status,StatusCode::OK,"{body}");
        let chosen=&body["data"]["selected_capability"];
        assert_eq!(chosen["protocol"],"ark.images","{body}");
        assert_eq!(candidate(&body,"ark.images")["evidence"],"official_model");
        assert_eq!(chosen["base_url_override"].as_str(),draft["capability"]["base_url_override"].as_str());
        if explicit {assert_eq!(chosen["endpoint"],"/tenant-image");}
        assert_eq!(chosen["provider_params"]["watermark"],false);
        assert_eq!(body["data"]["candidates"].as_array().unwrap().len(),1);
    }
    assert!(server.received_requests().await.unwrap().iter().all(|request| request.method.as_str()=="GET"));
}

#[tokio::test]
async fn catalog_task_evidence_is_read_from_and_bound_to_the_selected_role_and_root() {
    for (declared,override_root,available) in [(true,false,true),(false,false,false),(true,true,false)] {
        let default_server=MockServer::start().await;let selected=MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"regional-image-id","tasks":["image_generation"]}]})))
            .expect(0).mount(&default_server).await;
        Mock::given(method("GET")).and(path("/region/v1/models")).and(header("authorization","Bearer named-media-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"regional-image-id","tasks":if declared {vec!["image_generation"]} else {vec!["chat"]}}]})))
            .expect(1).mount(&selected).await;
        let (router,_)=setup().await;let mut draft=anonymous(&default_server,"siliconflow.images");draft["platform"]=json!("siliconflow");draft["model"]=json!("regional-image-id");
        draft["capability"]=json!({"task":"image_generation","protocol":"siliconflow.images","connection_role":"media","provider_params":{}});
        draft["connections"]=json!([{"role":"media","base_url":format!("{}/region/v1",selected.uri()),"auth_scheme":"bearer","credentials":{"api_keys":["named-media-key"]}}]);
        if override_root {draft["capability"]["base_url_override"]=json!(format!("{}/another/v1",selected.uri()));}
        let (status,body)=post(&router,ROUTE,draft).await;assert_eq!(status,StatusCode::OK,"{body}");assert!(!body.to_string().contains("named-media-key"));
        let current=candidate(&body,"siliconflow.images");
        if available {
            assert_eq!(current["status"],"available","{body}");assert_eq!(current["evidence"],"catalog_task");
            assert_eq!(body["data"]["selected_capability"]["connection_role"],"media");
        } else {assert_eq!(current["status"],"inconclusive","{body}");assert!(body["data"].get("selected_capability").is_none());}
        assert!(selected.received_requests().await.unwrap().iter().all(|request| request.method.as_str()=="GET"));
        assert!(default_server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn catalog_discovery_never_relabels_a_native_connection_key_as_gateway_bearer_auth() {
    let server=MockServer::start().await;
    let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.images");draft["platform"]=json!("nomifun-model-gateway");
    draft["capability"]=json!({"task":"image_generation","protocol":"openai.images","connection_role":"native","provider_params":{}});
    draft["connections"]=json!([{"role":"native","base_url":format!("{}/v1",server.uri()),"auth_scheme":"header_key:x-api-key","credentials":{"api_keys":["native-connection-key"]}}]);
    let (status,body)=post(&router,ROUTE,draft).await;assert_eq!(status,StatusCode::OK,"{body}");
    assert!(body["data"].get("selected_capability").is_none());assert!(server.received_requests().await.unwrap().is_empty());
    assert!(!body.to_string().contains("native-connection-key"));
}

#[tokio::test]
async fn explicit_custom_endpoints_and_cross_origin_credential_guards_apply_to_every_candidate() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/custom-call")).respond_with(ResponseTemplate::new(405)).expect(1).mount(&server).await;
    let (router,_)=setup().await;
    let mut custom=anonymous(&server,"openai.responses");custom["capability"]["endpoint"]=json!("/custom-call");
    let (_,body)=post(&router,ROUTE,custom).await;
    assert_eq!(candidate(&body,"openai.responses")["status"],"incompatible");
    assert_eq!(candidate(&body,"openai.chat_text")["status"],"blocked");
    assert!(candidate(&body,"openai.chat_text")["message"].as_str().unwrap().contains("custom endpoint"));
    let mut cross_origin=anonymous(&server,"openai.responses");cross_origin["capability"]["base_url_override"]=json!("https://different-origin.invalid/v1");
    let (_,body)=post(&router,ROUTE,cross_origin).await;
    assert!(body["data"].get("selected_capability").is_none());
    assert!(body["data"]["candidates"].as_array().unwrap().iter().all(|candidate| candidate["status"] == "blocked"));
    assert_eq!(server.received_requests().await.unwrap().len(),1,"no credentials may be sent to an unconsented origin");
}

#[tokio::test]
async fn native_protocol_parameter_choices_are_reported_as_blocked_before_any_native_request() {
    let server=MockServer::start().await;let native=MockServer::start().await;openai_tools(&server).await;
    let (router,_)=setup().await;let mut draft=anonymous(&server,"openai.chat_text");draft["platform"]=json!("custom");
    draft["capability"]["provider_params"]=json!({"vendor_extension":{"keep":true}});
    draft["connections"]=json!([{"role":"native","base_url":native.uri(),"auth_scheme":"header_key:x-api-key","credentials":{"api_keys":["native-choice-key"]}}]);
    let (status,body)=post(&router,ROUTE,draft).await;assert_eq!(status,StatusCode::OK,"{body}");
    let blocked=candidate(&body,"anthropic.messages");assert_eq!(blocked["status"],"blocked");assert_eq!(blocked["error_kind"],"invalid_request");
    assert!(blocked["message"].as_str().unwrap().contains("native protocol parameters"));assert!(blocked.get("capability").is_none());
    assert!(native.received_requests().await.unwrap().is_empty());assert!(!body.to_string().contains("native-choice-key"));
    assert_eq!(body["data"]["selected_capability"]["provider_params"]["vendor_extension"],json!({"keep":true}));
}

async fn stored_provider(db: &nomifun_db::Database, base_url: &str) -> String {
    let encrypted=nomifun_common::encrypt_string(&json!({"api_keys":[SECRET]}).to_string(),&KEY).unwrap();
    let capabilities=[NewProviderModelCapability {task:"chat",traits:"[]",protocol:"openai.responses",connection_role:"default",provider_params:"{}",..Default::default()}];
    let model=NewProviderModel {model:"original-model",enabled:true,capabilities:&capabilities,..Default::default()};
    SqliteProviderRepository::new(db.pool().clone()).create(CreateProviderParams {provider_id:None,platform:"openai",name:"Stored",base_url,
        auth_scheme:"bearer",credentials_encrypted:&encrypted,enabled:true,bedrock_config:None,sort_order:None},&model,&[]).await.unwrap().0.provider_id
}

#[tokio::test]
async fn stored_named_credentials_are_used_without_persisting_discovered_capabilities() {
    use nomifun_db::{IProviderConnectionRepository,IProviderModelRepository,SqliteProviderConnectionRepository,SqliteProviderModelRepository,UpsertProviderConnectionParams};
    let server=MockServer::start().await;
    mount_tools(&server,"/named/responses","responses","authorization",SECRET,Duration::ZERO).await;
    mount_tools(&server,"/named/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
    let (router,db)=setup().await;let id=stored_provider(&db,"https://unused-default.invalid/v1").await;
    let providers=SqliteProviderRepository::new(db.pool().clone());let current=providers.find_by_id(&id).await.unwrap().unwrap();
    let encrypted=nomifun_common::encrypt_string(&json!({"api_keys":[SECRET]}).to_string(),&KEY).unwrap();
    SqliteProviderConnectionRepository::new(db.pool().clone()).upsert(&id,current.config_revision,&UpsertProviderConnectionParams {role:"chat",label:None,
        base_url:&format!("{}/named",server.uri()),auth_scheme:"bearer",credentials_encrypted:&encrypted,extra:"{}"}).await.unwrap();
    let before=providers.find_by_id(&id).await.unwrap().unwrap();let mut draft=capability("openai.responses");draft["connection_role"]=json!("chat");
    let (status,body)=post(&router,&format!("/api/providers/{id}/discover-model-protocols"),json!({"model":"exact-user-model","capability":draft,"apply_recommended_defaults":false})).await;
    assert_eq!(status,StatusCode::OK,"{body}");assert_eq!(body["data"]["selected_capability"]["connection_role"],"chat");assert_eq!(body["data"]["config_revision"],before.config_revision);
    assert_eq!(body["data"]["selected_capability"]["provider_params"],json!({}),"persisted/user-owned empty parameters must not be refilled");
    let after=providers.find_by_id(&id).await.unwrap().unwrap();assert_eq!(after.config_revision,before.config_revision);assert_eq!(after.credentials_encrypted,before.credentials_encrypted);
    let models=SqliteProviderModelRepository::new(db.pool().clone()).list_for_provider(&id).await.unwrap();assert_eq!(models.len(),1);assert_eq!(models[0].model,"original-model");
}

#[tokio::test]
async fn provider_revision_changes_invalidate_the_entire_discovery_result() {
    let server=MockServer::start().await;
    mount_tools(&server,"/v1/responses","responses","authorization",SECRET,Duration::from_millis(100)).await;
    mount_tools(&server,"/v1/chat/completions","chat","authorization",SECRET,Duration::ZERO).await;
    let (router,db)=setup().await;let id=stored_provider(&db,&format!("{}/v1",server.uri())).await;let uri=format!("/api/providers/{id}/discover-model-protocols");
    let task=tokio::spawn(async move {post(&router,&uri,json!({"model":"exact-user-model","capability":capability("openai.responses")})).await});
    tokio::time::timeout(Duration::from_secs(5),async {while server.received_requests().await.unwrap().is_empty() {tokio::time::sleep(Duration::from_millis(5)).await;}}).await.unwrap();
    nomifun_db::sqlx::query("UPDATE providers SET config_revision=config_revision+1 WHERE provider_id=?").bind(&id).execute(db.pool()).await.unwrap();
    let (status,body)=task.await.unwrap();assert_eq!(status,StatusCode::CONFLICT,"{body}");assert!(!body.to_string().contains("selected_capability"));
}

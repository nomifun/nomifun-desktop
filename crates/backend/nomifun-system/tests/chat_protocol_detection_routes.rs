//! Real production serializers/stream decoders against bounded local fixtures.
mod common;
use std::time::Duration;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{header, method, path};
use nomifun_db::{CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderRepository, init_database_memory};
use nomifun_system::{VersionCheckService, system_routes};

const KEY: [u8;32] = [0x49;32];
const SECRET: &str = "setup-only-secret";

async fn setup() -> (axum::Router, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let state = common::build_system_state(&db,KEY,client.clone(),VersionCheckService::new(client,"0.1.0".into()),
        std::env::temp_dir(),std::env::temp_dir(),false);
    (system_routes(state),db)
}
fn capability(protocol: &str) -> Value {
    json!({"task":"chat","protocol":protocol,"connection_role":"default","provider_params":{},
        "context_limit":100000,"output_limit":16000})
}
fn anonymous(server: &MockServer, protocol: &str) -> Value {
    json!({"platform":"custom","base_url":format!("{}/v1",server.uri()),"auth_scheme":"bearer",
        "credentials":{"api_keys":[SECRET]},"model":"exact-user-model","capability":capability(protocol)})
}
async fn post(router: &axum::Router, uri: &str, body: Value) -> (StatusCode,Value) {
    let response = router.clone().oneshot(Request::builder().method("POST").uri(uri)
        .header("content-type","application/json").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(!value.to_string().contains(SECRET),"response must not contain credentials");
    (status,value)
}
fn chat_success() -> String {
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":"OK"},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":1}}))
}
fn responses_success() -> String {
    let item = json!({"id":"message_1","type":"message","role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":"OK","annotations":[]}]});
    let frames = [
        json!({"type":"response.created","response":{"id":"response_1","status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"message_1","type":"message","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.output_text.delta","output_index":0,"item_id":"message_1","content_index":0,"delta":"OK"}),
        json!({"type":"response.output_item.done","output_index":0,"item":item}),
        json!({"type":"response.completed","response":{"id":"response_1","status":"completed","output":[item],"usage":{"input_tokens":3,"output_tokens":1}}}),
    ];
    frames.into_iter().map(|frame|format!("event: {}\ndata: {frame}\n\n",frame["type"].as_str().unwrap())).collect()
}
async fn mount_success(server: &MockServer, endpoint: &str, responses: bool, count: u64) {
    Mock::given(method("POST")).and(path(endpoint)).and(header("authorization",format!("Bearer {SECRET}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(if responses {responses_success()} else {chat_success()},"text/event-stream"))
        .expect(count).mount(server).await;
}

#[tokio::test]
async fn malformed_responses_stream_selects_and_returns_the_chat_capability_actually_tested() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: {broken-json}\n\n","text/event-stream"))
        .expect(1).mount(&server).await;
    mount_success(&server,"/v1/chat/completions",false,1).await;
    let (router,_) = setup().await;
    let mut draft = anonymous(&server,"openai.responses");
    draft["capability"]["endpoint"] = json!("/responses?tenant=one");
    draft["capability"]["provider_params"] = json!({"chain_rounds":false,"include":null,"temperature":0.2,"vendor":{"keep":true}});
    let (status,body) = post(&router,"/api/providers/detect-chat-protocol",draft).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    let data=&body["data"];
    assert_eq!(data["selected_protocol"],"openai.chat_text");
    assert_eq!(data["attempts"][0]["status"],"incompatible");
    assert_eq!(data["attempts"][1]["status"],"verified");
    assert_eq!(data["selected_capability"]["endpoint"],"/chat/completions?tenant=one");
    assert_eq!(data["selected_capability"]["output_limit"],16000);
    assert_eq!(data["selected_capability"]["context_limit"],100000);
    assert_eq!(data["selected_capability"]["provider_params"],json!({"temperature":0.2,"vendor":{"keep":true}}));
    let received=server.received_requests().await.unwrap();
    assert_eq!(received.len(),2);
    let tested: Value=serde_json::from_slice(&received[1].body).unwrap();
    assert_eq!(tested["model"],"exact-user-model");
    assert_eq!(tested["max_tokens"],4096);
    assert_eq!(tested["vendor"],data["selected_capability"]["provider_params"]["vendor"]);
    assert_eq!(data["attempts"][1]["probe_output_limit"],4096);
}

#[tokio::test]
async fn an_explicit_missing_responses_endpoint_can_select_the_working_chat_route() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"error":{"code":"endpoint_not_found"}})))
        .expect(1).mount(&server).await;
    mount_success(&server,"/v1/chat/completions",false,1).await;
    let (router,_) = setup().await;
    let (status,body) = post(&router,"/api/providers/detect-chat-protocol",anonymous(&server,"openai.responses")).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    assert_eq!(body["data"]["selected_protocol"],"openai.chat_text");
    assert_eq!(body["data"]["attempts"][0]["status"],"incompatible");
    assert_eq!(body["data"]["attempts"][1]["status"],"verified");
}

#[tokio::test]
async fn equivalent_output_and_thinking_policies_are_preserved_in_the_tested_candidate() {
    let schema=json!({"name":"result","strict":true,"schema":{"type":"object",
        "properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}});
    let scenarios = [
        ("openai.responses", "/v1/responses", "/v1/chat/completions", "openai.chat_text", false,
            json!({"text":{"format":{"type":"json_schema","name":"result","strict":true,
                "schema":schema["schema"]},"verbosity":"low"}}),
            json!({"response_format":{"type":"json_schema","json_schema":schema},"verbosity":"low"})),
        ("openai.chat_text", "/v1/chat/completions", "/v1/responses", "openai.responses", true,
            json!({"thinking":{"type":"disabled"},"response_format":{"type":"json_object"},
                "verbosity":"high","stream_options":{"include_usage":false}}),
            json!({"thinking":{"type":"disabled"},"text":{"format":{"type":"json_object"},"verbosity":"high"}})),
    ];
    for (protocol,endpoint,other,selected,responses,params,expected) in scenarios {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path(endpoint))
            .respond_with(ResponseTemplate::new(405).set_body_json(json!({"error":{"code":"unsupported_endpoint"}})))
            .expect(1).mount(&server).await;
        mount_success(&server,other,responses,1).await;
        let (router,_)=setup().await;
        let mut draft=anonymous(&server,protocol);
        draft["capability"]["provider_params"]=params;
        let (status,body)=post(&router,"/api/providers/detect-chat-protocol",draft).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["selected_protocol"],selected,"{body}");
        assert_eq!(body["data"]["selected_capability"]["provider_params"],expected);
        assert_eq!(body["data"]["selected_capability"]["output_limit"],16000);
        let received=server.received_requests().await.unwrap();
        assert_eq!(received.len(),2);
        let tested: Value=serde_json::from_slice(&received[1].body).unwrap();
        for (key,value) in expected.as_object().unwrap() { assert_eq!(&tested[key],value,"{key}"); }
        if responses {
            assert!(tested.get("response_format").is_none());
            assert!(tested.get("stream_options").is_none());
        } else { assert!(tested.get("text").is_none()); }
    }
}

#[tokio::test]
async fn conflicting_or_unmapped_policies_stop_before_a_paid_candidate_without_losing_values() {
    let scenarios = [
        ("openai.responses", "/v1/responses", "/v1/chat/completions", "text",
            json!({"text":{"future_policy":"keep"}})),
        ("openai.chat_text", "/v1/chat/completions", "/v1/responses", "response_format",
            json!({"response_format":{"type":"json_object"},"text":{"format":{"type":"text"}}})),
    ];
    for (protocol,endpoint,other,key,params) in scenarios {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path(endpoint))
            .respond_with(ResponseTemplate::new(405).set_body_json(json!({"error":{"code":"unsupported_endpoint"}})))
            .expect(1).mount(&server).await;
        mount_success(&server,other,protocol == "openai.chat_text",0).await;
        let (router,_)=setup().await;
        let mut draft=anonymous(&server,protocol);
        draft["capability"]["provider_params"]=params;
        let original=draft.clone();
        let (status,body)=post(&router,"/api/providers/detect-chat-protocol",draft).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["attempts"][0]["status"],"incompatible");
        assert_eq!(body["data"]["attempts"][1]["status"],"failed");
        assert_eq!(body["data"]["attempts"][1]["error_kind"],"invalid_request");
        assert!(body["data"]["attempts"][1]["message"].as_str().unwrap().contains(key));
        assert!(body["data"].get("selected_protocol").is_none());
        assert!(body["data"].get("selected_capability").is_none());
        let received=server.received_requests().await.unwrap();
        assert_eq!(received.len(),1,"an unsafe conversion must not issue the other candidate");
        let tested: Value=serde_json::from_slice(&received[0].body).unwrap();
        assert_eq!(tested[key],original["capability"]["provider_params"][key]);
        assert_eq!(original["capability"]["output_limit"],16000);
    }
}

#[tokio::test]
async fn same_protocol_nested_reasoning_effort_is_normalized_before_testing_and_returning_the_draft() {
    let server=MockServer::start().await;
    mount_success(&server,"/v1/responses",true,1).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"openai.responses");
    draft["capability"]["provider_params"]=json!({"reasoning":{"effort":"high","summary":"auto"}});
    let (status,body)=post(&router,"/api/providers/detect-chat-protocol",draft).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    assert_eq!(body["data"]["selected_capability"]["provider_params"],
        json!({"reasoning_effort":"high","reasoning":{"summary":"auto"}}));
    assert_eq!(body["data"]["attempts"].as_array().unwrap().len(),1);
    let received=server.received_requests().await.unwrap();
    let tested: Value=serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(tested["reasoning"],json!({"effort":"high","summary":"auto"}));
}

#[tokio::test]
async fn structured_sse_protocol_rejections_can_select_the_other_protocol_without_losing_machine_evidence() {
    let errors=[
        ("openai.responses", "/v1/responses", "/v1/chat/completions", "openai.chat_text", false,
            vec![json!({"type":"error","code":"unsupported_protocol","message":"Use Chat Completions"})]),
        ("openai.responses", "/v1/responses", "/v1/chat/completions", "openai.chat_text", false,
            vec![json!({"type":"response.created","response":{"id":"r","status":"in_progress","output":[]}}),
            json!({"type":"response.failed","response":{"id":"r","status":"failed","output":[],
                "error":{"code":"unsupported_protocol","message":"Use Chat Completions"}}})]),
        ("openai.chat_text", "/v1/chat/completions", "/v1/responses", "openai.responses", true,
            vec![json!({"error":{"type":"invalid_request_error","code":"unsupported_protocol","message":"Use Responses"}})]),
    ];
    for (protocol,endpoint,other,selected,responses,frames) in errors {
        let server=MockServer::start().await;
        let stream: String=frames.into_iter().map(|frame|format!("event: {}\ndata: {frame}\n\n",frame["type"].as_str().unwrap_or("error"))).collect();
        Mock::given(method("POST")).and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_raw(stream,"text/event-stream"))
            .expect(1).mount(&server).await;
        mount_success(&server,other,responses,1).await;
        let (router,_)=setup().await;
        let (status,body)=post(&router,"/api/providers/detect-chat-protocol",anonymous(&server,protocol)).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["selected_protocol"],selected,"{body}");
        assert_eq!(body["data"]["attempts"][0]["status"],"incompatible");
        assert_eq!(body["data"]["attempts"][1]["status"],"verified");
    }
}

#[tokio::test]
async fn working_current_protocol_is_preserved_without_trying_another_or_saving() {
    for (protocol,endpoint,responses) in [("openai.chat_text","/v1/chat/completions",false),("openai.responses","/v1/responses",true)] {
        let server=MockServer::start().await;
        mount_success(&server,endpoint,responses,1).await;
        let (router,db)=setup().await;
        let draft=anonymous(&server,protocol);
        let original=draft["capability"].clone();
        let (status,body)=post(&router,"/api/providers/detect-chat-protocol",draft).await;
        assert_eq!(status,StatusCode::OK,"{body}");
        assert_eq!(body["data"]["selected_protocol"],protocol);
        assert_eq!(body["data"]["attempts"].as_array().unwrap().len(),1);
        let selected=&body["data"]["selected_capability"];
        assert_eq!(selected["protocol"],original["protocol"]);
        assert_eq!(selected["output_limit"],original["output_limit"]);
        assert_eq!(selected["provider_params"],original["provider_params"]);
        assert_eq!(server.received_requests().await.unwrap().len(),1);
        assert!(SqliteProviderRepository::new(db.pool().clone()).list().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn authentication_quota_rate_limits_and_model_errors_never_fall_back() {
    for (status,code,kind) in [(401,"invalid_api_key","unauthorized"),(403,"forbidden","forbidden"),
        (400,"insufficient_quota","insufficient_quota"),(429,"rate_limit_exceeded","rate_limited"),
        (404,"model_not_found","model_unavailable"),(404,"","not_found")] {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(status).insert_header("retry-after","0").set_body_json(json!({"error":{"code":code,"message":SECRET}})))
            .expect(1).mount(&server).await;
        mount_success(&server,"/v1/chat/completions",false,0).await;
        let (router,_)=setup().await;
        let (http,body)=post(&router,"/api/providers/detect-chat-protocol",anonymous(&server,"openai.responses")).await;
        assert_eq!(http,StatusCode::OK,"{body}");
        assert!(body["data"].get("selected_protocol").is_none(),"{status}");
        assert_eq!(body["data"]["attempts"].as_array().unwrap().len(),1);
        assert_eq!(body["data"]["attempts"][0]["status"],"failed");
        assert_eq!(body["data"]["attempts"][0]["error_kind"],kind);
    }
}

#[tokio::test]
async fn exhausted_output_budget_is_inconclusive_and_does_not_select_chat() {
    let server=MockServer::start().await;
    let frames=[json!({"type":"response.created","response":{"id":"r","status":"in_progress","output":[]}}),
        json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","output":[],"incomplete_details":{"reason":"max_output_tokens"}}})];
    let stream: String=frames.into_iter().map(|frame|format!("event: {}\ndata: {frame}\n\n",frame["type"].as_str().unwrap())).collect();
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(stream,"text/event-stream")).expect(1).mount(&server).await;
    mount_success(&server,"/v1/chat/completions",false,0).await;
    let (router,_)=setup().await;
    let (_,body)=post(&router,"/api/providers/detect-chat-protocol",anonymous(&server,"openai.responses")).await;
    assert_eq!(body["data"]["attempts"][0]["status"],"failed","{body}");
    assert!(body["data"]["attempts"][0]["message"].as_str().unwrap().contains("inconclusive"));
    assert!(body["data"].get("selected_protocol").is_none());
}

#[tokio::test]
async fn html_bodies_cannot_prove_compatibility_when_the_provider_error_drops_the_http_status() {
    // The production NonApiResponse error does not retain its HTTP status.
    // Treat both an SPA and an HTML authentication rejection as inconclusive,
    // so a hidden 401 can never authorize trying another paid candidate.
    for status in [200,401] {
        let server=MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(status).set_body_raw("<!doctype html><html>gateway</html>","text/html"))
            .expect(1).mount(&server).await;
        mount_success(&server,"/v1/chat/completions",false,0).await;
        let (router,_)=setup().await;
        let (_,body)=post(&router,"/api/providers/detect-chat-protocol",anonymous(&server,"openai.responses")).await;
        assert_eq!(body["data"]["attempts"][0]["status"],"failed");
        assert_eq!(body["data"]["attempts"][0]["error_kind"],"non_api_response");
        assert_eq!(body["data"]["attempts"].as_array().unwrap().len(),1);
        assert!(body["data"].get("selected_capability").is_none());
    }
}

#[tokio::test]
async fn named_connection_is_resolved_from_the_draft_and_cross_origin_consent_is_enforced() {
    let server=MockServer::start().await;
    mount_success(&server,"/named/responses",true,1).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"openai.responses");
    draft["capability"]["connection_role"]=json!("chat");
    draft["connections"]=json!([{"role":"chat","base_url":format!("{}/named",server.uri()),"auth_scheme":"bearer","credentials":{"api_keys":[SECRET]}}]);
    let (status,body)=post(&router,"/api/providers/detect-chat-protocol",draft.clone()).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    assert_eq!(body["data"]["selected_capability"]["connection_role"],"chat");
    draft["capability"]["base_url_override"]=json!("https://different-origin.invalid/v1");
    let (status,_)=post(&router,"/api/providers/detect-chat-protocol",draft).await;
    assert_eq!(status,StatusCode::BAD_REQUEST);
    assert_eq!(server.received_requests().await.unwrap().len(),1);
}

#[tokio::test]
async fn arbitrary_custom_endpoint_is_not_replaced_when_the_first_protocol_fails() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/custom-call"))
        .respond_with(ResponseTemplate::new(405).set_body_json(json!({"error":{"code":"unsupported_endpoint"}})))
        .expect(1).mount(&server).await;
    let (router,_)=setup().await;
    let mut draft=anonymous(&server,"openai.responses");
    draft["capability"]["endpoint"]=json!("/custom-call");
    let (status,body)=post(&router,"/api/providers/detect-chat-protocol",draft).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    assert_eq!(body["data"]["attempts"].as_array().unwrap().len(),2);
    assert_eq!(body["data"]["attempts"][1]["status"],"failed");
    assert!(body["data"]["attempts"][1]["message"].as_str().unwrap().contains("custom endpoint"));
    assert!(body["data"].get("selected_capability").is_none());
    assert_eq!(server.received_requests().await.unwrap().len(),1);
}

async fn stored_provider(db: &nomifun_db::Database, base_url: &str) -> String {
    let encrypted=nomifun_common::encrypt_string(&json!({"api_keys":[SECRET]}).to_string(),&KEY).unwrap();
    let capabilities=[NewProviderModelCapability {task:"chat",traits:"[]",protocol:"openai.responses",connection_role:"default",provider_params:"{}",..Default::default()}];
    let model=NewProviderModel {model:"original-model",enabled:true,capabilities:&capabilities,..Default::default()};
    SqliteProviderRepository::new(db.pool().clone()).create(CreateProviderParams {provider_id:None,platform:"custom",name:"Stored",base_url,
        auth_scheme:"bearer",credentials_encrypted:&encrypted,enabled:true,bedrock_config:None,sort_order:None},&model,&[]).await.unwrap().0.provider_id
}

#[tokio::test]
async fn stored_named_credentials_are_used_without_echoing_or_persisting_the_tested_draft() {
    use nomifun_db::{IProviderConnectionRepository,IProviderModelRepository,SqliteProviderConnectionRepository,
        SqliteProviderModelRepository,UpsertProviderConnectionParams};
    let server=MockServer::start().await;
    let named_secret="named-connection-secret";
    Mock::given(method("POST")).and(path("/named/responses"))
        .and(header("authorization",format!("Bearer {named_secret}")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(responses_success(),"text/event-stream"))
        .expect(1).mount(&server).await;
    let (router,db)=setup().await;
    let provider_id=stored_provider(&db,"https://unused-default.invalid/v1").await;
    let providers=SqliteProviderRepository::new(db.pool().clone());
    let current=providers.find_by_id(&provider_id).await.unwrap().unwrap();
    let encrypted=nomifun_common::encrypt_string(&json!({"api_keys":[named_secret]}).to_string(),&KEY).unwrap();
    SqliteProviderConnectionRepository::new(db.pool().clone()).upsert(&provider_id,current.config_revision,
        &UpsertProviderConnectionParams {role:"chat",label:None,base_url:&format!("{}/named",server.uri()),
            auth_scheme:"bearer",credentials_encrypted:&encrypted,extra:"{}"}).await.unwrap();
    let before=providers.find_by_id(&provider_id).await.unwrap().unwrap();
    let mut draft=capability("openai.responses");
    draft["connection_role"]=json!("chat");
    let (status,body)=post(&router,&format!("/api/providers/{provider_id}/detect-chat-protocol"),
        json!({"model":"exact-user-model","capability":draft})).await;
    assert_eq!(status,StatusCode::OK,"{body}");
    assert_eq!(body["data"]["selected_protocol"],"openai.responses");
    assert_eq!(body["data"]["selected_capability"]["connection_role"],"chat");
    assert_eq!(body["data"]["config_revision"],before.config_revision);
    assert!(!body.to_string().contains(named_secret));
    let after=providers.find_by_id(&provider_id).await.unwrap().unwrap();
    assert_eq!(after.config_revision,before.config_revision);
    assert_eq!(after.credentials_encrypted,before.credentials_encrypted);
    let models=SqliteProviderModelRepository::new(db.pool().clone()).list_for_provider(&provider_id).await.unwrap();
    assert_eq!(models.len(),1);
    assert_eq!(models[0].model,"original-model");
}

#[tokio::test]
async fn a_stored_provider_revision_change_invalidates_the_detection_without_saving_a_model() {
    let server=MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(300)).set_body_raw(responses_success(),"text/event-stream"))
        .expect(1).mount(&server).await;
    let (router,db)=setup().await;
    let id=stored_provider(&db,&format!("{}/v1",server.uri())).await;
    let uri=format!("/api/providers/{id}/detect-chat-protocol");
    let task=tokio::spawn(async move {post(&router,&uri,json!({"model":"exact-user-model","capability":capability("openai.responses")})).await});
    tokio::time::timeout(Duration::from_secs(5),async {
        while server.received_requests().await.unwrap().is_empty() {tokio::time::sleep(Duration::from_millis(5)).await;}
    }).await.unwrap();
    nomifun_db::sqlx::query("UPDATE providers SET config_revision = config_revision + 1 WHERE provider_id = ?")
        .bind(&id).execute(db.pool()).await.unwrap();
    let (status,body)=task.await.unwrap();
    assert_eq!(status,StatusCode::CONFLICT,"{body}");
    assert!(!body.to_string().contains("selected_capability"));
}

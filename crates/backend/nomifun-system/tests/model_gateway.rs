//! Gateway contract, secret transport, transactional graph and conservative sync.
mod common;
use std::sync::Arc;
use nomifun_api_types::*;
use nomifun_db::{init_database_memory, IProviderRepository, IProviderConnectionRepository, SqliteProviderRepository, SqliteProviderConnectionRepository};
use nomifun_system::{SystemRouterState, VersionCheckService};
use serde_json::{json, Value};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path, header}};
use tower::ServiceExt;
use http_body_util::BodyExt;

const KEY: [u8; 32] = [0x73;32];
fn state(db: &nomifun_db::Database) -> SystemRouterState {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    common::build_system_state(db, KEY, client.clone(), VersionCheckService::new(client, "0.1.0".into()), std::env::temp_dir(), std::env::temp_dir(), false)
}
fn meta() -> Value { json!({"contract_version":"1.0","operator":{"name":"Independent Partner","homepage_url":"https://operator.example/","console_url":"https://operator.example/console","purchase_url":"https://operator.example/purchase","terms_url":null,"privacy_url":null},"capabilities":["openai-response","anthropic","gemini"],"optional_endpoints":[]}) }
fn model(id: &str, endpoint: &str, context: i64) -> Value { json!({"id":id,"display_name":id,"vendor":"fixture","tasks":["chat"],"task_endpoints":{"chat":{"endpoints":[endpoint],"preferred_endpoint":endpoint}},"context_window":context,"max_output_tokens":4096,"input_modalities":["text","image"],"traits":["vision_input"],"pricing":[{"task":"chat","meter":"input_tokens","unit_size":1000,"amount":9,"currency":"USD"}],"included_in_plan":true,"status":"available"}) }
fn catalog() -> Value { json!({"contract_version":"1.0","models":[model("gpt-fixture","openai-response",128000),model("claude-fixture","anthropic",200000),model("gemini-fixture","gemini",1000000)]}) }
fn account() -> Value { json!({"contract_version":"1.0","plan":{"name":"Fixture Plan","period_start":"2026-10-01T00:00:00Z","period_end":"2026-11-01T00:00:00Z","quota":{"unit":"tokens","total":null,"used":12}},"balance":{"amount":-5,"currency":"USD"},"key":{"name":"Fixture Key","expires_at":null,"quota_unit":"tokens","remaining_quota":null},"rate_limits":{"requests_per_minute":null,"tokens_per_minute":1000,"concurrent_requests":0}}) }
async fn mount(server: &MockServer, key: &str, value: Value) {
    Mock::given(method("GET")).and(path("/nomifun/v1/meta")).respond_with(ResponseTemplate::new(200).set_body_json(meta())).mount(server).await;
    Mock::given(method("GET")).and(path("/nomifun/v1/catalog")).and(header("authorization",format!("Bearer {key}"))).respond_with(ResponseTemplate::new(200).set_body_json(value)).mount(server).await;
    Mock::given(method("GET")).and(path("/nomifun/v1/account")).and(header("authorization",format!("Bearer {key}"))).respond_with(ResponseTemplate::new(200).set_body_json(account())).mount(server).await;
}
async fn create(service: &nomifun_system::ProviderService, server: &MockServer) -> ProviderResponse {
    service.create_gateway(CreateModelGatewayRequest { base_url: server.uri(), api_key:"fixture-key".into(),name:"My Community Gateway".into(),models:vec!["gpt-fixture".into(),"claude-fixture".into(),"gemini-fixture".into()] }).await.unwrap()
}

#[test]
fn strict_catalog_mapping_rejects_entire_invalid_directory() {
    let valid = nomifun_system::model_gateway::parse_catalog(catalog()).unwrap();
    assert_eq!(valid.imports[0].capabilities[0].protocol,"openai.responses");
    assert_eq!(valid.imports[1].capabilities[0].connection_role,"anthropic");
    assert_eq!(valid.imports[2].capabilities[0].connection_role,"gemini");
    assert_eq!(valid.imports[1].capabilities[0].output_limit,Some(4096));
    for bad in [
        json!(null), json!("unknown-endpoint"), json!("openai"),
    ] {
        let mut value = catalog(); value["models"][1]["task_endpoints"]["chat"]["preferred_endpoint"] = bad;
        assert!(nomifun_system::model_gateway::parse_catalog(value).is_err());
    }
    for field in ["max_output_tokens","context_window","pricing","tasks"] {
        let mut value = catalog(); value["models"][1].as_object_mut().unwrap().remove(field);
        assert!(nomifun_system::model_gateway::parse_catalog(value).is_err());
    }
    let mut value = catalog(); value["models"][1]["max_output_tokens"]=Value::Null;
    assert!(nomifun_system::model_gateway::parse_catalog(value).is_err());
    let mut value = catalog(); value["models"][0]["pricing"][0]["amount"]=json!(1.5);
    assert!(nomifun_system::model_gateway::parse_catalog(value).is_err());
    let mut value = catalog(); value["models"][0]["task_endpoints"]["rerank"]=json!({"endpoints":["jina-rerank"],"preferred_endpoint":"jina-rerank"});
    assert!(nomifun_system::model_gateway::parse_catalog(value).is_err());
}

#[test]
fn safe_gateway_roots_are_exact_and_never_strip_credentials() {
    use nomifun_system::model_gateway::normalize_gateway_root;
    assert_eq!(normalize_gateway_root("https://example.com/v1/").unwrap(),"https://example.com");
    assert_eq!(normalize_gateway_root("http://127.0.0.1:1234").unwrap(),"http://127.0.0.1:1234");
    for value in ["file:///tmp/key","https://user:secret@example.com","https://example.com?key=secret","https://example.com#key","https://example.com/custom/v1","https://example.com/v1/chat/completions"] { assert!(normalize_gateway_root(value).is_err(),"{value}"); }
}

#[test]
fn catalog_prices_accept_native_i64_and_token_limits_require_safe_renderer_integers() {
    let mut value=catalog();
    value["models"][0]["pricing"][0]["unit_size"]=json!(i64::MAX);
    value["models"][0]["pricing"][0]["amount"]=json!(i64::MAX);
    value["models"][0]["context_window"]=json!(9_007_199_254_740_991_i64);
    let parsed=nomifun_system::model_gateway::parse_catalog(value.clone()).unwrap();
    assert_eq!(parsed.models[0].pricing[0].amount,i64::MAX);
    assert_eq!(parsed.models[0].pricing[0].unit_size,i64::MAX);
    for field in ["context_window","max_output_tokens"] {
        let mut unsafe_limit=value.clone(); unsafe_limit["models"][0][field]=json!(9_007_199_254_740_992_i64);
        assert!(nomifun_system::model_gateway::parse_catalog(unsafe_limit).is_err());
    }
    for invalid in [json!("9223372036854775807"),json!(1.5),json!(i64::MAX as u64+1)] {
        for field in ["amount","unit_size"] {
            let mut invalid_price=value.clone(); invalid_price["models"][0]["pricing"][0][field]=invalid.clone();
            assert!(nomifun_system::model_gateway::parse_catalog(invalid_price).is_err());
        }
    }
}

#[tokio::test]
async fn meta_is_anonymous_and_account_preserves_unknown_quota() {
    let server = MockServer::start().await; mount(&server,"fixture-key",catalog()).await;
    let meta = nomifun_system::model_gateway::fetch_meta(&server.uri()).await.unwrap();
    assert_eq!(meta.operator.name,"Independent Partner");
    let account = nomifun_system::model_gateway::fetch_account(&server.uri(),"fixture-key").await.unwrap();
    assert_eq!(account.balance.amount,-5); assert_eq!(account.key.remaining_quota,None);
    assert_eq!(account.plan.unwrap().quota.total,None);
    let requests = server.received_requests().await.unwrap();
    assert!(!requests[0].headers.contains_key("authorization"));
    assert!(requests.iter().all(|r| r.url.query().is_none()));
}

#[tokio::test]
async fn control_plane_rejects_redirects_insecure_links_and_reflected_secrets() {
    let target = MockServer::start().await; let origin = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(302).insert_header("Location",target.uri())).mount(&origin).await;
    assert!(nomifun_system::model_gateway::fetch_catalog(&origin.uri(),"fixture-secret").await.is_err());
    assert!(target.received_requests().await.unwrap().is_empty());
    origin.reset().await;
    let mut invalid_meta = meta(); invalid_meta["operator"]["purchase_url"]=json!("http://operator.example/purchase");
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(invalid_meta)).mount(&origin).await;
    assert!(nomifun_system::model_gateway::fetch_meta(&origin.uri()).await.is_err());
    origin.reset().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(401).set_body_string("fixture-secret")).mount(&origin).await;
    let error = nomifun_system::model_gateway::fetch_catalog(&origin.uri(),"fixture-secret").await.unwrap_err();
    assert!(!error.to_string().contains("fixture-secret"));
}

#[tokio::test]
async fn graph_create_and_rotate_updates_all_roles_once_and_rejects_cross_origin_key_reuse() {
    let server = MockServer::start().await; mount(&server,"fixture-key",catalog()).await;
    let db = init_database_memory().await.unwrap(); let state = state(&db);
    let created = create(&state.provider_service,&server).await;
    assert_eq!(created.models.len(),3); assert!(!serde_json::to_string(&created).unwrap().contains("fixture-key"));
    assert_eq!(created.base_url,format!("{}/v1",server.uri()));
    let repository = SqliteProviderRepository::new(db.pool().clone());
    let before = repository.find_by_id(&created.provider_id).await.unwrap().unwrap();
    let profile_repo = SqliteProviderConnectionRepository::new(db.pool().clone());
    let profiles = profile_repo.list_for_provider(&created.provider_id).await.unwrap();
    assert_eq!(profiles.len(),2); assert!(profiles.iter().all(|profile| profile.base_url==server.uri()));
    // No-op edits retain ciphertext, revision and health observations.
    state.provider_service.update_gateway_connection(&created.provider_id,UpdateModelGatewayConnectionRequest {base_url:server.uri(),api_key:None,name:Some("Renamed".into())}).await.unwrap();
    let renamed = repository.find_by_id(&created.provider_id).await.unwrap().unwrap();
    assert_eq!(before.config_revision,renamed.config_revision);
    server.reset().await; mount(&server,"rotated-key",catalog()).await;
    state.provider_service.update_gateway_connection(&created.provider_id,UpdateModelGatewayConnectionRequest {base_url:server.uri(),api_key:Some("rotated-key".into()),name:None}).await.unwrap();
    let rotated = repository.find_by_id(&created.provider_id).await.unwrap().unwrap();
    assert_eq!(rotated.config_revision,renamed.config_revision+1);
    assert_eq!(state.provider_service.api_keys(&created.provider_id).await.unwrap(),vec!["rotated-key"]);
    for profile in profile_repo.list_for_provider(&created.provider_id).await.unwrap() {
        let plaintext = nomifun_common::decrypt_string(&profile.credentials_encrypted,&KEY).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&plaintext).unwrap(),json!({"api_keys":["rotated-key"]}));
    }
    let other = MockServer::start().await;
    assert!(state.provider_service.update_gateway_connection(&created.provider_id,UpdateModelGatewayConnectionRequest {base_url:other.uri(),api_key:None,name:None}).await.is_err());
    assert!(other.received_requests().await.unwrap().is_empty());
    assert!(state.provider_service.update(&created.provider_id,UpdateProviderRequest {base_url:Some(other.uri()),..Default::default()}).await.is_err());
    assert!(state.provider_connection_service.delete(&created.provider_id,"anthropic").await.is_err());
}

#[tokio::test]
async fn directory_sync_adds_updates_unmodified_metadata_and_preserves_overrides_and_missing_rows() {
    let server = MockServer::start().await; mount(&server,"fixture-key",catalog()).await;
    let db = init_database_memory().await.unwrap(); let state = state(&db);
    let created = create(&state.provider_service,&server).await;
    let mut customized: ProviderModelInput = serde_json::from_value(json!({"model":"gpt-fixture","display_name":"My GPT","enabled":false,"sort_order":12,"description":"User notes","capabilities":created.models[0].capabilities.iter().map(|cap| json!({"task":cap.task,"traits":cap.traits,"protocol":cap.protocol,"connection_role":cap.connection_role,"provider_params":cap.provider_params,"context_limit":cap.context_limit,"output_limit":cap.output_limit})).collect::<Vec<_>>()})).unwrap();
    customized.capabilities[0].context_limit=Some(7777);
    customized.capabilities[0].protocol="openai.chat_text".into();
    state.provider_model_service.save(SaveProviderModelRequest {provider_id:created.provider_id.clone(),model:customized}).await.unwrap();
    let changed = json!({"contract_version":"1.0","models":[model("gpt-fixture","anthropic",256000),model("claude-fixture","anthropic",300000),model("new-fixture","openai",64000)]});
    server.reset().await; mount(&server,"fixture-key",changed).await;
    let result = state.provider_service.sync_gateway_catalog(&created.provider_id).await.unwrap();
    assert_eq!(result.added,1); assert_eq!(result.models.len(),4);
    let user = result.models.iter().find(|m|m.model=="gpt-fixture").unwrap();
    assert_eq!(user.display_name.as_deref(),Some("My GPT")); assert_eq!(user.description.as_deref(),Some("User notes"));
    assert!(!user.enabled); assert_eq!(user.sort_order,12); assert_eq!(user.capabilities[0].context_limit,Some(7777));
    assert_eq!(user.capabilities[0].protocol,"openai.chat_text"); assert_eq!(user.capabilities[0].connection_role,"default");
    assert_eq!(result.models.iter().find(|m|m.model=="claude-fixture").unwrap().capabilities[0].context_limit,Some(300000));
    assert!(result.models.iter().any(|m|m.model=="gemini-fixture"));
    let again = state.provider_service.sync_gateway_catalog(&created.provider_id).await.unwrap();
    assert_eq!(again.added,0); assert_eq!(again.updated,0);
}

#[tokio::test]
async fn invalid_catalog_never_creates_half_provider_or_updates_existing_graph() {
    let server=MockServer::start().await;
    let mut invalid_catalog=catalog(); invalid_catalog["models"][1]["max_output_tokens"]=Value::Null;
    mount(&server,"fixture-key",invalid_catalog).await;
    let db=init_database_memory().await.unwrap(); let state=state(&db);
    assert!(state.provider_service.create_gateway(CreateModelGatewayRequest{base_url:server.uri(),api_key:"fixture-key".into(),name:"Test".into(),models:vec!["gpt-fixture".into()]}).await.is_err());
    assert!(state.provider_service.list().await.unwrap().is_empty());
    server.reset().await; mount(&server,"fixture-key",catalog()).await;
    let provider=create(&state.provider_service,&server).await;
    let repository=Arc::new(SqliteProviderRepository::new(db.pool().clone()));
    let before=repository.find_by_id(&provider.provider_id).await.unwrap().unwrap();
    server.reset().await; let mut bad=catalog(); bad["models"][0]["task_endpoints"]["chat"].as_object_mut().unwrap().remove("preferred_endpoint"); mount(&server,"new-key",bad).await;
    assert!(state.provider_service.update_gateway_connection(&provider.provider_id,UpdateModelGatewayConnectionRequest{base_url:server.uri(),api_key:Some("new-key".into()),name:Some("Bad".into())}).await.is_err());
    let after=repository.find_by_id(&provider.provider_id).await.unwrap().unwrap();
    assert_eq!(before.config_revision,after.config_revision);assert_eq!(before.credentials_encrypted,after.credentials_encrypted);assert_eq!(before.name,after.name);
}

#[tokio::test]
async fn literal_meta_catalog_create_routes_and_saved_account_are_reachable() {
    let server = MockServer::start().await; mount(&server,"fixture-key",catalog()).await;
    let db = init_database_memory().await.unwrap();
    let router = nomifun_system::system_routes(state(&db));
    for (url, body) in [
        ("/api/providers/model-gateway/meta",json!({"base_url":server.uri()})),
        ("/api/providers/model-gateway/catalog",json!({"base_url":server.uri(),"api_key":"fixture-key"})),
    ] {
        let request = axum::http::Request::builder().method("POST").uri(url).header("content-type","application/json").body(axum::body::Body::from(body.to_string())).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(),axum::http::StatusCode::OK);
        let value:Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(value["data"]["contract_version"],"1.0");
    }
    let body=json!({"base_url":server.uri(),"api_key":"fixture-key","name":"Partner","models":["gpt-fixture","claude-fixture"]});
    let response=router.clone().oneshot(axum::http::Request::builder().method("POST").uri("/api/providers/model-gateway/create").header("content-type","application/json").body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),axum::http::StatusCode::CREATED);
    let value:Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["data"]["models"].as_array().unwrap().len(),2);
    let id=value["data"]["provider_id"].as_str().unwrap();
    let response=router.oneshot(axum::http::Request::builder().uri(format!("/api/providers/{id}/model-gateway/account")).body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),axum::http::StatusCode::OK);
    let value:Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(value["data"]["key"]["remaining_quota"].is_null());
}

#[tokio::test]
async fn desktop_http_preserves_native_int64_extremes_as_decimal_strings() {
    let server=MockServer::start().await; mount(&server,"fixture-key",catalog()).await;
    let db=init_database_memory().await.unwrap(); let state=state(&db);
    let provider=create(&state.provider_service,&server).await;
    let router=nomifun_system::system_routes(state);
    for amount in [i64::MIN,i64::MAX] {
        server.reset().await;
        // Native wire fixtures remain JSON integers, independently authored;
        // response DTO serialization intentionally represents desktop strings.
        let mut native=account(); native["balance"]["amount"]=json!(amount);
        native["plan"]["quota"]["total"]=json!(i64::MAX); native["plan"]["quota"]["used"]=json!(i64::MAX);
        native["key"]["remaining_quota"]=json!(i64::MAX); native["rate_limits"]["requests_per_minute"]=json!(i64::MAX);
        native["rate_limits"]["tokens_per_minute"]=Value::Null;
        Mock::given(method("GET")).and(path("/nomifun/v1/account")).and(header("authorization","Bearer fixture-key")).respond_with(ResponseTemplate::new(200).set_body_json(native)).mount(&server).await;
        let response=router.clone().oneshot(axum::http::Request::builder().uri(format!("/api/providers/{}/model-gateway/account",provider.provider_id)).body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),axum::http::StatusCode::OK);
        let body=response.into_body().collect().await.unwrap().to_bytes();
        let value:Value=serde_json::from_slice(&body).unwrap();
        assert_eq!(value["data"]["balance"]["amount"].as_str(),Some(amount.to_string().as_str()));
        for field in ["total","used"] { assert_eq!(value["data"]["plan"]["quota"][field].as_str(),Some(i64::MAX.to_string().as_str())); }
        assert_eq!(value["data"]["key"]["remaining_quota"].as_str(),Some(i64::MAX.to_string().as_str()));
        assert_eq!(value["data"]["rate_limits"]["requests_per_minute"].as_str(),Some(i64::MAX.to_string().as_str()));
        assert_eq!(value["data"]["rate_limits"]["concurrent_requests"],"0");
        assert!(value["data"]["rate_limits"]["tokens_per_minute"].is_null());
    }
    server.reset().await;
    let mut native_catalog=catalog(); native_catalog["models"][0]["pricing"][0]["unit_size"]=json!(i64::MAX); native_catalog["models"][0]["pricing"][0]["amount"]=json!(i64::MAX);
    mount(&server,"fixture-key",native_catalog).await;
    let body=json!({"base_url":server.uri(),"api_key":"fixture-key"});
    let response=router.oneshot(axum::http::Request::builder().method("POST").uri("/api/providers/model-gateway/catalog").header("content-type","application/json").body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),axum::http::StatusCode::OK);
    let value:Value=serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    for field in ["amount","unit_size"] { assert_eq!(value["data"]["models"][0]["pricing"][0][field].as_str(),Some(i64::MAX.to_string().as_str())); }
}

#[tokio::test]
async fn account_rejects_native_strings_fractions_and_out_of_range_values() {
    let server=MockServer::start().await;
    for invalid in [json!("9223372036854775807"),json!(1.5),json!(i64::MAX as u64+1)] {
        for (group,field) in [("balance","amount"),("key","remaining_quota"),("rate_limits","requests_per_minute")] {
            server.reset().await;
            let mut native=account(); native[group][field]=invalid.clone();
            Mock::given(method("GET")).and(path("/nomifun/v1/account")).respond_with(ResponseTemplate::new(200).set_body_json(native)).mount(&server).await;
            assert!(nomifun_system::model_gateway::fetch_account(&server.uri(),"fixture-key").await.is_err());
        }
    }
    for invalid in [json!("9223372036854775807"),json!(1.5),json!(i64::MAX as u64+1)] {
        for field in ["total","used"] {
            server.reset().await; let mut native=account(); native["plan"]["quota"][field]=invalid.clone();
            Mock::given(method("GET")).and(path("/nomifun/v1/account")).respond_with(ResponseTemplate::new(200).set_body_json(native)).mount(&server).await;
            assert!(nomifun_system::model_gateway::fetch_account(&server.uri(),"fixture-key").await.is_err());
        }
    }
}

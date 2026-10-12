//! Independent synthetic Gateway import -> exact task selection -> HTTP execution.
//! Chat uses the shared single-attempt transport here; native broker semantics
//! are covered by the application broker tests.
mod common;

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use nomifun_api_types::{CreateModelGatewayRequest, ModelTask};
use nomifun_db::{
    Database, SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
};
use nomifun_model_invoke::{
    AdapterRegistry, AuthMaterial, EmbedRequest, ImageEditRequest, ImageGenRequest,
    InputAsset, InvokeError, InvokeErrorKind, MaterializeLimits, ModelInvokeService,
    ModelRef, OpaqueCredentialLease, OpaqueCredentialResolver, RerankRequest,
    SingleAttemptFraming, SingleAttemptHttpExecutor, SingleAttemptRequest,
    TaskOutcome, TaskRequest, TaskResult, default_adapters,
};
use nomifun_system::{SystemRouterState, VersionCheckService};
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{body_partial_json, header, method, path}};

const KEY: [u8; 32] = [0x75; 32];
const MODEL: &str = "opaque-multi-task";
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a8l8AAAAASUVORK5CYII=";
const TASKS: [(ModelTask, &str, &str, &str); 5] = [
    (ModelTask::Chat, "chat", "openai", "openai.chat_text"),
    (ModelTask::ImageGeneration, "image_generation", "image-generation", "openai.images"),
    (ModelTask::ImageEdit, "image_edit", "image-generation", "openai.images"),
    (ModelTask::Embedding, "embedding", "embeddings", "openai.embeddings"),
    (ModelTask::Rerank, "rerank", "jina-rerank", "generic.rerank"),
];

fn state(db: &Database) -> SystemRouterState {
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    common::build_system_state(db, KEY, http.clone(),
        VersionCheckService::new(http, "0.1.0".into()),
        std::env::temp_dir(), std::env::temp_dir(), false)
}

fn invoke_service(db: &Database) -> ModelInvokeService {
    ModelInvokeService::new(
        Arc::new(SqliteProviderRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderConnectionRepository::new(db.pool().clone())),
        KEY, reqwest::Client::builder().no_proxy().build().unwrap(),
        AdapterRegistry::new(default_adapters()),
    )
}

async fn mount_catalog(server: &MockServer, tasks: &[(ModelTask, &str, &str, &str)]) {
    let endpoints = tasks.iter().map(|(_, task, endpoint, _)|
        ((*task).to_owned(), json!({"endpoints":[endpoint],"preferred_endpoint":endpoint})))
        .collect::<serde_json::Map<_, _>>();
    let meta = json!({
        "contract_version":"1.0",
        "operator":{"name":"Independent Fixture Operator","homepage_url":"https://operator.example/",
            "console_url":"https://operator.example/console","purchase_url":"https://operator.example/purchase",
            "terms_url":null,"privacy_url":null},
        "capabilities":["openai","image-generation","embeddings","jina-rerank"],"optional_endpoints":[]
    });
    let catalog = json!({"contract_version":"1.0","models":[{
        "id":MODEL,"display_name":"Opaque fixture","vendor":"fixture",
        "tasks":tasks.iter().map(|(_, task, _, _)| *task).collect::<Vec<_>>(),
        "task_endpoints":endpoints,"context_window":32768,"max_output_tokens":4096,
        // Missing descriptive labels must not disable typed image-edit inputs.
        "input_modalities":["text","image"],"traits":[],"pricing":[],
        "included_in_plan":true,"status":"available"
    }]});
    Mock::given(method("GET")).and(path("/nomifun/v1/meta"))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta)).mount(server).await;
    Mock::given(method("GET")).and(path("/nomifun/v1/catalog"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(catalog)).mount(server).await;
}

async fn import(state: &SystemRouterState, server: &MockServer) -> ModelRef {
    let provider = state.provider_service.create_gateway(CreateModelGatewayRequest {
        base_url: server.uri(), api_key: "fixture-key".into(),
        name: "Community Fixture Gateway".into(), models: vec![MODEL.into()],
    }).await.unwrap();
    ModelRef { provider_id: provider.provider_id, model: MODEL.into() }
}

struct FixtureCredentialAuthority(AuthMaterial);

#[async_trait::async_trait]
impl OpaqueCredentialResolver for FixtureCredentialAuthority {
    async fn resolve(&self, lease: &OpaqueCredentialLease) -> Result<AuthMaterial, InvokeError> {
        assert_eq!(lease.handle(), "synthetic-chat-lease");
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn imported_five_task_model_selects_exact_provider_and_executes_native_http_shapes() {
    let database = init_database_memory().await.unwrap();
    let state = state(&database);
    let other = MockServer::start().await;
    let selected = MockServer::start().await;
    // Two providers publish the same opaque model id, with different tasks.
    mount_catalog(&other, &TASKS[..1]).await;
    mount_catalog(&selected, &TASKS).await;
    let other_model = import(&state, &other).await;
    let chosen = import(&state, &selected).await;
    assert_ne!(chosen.provider_id, other_model.provider_id);
    let invoke = invoke_service(&database);

    for (task, _, _, protocol) in TASKS {
        let candidates = invoke.available_task_models(task).await.unwrap();
        assert!(candidates.iter().any(|model| model.provider_id == chosen.provider_id && model.model == MODEL));
        assert_eq!(candidates.len(), if task == ModelTask::Chat { 2 } else { 1 });
        let config = invoke.resolve_task_config(&chosen, task).await.unwrap();
        assert_eq!(config.provider_id, chosen.provider_id);
        assert_eq!(config.model, MODEL);
        assert_eq!(config.protocol, protocol);
        assert_eq!(config.connection.role, "default");
        assert_eq!(config.connection.base_url, format!("{}/v1", selected.uri()));
        assert_eq!(config.context_limit, Some(32768));
        assert_eq!(config.output_limit, Some(4096));
    }

    let error = invoke.invoke(&other_model, TaskRequest::Embedding(EmbedRequest {
        inputs: vec!["must not fall back".into()], extra: json!({}),
    })).await.unwrap_err();
    assert_eq!(error.kind, InvokeErrorKind::UnsupportedTask);

    Mock::given(method("POST")).and(path("/v1/images/generations"))
        .and(header("authorization", "Bearer fixture-key"))
        .and(body_partial_json(json!({"model":MODEL,"prompt":"synthetic generation","n":1})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"b64_json":PNG,"mime_type":"image/png"}]})))
        .expect(1).mount(&selected).await;
    Mock::given(method("POST")).and(path("/v1/images/edits"))
        .and(header("authorization", "Bearer fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"b64_json":PNG,"mime_type":"image/png"}]})))
        .expect(1).mount(&selected).await;
    Mock::given(method("POST")).and(path("/v1/embeddings"))
        .and(header("authorization", "Bearer fixture-key"))
        .and(body_partial_json(json!({"model":MODEL,"input":["alpha","beta"]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
            {"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]})))
        .expect(1).mount(&selected).await;
    Mock::given(method("POST")).and(path("/v1/rerank"))
        .and(header("authorization", "Bearer fixture-key"))
        .and(body_partial_json(json!({"model":MODEL,"query":"beta","documents":["alpha","beta"],"top_n":2})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":[
            {"index":1,"relevance_score":0.9,"document":{"text":"beta"}},
            {"index":0,"relevance_score":0.1,"document":{"text":"alpha"}}]})))
        .expect(1).mount(&selected).await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer fixture-key"))
        .and(body_partial_json(json!({"model":MODEL,"stream":true})))
        .respond_with(ResponseTemplate::new(200).insert_header("Content-Type", "text/event-stream")
            .set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"fixture answer\"}}]}\n\ndata: [DONE]\n\n"))
        .expect(1).mount(&selected).await;

    for request in [
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt:"synthetic generation".into(), count:1, size:None, quality:None,
            extra:json!({"model":"must-not-replace-selected-model"}),
        }),
        TaskRequest::ImageEdit(ImageEditRequest {
            prompt:"synthetic edit".into(), count:1, size:Some("1024x1024".into()), quality:Some("high".into()),
            inputs: vec![
                InputAsset { id:None, role:"reference".into(), bytes:b"first-reference".to_vec(), mime:"image/png".into() },
                InputAsset { id:None, role:"reference".into(), bytes:b"second-reference".to_vec(), mime:"image/jpeg".into() },
                InputAsset { id:None, role:"mask".into(), bytes:b"synthetic-mask".to_vec(), mime:"image/png".into() },
            ], extra:json!({}),
        }),
    ] {
        let (outcome, context) = invoke.invoke_with_context(&chosen, request).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else { panic!("expected image assets") };
        let materialized = invoke.materialize_assets_for_invocation(&context, assets, MaterializeLimits::default()).await.unwrap();
        assert_eq!(materialized.len(), 1);
        assert!(materialized[0].bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(materialized[0].mime.as_deref(), Some("image/png"));
    }
    let TaskOutcome::Done(TaskResult::Embeddings(vectors)) = invoke.invoke(&chosen,
        TaskRequest::Embedding(EmbedRequest { inputs:vec!["alpha".into(),"beta".into()], extra:json!({}) }))
        .await.unwrap() else { panic!("expected embedding vectors") };
    assert_eq!(vectors, vec![vec![1.0,0.0],vec![0.0,1.0]]);
    let documents = vec!["alpha".to_owned(),"beta".to_owned()];
    let TaskOutcome::Done(TaskResult::Reranked(results)) = invoke.invoke(&chosen,
        TaskRequest::Rerank(RerankRequest { query:"beta".into(), documents:documents.clone(), top_n:Some(2), extra:json!({}) }))
        .await.unwrap() else { panic!("expected rerank results") };
    assert_eq!(results.iter().map(|result| documents[result.index].as_str()).collect::<Vec<_>>(), vec!["beta","alpha"]);
    assert_eq!(results[0].document.as_deref(), Some("beta"));

    let chat = invoke.resolve_task_config(&chosen, ModelTask::Chat).await.unwrap();
    let transport = SingleAttemptHttpExecutor::new(reqwest::Client::builder().no_proxy().build().unwrap(),
        Arc::new(FixtureCredentialAuthority(chat.connection.auth.clone())));
    let mut stream = transport.open_stream(SingleAttemptRequest {
        protocol:chat.protocol.clone(), url:chat.http_endpoint().unwrap(), model:chat.model.clone(),
        body:json!({"model":chat.model,"stream":true,"messages":[{"role":"user","content":"synthetic chat"}]}),
        credential:OpaqueCredentialLease::new("synthetic-chat-lease").unwrap(),
        timeout:Duration::from_secs(5), idle_timeout:Duration::from_secs(5),
        framing:SingleAttemptFraming::Sse, region:None,
    }).await.unwrap();
    let frame = stream.next().await.unwrap().unwrap();
    assert_eq!(frame.data["choices"][0]["delta"]["content"], "fixture answer");
    while let Some(frame) = stream.next().await { frame.unwrap(); }

    let requests = selected.received_requests().await.unwrap();
    let edit = requests.iter().find(|request| request.url.path() == "/v1/images/edits").unwrap();
    assert!(edit.headers["content-type"].to_str().unwrap().starts_with("multipart/form-data; boundary="));
    let body = String::from_utf8_lossy(&edit.body);
    for part in ["name=\"image[]\"", "name=\"mask\"", "name=\"model\"", MODEL,
        "first-reference", "second-reference", "synthetic-mask", "synthetic edit"] {
        assert!(body.contains(part), "missing multipart part {part}");
    }
    assert!(body.find("first-reference").unwrap() < body.find("second-reference").unwrap());
    assert_eq!(requests.iter().filter(|request| request.method.as_str() == "POST").count(), 5);
    for request in requests.iter().filter(|request| request.method.as_str() == "POST") {
        assert!(!String::from_utf8_lossy(&request.body)
            .contains(nomifun_api_types::MODEL_GATEWAY_CATALOG_BASELINE_PARAM));
    }
    assert!(other.received_requests().await.unwrap().iter().all(|request| request.method.as_str() == "GET"));
}

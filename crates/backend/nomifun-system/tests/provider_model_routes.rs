//! Black-box tests for the single provider-model full-save surface.

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use nomifun_api_types::{ModelTask, ModelTrait};
use nomifun_db::{
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
};
use nomifun_model_invoke::{
    AdapterRegistry, ModelInvokeService, ModelRef, default_adapters,
};
use nomifun_system::{SystemRouterState, VersionCheckService, system_routes};

const TEST_KEY: [u8; 32] = [0x42; 32];

fn build_state(db: &nomifun_db::Database) -> SystemRouterState {
    let http = reqwest::Client::new();
    common::build_system_state(
        db,
        TEST_KEY,
        http.clone(),
        VersionCheckService::new(http, "0.1.0".into()),
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    )
}

fn build_invoke(db: &nomifun_db::Database) -> ModelInvokeService {
    ModelInvokeService::new(
        Arc::new(SqliteProviderRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(
            db.pool().clone(),
        )),
        Arc::new(SqliteProviderConnectionRepository::new(db.pool().clone())),
        TEST_KEY,
        reqwest::Client::new(),
        AdapterRegistry::new(default_adapters()),
    )
}

fn request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder().method(method).uri(uri);
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn chat_capability() -> Value {
    json!({
        "task": "chat",
        "traits": [],
        "protocol": "openai.chat_text",
        "connection_role": "default",
        "provider_params": {}
    })
}

async fn create_provider(db: &nomifun_db::Database, platform: &str, name: &str) -> String {
    let response = system_routes(build_state(db))
        .oneshot(request(
            "POST",
            "/api/providers",
            Some(json!({
                "platform": platform,
                "name": name,
                "base_url": "https://api.example.test/v1",
                "auth_scheme": "bearer",
                "credentials": {"api_keys": ["sk-test"]},
                "initial_model": {
                    "model": "seed-chat",
                    "capabilities": [chat_capability()]
                },
                "connections": []
            })),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    body_json(response).await["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn seed_saved_model(
    db: &nomifun_db::Database,
    provider_id: &str,
    model: &str,
    task: &str,
    protocol: &str,
) {
    let provider = SqliteProviderRepository::new(db.pool().clone())
        .find_by_id(provider_id).await.unwrap().unwrap();
    let capabilities = [NewProviderModelCapability {
        task, traits: "[]", protocol, connection_role: "default", provider_params: "{}",
        ..Default::default()
    }];
    // Bypass current save validation to reproduce configuration from a release
    // before the provider announced this model's retirement.
    SqliteProviderModelRepository::new(db.pool().clone()).save(
        provider_id, provider.config_revision, &NewProviderModel {
            model, enabled: true, capabilities: &capabilities, ..Default::default()
        },
    ).await.unwrap();
}

#[tokio::test]
async fn retired_agnes_video_is_hidden_from_every_current_model_list_without_deleting_saved_configuration() {
    let db = init_database_memory().await.unwrap();
    let agnes = create_provider(&db, "agnes", "Agnes").await;
    let custom = create_provider(&db, "custom", "Unrelated provider").await;
    let retired = "agnes-video-v2.0";
    // Also hide a legacy row incorrectly classified as image generation.
    seed_saved_model(&db, &agnes, retired, "image_generation", "agnes.images").await;
    seed_saved_model(&db, &custom, retired, "chat", "openai.chat_text").await;
    for (model, task, protocol) in [
        ("agnes-video-2.5", "video_generation", "agnes.video_jobs"),
        ("agnes-video-2.5-flash", "video_generation", "agnes.video_jobs"),
        ("agnes-image-2.0-flash", "image_generation", "agnes.images"),
        ("agnes-2.0-flash", "chat", "openai.chat_text"),
        ("agnes-video-future", "video_generation", "agnes.video_jobs"),
    ] {
        seed_saved_model(&db, &agnes, model, task, protocol).await;
    }
    let model_repo = SqliteProviderModelRepository::new(db.pool().clone());
    let capability_repo = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    let before_models = serde_json::to_value(model_repo.list().await.unwrap()).unwrap();
    let before_capabilities = serde_json::to_value(capability_repo.list().await.unwrap()).unwrap();
    let state = build_state(&db);
    let model_service = state.provider_model_service.clone();
    let app = system_routes(state);

    let response = app.clone().oneshot(request("GET", "/api/providers", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let providers = body_json(response).await;
    let agnes_view = providers["data"].as_array().unwrap().iter().find(|provider| provider["provider_id"] == agnes).unwrap();
    let custom_view = providers["data"].as_array().unwrap().iter().find(|provider| provider["provider_id"] == custom).unwrap();
    assert_eq!(agnes_view["models"].as_array().unwrap().len(), 6);
    assert!(!agnes_view["models"].as_array().unwrap().iter().any(|model| model["model"] == retired));
    assert!(custom_view["models"].as_array().unwrap().iter().any(|model| model["model"] == retired));

    for uri in ["/api/provider-models".to_owned(), format!("/api/provider-models?provider_id={agnes}")] {
        let response = app.clone().oneshot(request("GET", &uri, None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let models = body_json(response).await;
        let models = models["data"].as_array().unwrap();
        assert!(!models.iter().any(|model| model["provider_id"] == agnes && model["model"] == retired));
        assert!(models.iter().any(|model| model["provider_id"] == agnes && model["model"] == "agnes-video-2.5-flash"));
        assert!(models.iter().any(|model| model["provider_id"] == agnes && model["model"] == "agnes-video-future"));
        if uri == "/api/provider-models" {
            assert!(models.iter().any(|model| model["provider_id"] == custom && model["model"] == retired));
        }
    }
    assert!(model_service.get(&agnes, retired).await.unwrap().is_none());
    assert!(model_service.get(&custom, retired).await.unwrap().is_some());
    assert_eq!(serde_json::to_value(model_repo.list().await.unwrap()).unwrap(), before_models);
    assert_eq!(serde_json::to_value(capability_repo.list().await.unwrap()).unwrap(), before_capabilities);

    // Every aggregate response (including provider edits and clones) uses the
    // same projection; a later mutation must not make the hidden row reappear.
    let response = app.clone().oneshot(request("PUT", &format!("/api/providers/{agnes}"), Some(json!({"name":"Agnes renamed"})))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!body_json(response).await["data"]["models"].as_array().unwrap().iter().any(|model| model["model"] == retired));
    let response = app.oneshot(request("POST", &format!("/api/providers/{agnes}/clone"), None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let cloned = body_json(response).await;
    assert!(!cloned["data"]["models"].as_array().unwrap().iter().any(|model| model["model"] == retired));
    let cloned_id = cloned["data"]["provider_id"].as_str().unwrap();
    assert!(model_repo.get(cloned_id, retired).await.unwrap().is_some());
    assert!(model_repo.get(&agnes, retired).await.unwrap().is_some());
}

#[tokio::test]
async fn manual_configuration_cannot_reintroduce_the_retired_agnes_video_model() {
    let db = init_database_memory().await.unwrap();
    let agnes = create_provider(&db, "agnes", "Agnes").await;
    let app = system_routes(build_state(&db));
    for (task, protocol) in [("chat", "openai.chat_text"), ("image_generation", "agnes.images"), ("video_generation", "agnes.video_jobs")] {
        let response = app.clone().oneshot(request("PUT", "/api/provider-models", Some(json!({
            "provider_id":agnes, "model":{"model":"agnes-video-v2.0", "capabilities":[{
                "task":task, "protocol":protocol, "connection_role":"default", "provider_params":{}
            }]}
        })))).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = body_json(response).await;
        assert!(error.to_string().contains("taken offline"));
        assert!(error.to_string().contains("agnes-video-2.5-flash"));
    }
    let response = app.oneshot(request("POST", "/api/providers", Some(json!({
        "platform":"agnes", "name":"Retired model", "base_url":"https://api.example.test/v1",
        "auth_scheme":"bearer", "credentials":{"api_keys":["sk-test"]}, "initial_model":{
            "model":"agnes-video-v2.0", "capabilities":[chat_capability()]
        }
    })))).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(body_json(response).await.to_string().contains("taken offline"));
    assert!(SqliteProviderModelRepository::new(db.pool().clone()).get(&agnes, "agnes-video-v2.0").await.unwrap().is_none());
    assert_eq!(SqliteProviderRepository::new(db.pool().clone()).list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn task_protocol_mismatches_cannot_create_or_replace_saved_model_configuration() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "custom", "Task protocol contract").await;
    let app = system_routes(build_state(&db));
    let list_uri = format!("/api/provider-models?provider_id={provider_id}");
    let before = body_json(app.clone().oneshot(request("GET", &list_uri, None)).await.unwrap()).await;

    // Model IDs are intentionally arbitrary: validation is about the exact
    // task/protocol contract, independent of names or catalog suggestions.
    for (task, protocol) in [
        (ModelTask::ImageGeneration, "openai.chat_text"),
        (ModelTask::ImageEdit, "openai.chat_text"),
        (ModelTask::VideoGeneration, "openai.chat_text"),
        (ModelTask::MusicGeneration, "openai.chat_text"),
        (ModelTask::SpeechRecognition, "openai.chat_text"),
        (ModelTask::SpeechSynthesis, "openai.chat_text"),
        (ModelTask::Embedding, "openai.chat_text"),
        (ModelTask::Rerank, "openai.chat_text"),
        (ModelTask::Chat, "openai.audio_speech"),
        (ModelTask::SpeechRecognition, "openai.audio_speech"),
        (ModelTask::SpeechSynthesis, "openai.audio_transcriptions"),
    ] {
        for model in ["new-unknown-model", "seed-chat"] {
            let response = app.clone().oneshot(request(
                "PUT",
                "/api/provider-models",
                Some(json!({
                    "provider_id": provider_id,
                    "model": {
                        "model": model,
                        "description": "must never persist",
                        "capabilities": [{
                            "task": task,
                            "protocol": protocol,
                            "connection_role": "default",
                            "provider_params": {}
                        }]
                    }
                })),
            )).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{model}: {task:?}/{protocol}");
            let error = body_json(response).await;
            assert!(error.to_string().contains("task-incompatible"), "unexpected error: {error}");
        }
    }

    let after = body_json(app.oneshot(request("GET", &list_uri, None)).await.unwrap()).await;
    assert_eq!(after["data"], before["data"], "failed task/protocol validation must occur before any write");
    let resolved = build_invoke(&db).resolve_task_config(
        &ModelRef { provider_id, model: "seed-chat".into() },
        ModelTask::Chat,
    ).await.unwrap();
    assert_eq!(resolved.protocol, "openai.chat_text");
}

#[tokio::test]
async fn duplicate_traits_fail_at_save_and_unique_traits_resolve_unchanged() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "custom", "Trait contract").await;
    let model = "trait-contract-model";
    let save = |traits: Value| {
        json!({
            "provider_id": provider_id.clone(),
            "model": {
                "model": model,
                "capabilities": [{
                    "task": "chat",
                    "traits": traits,
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {}
                }]
            }
        })
    };

    let duplicate = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            "/api/provider-models",
            Some(save(json!(["streaming", "streaming"]))),
        ))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);

    let valid = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            "/api/provider-models",
            Some(save(json!(["vision_input", "web_search"]))),
        ))
        .await
        .unwrap();
    assert_eq!(valid.status(), StatusCode::OK);

    let resolved = build_invoke(&db)
        .resolve_task_config(
            &ModelRef {
                provider_id,
                model: model.to_owned(),
            },
            ModelTask::Chat,
        )
        .await
        .unwrap();
    assert_eq!(
        resolved.traits,
        vec![ModelTrait::VisionInput, ModelTrait::WebSearch]
    );
}

#[tokio::test]
async fn chat_context_settings_round_trip_through_model_routes() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "custom", "Context contract").await;
    let app = system_routes(build_state(&db));
    let save = |threshold: u8| json!({
        "provider_id": provider_id,
        "model": {
            "model": "context-model",
            "capabilities": [{
                "task": "chat",
                "protocol": "openai.chat_text",
                "connection_role": "default",
                "context_limit": 64_000,
                "compaction_threshold_pct": threshold
            }]
        }
    });
    let invalid = app.clone().oneshot(request("PUT", "/api/provider-models", Some(save(49)))).await.unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let saved = app.clone().oneshot(request("PUT", "/api/provider-models", Some(save(60)))).await.unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    let listed = app.oneshot(request(
        "GET",
        &format!("/api/provider-models?provider_id={provider_id}"),
        None,
    )).await.unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = body_json(listed).await;
    let capability = &listed["data"].as_array().unwrap().iter()
        .find(|row| row["model"] == "context-model")
        .unwrap()["capabilities"][0];
    assert_eq!(capability["context_limit"], 64_000);
    assert_eq!(capability["compaction_threshold_pct"], 60);
}

#[tokio::test]
async fn model_token_limits_round_trip_custom_and_explicit_default_without_clamping() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "custom", "Nullable budget contract").await;
    let app = system_routes(build_state(&db));
    for (context, output, explicit_null) in [
        (Some(1_000_003_i64), Some(531_007_i64), false),
        (Some(131_129), Some(31_007), false),
        (None, None, true),
        (Some(131_129), Some(31_007), false),
        (None, None, false),
    ] {
        let mut capability = chat_capability();
        if let Some(value) = context { capability["context_limit"] = json!(value); }
        else if explicit_null { capability["context_limit"] = Value::Null; }
        if let Some(value) = output { capability["output_limit"] = json!(value); }
        else if explicit_null { capability["output_limit"] = Value::Null; }
        let saved = app.clone().oneshot(request("PUT", "/api/provider-models", Some(json!({
            "provider_id": provider_id, "model":{"model":"budget-roundtrip", "capabilities":[capability]}
        })))).await.unwrap();
        assert_eq!(saved.status(), StatusCode::OK);
        let saved = body_json(saved).await;
        let capability = &saved["data"]["capabilities"][0];
        assert_eq!(capability.get("context_limit").and_then(Value::as_i64), context);
        assert_eq!(capability.get("output_limit").and_then(Value::as_i64), output);
        let repository = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
        let row = nomifun_db::IProviderModelCapabilityRepository::get(
            &repository, &provider_id, "budget-roundtrip", "chat"
        ).await.unwrap().unwrap();
        assert_eq!((row.context_limit, row.output_limit), (context, output));
    }
}

#[tokio::test]
async fn provider_clone_preserves_manual_and_default_model_token_limits() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "custom", "Budget clone source").await;
    let app = system_routes(build_state(&db));
    for (model, context, output) in [("manual-model", Some(1_000_003_i64), Some(31_007_i64)), ("default-model", None, None)] {
        let mut capability = chat_capability();
        capability["context_limit"] = json!(context);
        capability["output_limit"] = json!(output);
        if model == "manual-model" {
            capability["provider_params"] = json!({"_nomifun_context_limit_kind":"input_only"});
        }
        let response = app.clone().oneshot(request("PUT", "/api/provider-models", Some(json!({
            "provider_id":provider_id,"model":{"model":model,"capabilities":[capability]}
        })))).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let repository = SqliteProviderRepository::new(db.pool().clone());
    let cloned = nomifun_db::IProviderRepository::clone_graph(&repository, &provider_id, "Budget clone").await.unwrap();
    let capabilities = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    let source = nomifun_db::IProviderModelCapabilityRepository::list_for_provider(&capabilities, &provider_id)
        .await.unwrap().into_iter().map(|row|(row.model,row.task,row.context_limit,row.output_limit,row.provider_params)).collect::<Vec<_>>();
    let target = nomifun_db::IProviderModelCapabilityRepository::list_for_provider(&capabilities, &cloned.provider_id)
        .await.unwrap().into_iter().map(|row|(row.model,row.task,row.context_limit,row.output_limit,row.provider_params)).collect::<Vec<_>>();
    assert_eq!(target, source);
}

#[tokio::test]
async fn mixed_case_ark_platforms_reject_invalid_video_models_before_persistence() {
    let db = init_database_memory().await.unwrap();
    let app = system_routes(build_state(&db));
    for platform in ["ArK", "VOLCENGINE"] {
        let provider_id = create_provider(&db, platform, "Ark model validation").await;
        let save = |model: &str| {
            json!({
                "provider_id": provider_id,
                "model": {
                    "model": model,
                    "capabilities": [{
                        "task": "video_generation",
                        "protocol": "ark.video_jobs",
                        "connection_role": "default"
                    }]
                }
            })
        };
        for (model, expected_error) in [
            ("doubao-seedance-1.5-pro", "console display name"),
            ("doubao-seed-2-0-mini-260428", "Seed and Seedance"),
        ] {
            let response = app
                .clone()
                .oneshot(request("PUT", "/api/provider-models", Some(save(model))))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{platform}/{model}");
            let error = body_json(response).await;
            assert!(error.to_string().contains(expected_error), "{error}");
        }
        let listed = app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/api/provider-models?provider_id={provider_id}"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = body_json(listed).await;
        assert_eq!(listed["data"].as_array().unwrap().len(), 1);
        assert_eq!(listed["data"][0]["model"], "seed-chat");

        let model = "doubao-seedance-2-0-mini-260615";
        let response = app
            .clone()
            .oneshot(request("PUT", "/api/provider-models", Some(save(model))))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let resolved = build_invoke(&db)
            .resolve_task_config(
                &ModelRef { provider_id, model: model.to_owned() },
                ModelTask::VideoGeneration,
            )
            .await
            .unwrap();
        assert_eq!(resolved.protocol, "ark.video_jobs");
        assert_eq!(resolved.model, model);
    }
}

#[tokio::test]
async fn full_save_list_update_and_query_delete_roundtrip() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "stepfun", "StepFun").await;
    // Saving permits long natural keys; they must remain deletable as well.
    let model_name = format!("future-user-model-{}", "x".repeat(513));
    let model = model_name.as_str();

    let save = json!({
        "provider_id": provider_id.clone(),
        "model": {
            "model": model,
            "description": "user-entered model absent from the catalog",
            "capabilities": [
                {
                    "task": "speech_recognition",
                    "protocol": "stepfun.asr_sse",
                    "connection_role": "default",
                    "endpoint": "/audio/asr/sse",
                    "provider_params": {}
                },
                {
                    "task": "speech_synthesis",
                    "protocol": "stepfun.audio_speech",
                    "connection_role": "default",
                    "endpoint": "/audio/speech",
                    "provider_params": {"voice": "default"}
                }
            ]
        }
    });
    let response = system_routes(build_state(&db))
        .oneshot(request("PUT", "/api/provider-models", Some(save.clone())))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let saved = body_json(response).await;
    assert_eq!(saved["data"]["model"], model);
    assert_eq!(saved["data"]["capabilities"].as_array().unwrap().len(), 2);

    // Saving does not depend on a model-catalog hit. Both exact task rows must
    // immediately resolve through the same runtime authority used by probes
    // and real media calls.
    let invoke = build_invoke(&db);
    let model_ref = ModelRef {
        provider_id: provider_id.clone(),
        model: model.to_owned(),
    };
    let asr = invoke
        .resolve_task_config(&model_ref, ModelTask::SpeechRecognition)
        .await
        .unwrap();
    assert_eq!(asr.protocol, "stepfun.asr_sse");
    assert_eq!(asr.transport.endpoint.as_deref(), Some("/audio/asr/sse"));
    let tts = invoke
        .resolve_task_config(&model_ref, ModelTask::SpeechSynthesis)
        .await
        .unwrap();
    assert_eq!(tts.protocol, "stepfun.audio_speech");
    assert_eq!(tts.transport.endpoint.as_deref(), Some("/audio/speech"));
    assert_eq!(tts.provider_params["voice"], "default");

    let response = system_routes(build_state(&db))
        .oneshot(request(
            "GET",
            &format!("/api/provider-models?provider_id={provider_id}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["data"].as_array().unwrap().len(), 2);

    let updated = json!({
        "provider_id": provider_id.clone(),
        "model": {
            "model": model,
            "enabled": true,
            "description": "updated by user",
            "capabilities": [{
                "task": "speech_synthesis",
                "protocol": "stepfun.audio_speech",
                "connection_role": "default",
                "endpoint": "/audio/speech",
                "provider_params": {"voice": "updated"}
            }]
        }
    });
    let response = system_routes(build_state(&db))
        .oneshot(request("PUT", "/api/provider-models", Some(updated)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let updated = body_json(response).await;
    assert_eq!(updated["data"]["enabled"], true);
    assert_eq!(updated["data"]["description"], "updated by user");
    assert_eq!(updated["data"]["capabilities"].as_array().unwrap().len(), 1);
    assert_eq!(updated["data"]["capabilities"][0]["task"], "speech_synthesis");

    // PUT is an atomic full replacement: omitted ASR is gone, retained TTS is
    // updated, and a rejected replacement cannot disturb either fact.
    assert!(
        invoke
            .resolve_task_config(&model_ref, ModelTask::SpeechRecognition)
            .await
            .is_err()
    );
    let retained_tts = invoke
        .resolve_task_config(&model_ref, ModelTask::SpeechSynthesis)
        .await
        .unwrap();
    assert_eq!(retained_tts.provider_params["voice"], "updated");

    let invalid_replacement = json!({
        "provider_id": provider_id.clone(),
        "model": {
            "model": model,
            "description": "must roll back",
            "capabilities": [
                {
                    "task": "speech_synthesis",
                    "protocol": "stepfun.audio_speech",
                    "connection_role": "default",
                    "provider_params": {}
                },
                {
                    "task": "speech_synthesis",
                    "protocol": "stepfun.audio_speech",
                    "connection_role": "default",
                    "provider_params": {}
                }
            ]
        }
    });
    let response = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            "/api/provider-models",
            Some(invalid_replacement),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let after_rejection = system_routes(build_state(&db))
        .oneshot(request(
            "GET",
            &format!("/api/provider-models?provider_id={provider_id}"),
            None,
        ))
        .await
        .unwrap();
    let after_rejection = body_json(after_rejection).await;
    let persisted = after_rejection["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["model"] == model)
        .unwrap();
    assert_eq!(persisted["description"], "updated by user");
    assert_eq!(persisted["capabilities"].as_array().unwrap().len(), 1);
    assert_eq!(persisted["capabilities"][0]["task"], "speech_synthesis");

    let response = system_routes(build_state(&db))
        .oneshot(request(
            "DELETE",
            &format!("/api/provider-models?provider_id={provider_id}&model={model}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let providers = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    let providers = body_json(providers).await;
    assert_eq!(providers["data"][0]["models"].as_array().unwrap().len(), 1);

    let old_create = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/provider-models", Some(json!({}))))
        .await
        .unwrap();
    assert_eq!(old_create.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn base_url_override_origin_contract_matches_runtime_resolution() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "stepfun", "StepFun origin contract").await;
    let invoke = build_invoke(&db);

    let save = |model: &str, capability: Value| {
        json!({
            "provider_id": provider_id.clone(),
            "model": {"model": model, "capabilities": [capability]}
        })
    };

    let http_same = save(
        "http-origin",
        json!({
            "task":"chat",
            "protocol":"openai.chat_text",
            "connection_role":"default",
            "base_url_override":"https://api.example.test/v2",
            "endpoint":"chat/completions",
            "provider_params":{}
        }),
    );
    let response = system_routes(build_state(&db))
        .oneshot(request("PUT", "/api/provider-models", Some(http_same)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let resolved = invoke
        .resolve_task_config(
            &ModelRef {
                provider_id: provider_id.clone(),
                model: "http-origin".into(),
            },
            ModelTask::Chat,
        )
        .await
        .unwrap();
    assert_eq!(resolved.connection.base_url, "https://api.example.test/v2");

    let http_cross = save(
        "http-origin",
        json!({
            "task":"chat",
            "protocol":"openai.chat_text",
            "connection_role":"default",
            "base_url_override":"https://gateway.example.test/v1",
            "provider_params":{}
        }),
    );
    let response = system_routes(build_state(&db))
        .oneshot(request("PUT", "/api/provider-models", Some(http_cross)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let websocket_cross = save(
        "realtime-origin",
        json!({
            "task":"realtime_conversation",
            "protocol":"stepfun.realtime_s2s",
            "connection_role":"default",
            "base_url_override":"wss://realtime.example.test/v1",
            "realtime_endpoint":"realtime?model={model}",
            "provider_params":{}
        }),
    );
    let response = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            "/api/provider-models",
            Some(websocket_cross.clone()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let mut websocket_allowed = websocket_cross;
    websocket_allowed["model"]["capabilities"][0]["allow_cross_origin_credentials"] =
        json!(true);
    let response = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            "/api/provider-models",
            Some(websocket_allowed),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let resolved = invoke
        .resolve_task_config(
            &ModelRef {
                provider_id,
                model: "realtime-origin".into(),
            },
            ModelTask::RealtimeConversation,
        )
        .await
        .unwrap();
    assert_eq!(resolved.connection.base_url, "wss://realtime.example.test/v1");
}

#[tokio::test]
async fn full_save_rejects_unencodable_provider_params_before_persistence() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "openai", "OpenAI").await;
    let model = "multipart-complex-must-not-save";
    let response = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            "/api/provider-models",
            Some(json!({
                "provider_id": provider_id.clone(),
                "model": {
                    "model": model,
                    "capabilities": [{
                        "task": "image_edit",
                        "protocol": "openai.images",
                        "connection_role": "default",
                        "endpoint": "/images/edits",
                        "provider_params": {"future":{"nested":true}}
                    }]
                }
            })),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = body_json(response).await;
    assert!(
        error.to_string().contains("cannot losslessly encode"),
        "unexpected error: {error}"
    );

    let listed = system_routes(build_state(&db))
        .oneshot(request(
            "GET",
            &format!("/api/provider-models?provider_id={provider_id}"),
            None,
        ))
        .await
        .unwrap();
    assert!(body_json(listed).await["data"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["model"] != model));
}

#[tokio::test]
async fn list_filters_by_provider_id() {
    let db = init_database_memory().await.unwrap();
    let first = create_provider(&db, "openai", "One").await;
    let second = create_provider(&db, "openai", "Two").await;

    let all = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/provider-models", None))
        .await
        .unwrap();
    assert_eq!(body_json(all).await["data"].as_array().unwrap().len(), 2);

    let filtered = system_routes(build_state(&db))
        .oneshot(request(
            "GET",
            &format!("/api/provider-models?provider_id={first}"),
            None,
        ))
        .await
        .unwrap();
    let filtered = body_json(filtered).await;
    assert_eq!(filtered["data"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["data"][0]["provider_id"], first);
    assert_ne!(filtered["data"][0]["provider_id"], second);
}

#[tokio::test]
async fn invalid_capability_graph_and_missing_delete_are_rejected() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "openai", "OpenAI").await;

    let duplicate = json!({
        "provider_id": provider_id,
        "model": {
            "model": "duplicate",
            "capabilities": [chat_capability(), chat_capability()]
        }
    });
    let response = system_routes(build_state(&db))
        .oneshot(request("PUT", "/api/provider-models", Some(duplicate)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let missing_role = json!({
        "provider_id": provider_id,
        "model": {
            "model": "voice",
            "capabilities": [{
                "task": "speech_synthesis",
                "protocol": "openai.audio_speech",
                "connection_role": "voice",
                "provider_params": {}
            }]
        }
    });
    let response = system_routes(build_state(&db))
        .oneshot(request("PUT", "/api/provider-models", Some(missing_role)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = system_routes(build_state(&db))
        .oneshot(request(
            "DELETE",
            &format!("/api/provider-models?provider_id={provider_id}&model=missing"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

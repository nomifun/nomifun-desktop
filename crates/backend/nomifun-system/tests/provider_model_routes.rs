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
        None,
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
        "traits": ["streaming"],
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
            Some(save(json!(["streaming", "function_calling"]))),
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
        vec![ModelTrait::Streaming, ModelTrait::FunctionCalling]
    );
}

#[tokio::test]
async fn full_save_list_update_and_query_delete_roundtrip() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "stepfun", "StepFun").await;
    let model = "future-user-model-2026-08-11";

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

mod common;

use nomifun_db::init_database_memory;
use nomifun_system::{
    VersionCheckService,
    model_management::{ADD_MODEL, CREATE_PROVIDER, INSPECT, ModelManagementService},
};
use serde_json::{Value, json};

fn services(
    db: &nomifun_db::Database,
) -> (ModelManagementService, nomifun_system::SystemRouterState) {
    let http = reqwest::Client::new();
    let state = common::build_system_state(
        db,
        [42; 32],
        http.clone(),
        VersionCheckService::new(http, "0.1.0".into()),
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    );
    (
        ModelManagementService::new(
            state.provider_service.clone(),
            state.provider_model_service.clone(),
            state.provider_connection_service.clone(),
        ),
        state,
    )
}

fn model(name: &str) -> Value {
    json!({"model":name,"capabilities":[{"task":"chat","protocol":"openai.chat_text","connection_role":"default"}]})
}

fn provider() -> Value {
    json!({"platform":"custom","name":"Conversation import","base_url":"https://models.example/v1","auth_scheme":"bearer", "credentials":{"api_keys":["arbitrary-secret-without-known-prefix"]},"initial_model":model("chat-one")})
}

#[tokio::test]
async fn imported_configuration_is_visible_to_management_and_keeps_credentials_private() {
    let db = init_database_memory().await.unwrap();
    let (service, state) = services(&db);
    let created = service.execute(CREATE_PROVIDER, provider()).await.unwrap();
    assert_eq!(created["status"], "created");
    assert_eq!(created["connection_tested"], false);
    let id = created["provider_id"].as_str().unwrap();
    let list = service
        .execute(INSPECT, json!({"operation":"list"}))
        .await
        .unwrap();
    assert!(!list.to_string().contains("arbitrary-secret"));
    assert!(!created.to_string().contains("arbitrary-secret"));
    assert_eq!(list["providers"][0]["models"][0]["model"], "chat-one");
    assert_eq!(
        state.provider_service.api_keys(id).await.unwrap(),
        vec!["arbitrary-secret-without-known-prefix"]
    );
    let encrypted: String = sqlx_query_credentials(&db, id).await;
    assert!(!encrypted.contains("arbitrary-secret"));
    service
        .execute(
            ADD_MODEL,
            json!({"provider_id":id,"model":model("chat-two")}),
        )
        .await
        .unwrap();
    assert_eq!(
        state.provider_service.list().await.unwrap()[0].models.len(),
        2
    );
    assert_eq!(
        state.provider_service.api_keys(id).await.unwrap(),
        vec!["arbitrary-secret-without-known-prefix"]
    );
}

// Use the repository boundary to inspect encrypted-at-rest material.
async fn sqlx_query_credentials(db: &nomifun_db::Database, id: &str) -> String {
    use nomifun_db::IProviderRepository;
    nomifun_db::SqliteProviderRepository::new(db.pool().clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap()
        .credentials_encrypted
}

#[tokio::test]
async fn duplicate_imports_and_trimmed_model_names_do_not_overwrite() {
    let db = init_database_memory().await.unwrap();
    let (service, state) = services(&db);
    let created = service.execute(CREATE_PROVIDER, provider()).await.unwrap();
    let id = created["provider_id"].as_str().unwrap();
    let before = state.provider_service.list().await.unwrap();
    assert!(
        service
            .execute(CREATE_PROVIDER, provider())
            .await
            .unwrap_err()
            .contains("already exists")
    );
    let mut replacement = model("chat-one");
    replacement["capabilities"][0]["protocol"] = json!("openai.responses");
    assert!(
        service
            .execute(ADD_MODEL, json!({"provider_id":id,"model":replacement}))
            .await
            .unwrap_err()
            .contains("already exists")
    );
    assert!(
        service
            .execute(
                ADD_MODEL,
                json!({"provider_id":id,"model":model(" chat-one ")})
            )
            .await
            .is_err()
    );
    assert_eq!(state.provider_service.list().await.unwrap(), before);
}

#[tokio::test]
async fn invalid_protocol_and_connection_leave_no_partial_provider() {
    let db = init_database_memory().await.unwrap();
    let (service, state) = services(&db);
    let mut invalid = provider();
    invalid["initial_model"]["capabilities"][0]["protocol"] = json!("imaginary.protocol");
    assert!(service.execute(CREATE_PROVIDER, invalid).await.is_err());
    let mut invalid = provider();
    invalid["initial_model"]["capabilities"][0]["connection_role"] = json!("missing");
    assert!(service.execute(CREATE_PROVIDER, invalid).await.is_err());
    assert!(state.provider_service.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn named_connection_and_multimodal_configuration_use_the_existing_contract() {
    let db = init_database_memory().await.unwrap();
    let (service, state) = services(&db);
    let mut input = provider();
    input["initial_model"]["capabilities"].as_array_mut().unwrap().push(json!({
        "task":"speech_synthesis","protocol":"openai.audio_speech","connection_role":"voice","provider_params":{"voice":"alloy"}
    }));
    input["connections"] = json!([{"role":"voice","base_url":"https://voice.example/v1","auth_scheme":"bearer","credentials":{"api_keys":["voice-secret"]}}]);
    let created = service.execute(CREATE_PROVIDER, input).await.unwrap();
    let id = created["provider_id"].as_str().unwrap();
    assert_eq!(
        state.provider_model_service.list(Some(id)).await.unwrap()[0]
            .capabilities
            .len(),
        2
    );
    let list = service
        .execute(INSPECT, json!({"operation":"list","provider_id":id}))
        .await
        .unwrap();
    assert_eq!(list["providers"][0]["connections"][0]["role"], "voice");
    assert!(!list.to_string().contains("voice-secret"));
}

#[tokio::test]
async fn protocol_inspection_is_authoritative_and_has_no_effects() {
    let db = init_database_memory().await.unwrap();
    let (service, state) = services(&db);
    let found = service
        .execute(
            INSPECT,
            json!({"operation":"protocols","platform":"custom","task":"chat"}),
        )
        .await
        .unwrap();
    assert_eq!(
        found["manifest"],
        serde_json::to_value(nomifun_model_invoke::protocol_manifest_for(
            "custom",
            nomifun_api_types::ModelTask::Chat
        ))
        .unwrap()
    );
    assert!(
        service
            .execute(
                INSPECT,
                json!({"operation":"protocols","platform":"custom"})
            )
            .await
            .is_err()
    );
    assert!(
        service
            .execute("model.management/delete", json!({}))
            .await
            .is_err()
    );
    assert!(state.provider_service.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn short_credentials_do_not_corrupt_saved_provider_identity() {
    let db = init_database_memory().await.unwrap();
    let (service, _) = services(&db);
    let mut input = provider();
    input["credentials"] = json!({"api_keys":["a"]});
    let created = service.execute(CREATE_PROVIDER, input).await.unwrap();
    let id = created["provider_id"].as_str().unwrap();
    nomifun_common::ProviderId::parse(id).unwrap();
    service.execute(ADD_MODEL, json!({"provider_id":id,"model":model("another")})).await.unwrap();
}

#[tokio::test]
async fn errors_do_not_echo_arbitrary_credentials() {
    let db = init_database_memory().await.unwrap();
    let (service, _) = services(&db);
    let mut input = provider();
    input["auth_scheme"] = json!("arbitrary-secret-without-known-prefix");
    let error = service.execute(CREATE_PROVIDER, input).await.unwrap_err();
    assert!(
        !error.contains("arbitrary-secret-without-known-prefix"),
        "{error}"
    );
}

#[tokio::test]
async fn simultaneous_conversation_additions_do_not_replace_models() {
    let db = init_database_memory().await.unwrap();
    let (service, state) = services(&db);
    let created = service.execute(CREATE_PROVIDER, provider()).await.unwrap();
    let input = json!({"provider_id":created["provider_id"],"model":model("chat-two")});
    let (a, b) = tokio::join!(
        service.execute(ADD_MODEL, input.clone()),
        service.execute(ADD_MODEL, input)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        state.provider_service.list().await.unwrap()[0].models.len(),
        2
    );
}

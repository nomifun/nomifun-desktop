//! Integration tests verifying that --work-dir is used for conversation workspace creation.

use nomifun_api_types::CreateConversationRequest;
use nomifun_app::{AppConfig, AppServices, build_conversation_state};
use nomifun_common::AgentType;

#[tokio::test]
async fn conversation_workspace_uses_work_dir() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let work_dir = tempfile::TempDir::new().unwrap();

    let db = nomifun_db::init_database_memory().await.unwrap();
    let config = AppConfig {
        data_dir: data_dir.path().to_path_buf(),
        work_dir: work_dir.path().to_path_buf(),
        auth_policy: nomifun_app::AuthPolicy::NoAuth,
        ..Default::default()
    };
    let services = AppServices::from_config(db, &config).await.unwrap();
    let state = build_conversation_state(&services, None);

    let request = CreateConversationRequest {
        r#type: AgentType::Acp,
        name: Some("test".to_string()),
        model: None,
        source: None,
        channel_chat_id: None,
        extra: serde_json::json!({}),
    };
    let response = state.service.create("system_default_user", request).await.unwrap();

    let workspace = response.extra.get("workspace").and_then(|v| v.as_str()).unwrap();
    assert!(
        workspace.starts_with(work_dir.path().to_str().unwrap()),
        "workspace should be under work_dir, got: {workspace}"
    );
    assert!(
        !workspace.starts_with(data_dir.path().to_str().unwrap()),
        "workspace should NOT be under data_dir, got: {workspace}"
    );
}

#[tokio::test]
async fn user_specified_workspace_is_not_overridden() {
    let data_dir = tempfile::TempDir::new().unwrap();
    let work_dir = tempfile::TempDir::new().unwrap();
    let custom_workspace = tempfile::TempDir::new().unwrap();

    let db = nomifun_db::init_database_memory().await.unwrap();
    let config = AppConfig {
        data_dir: data_dir.path().to_path_buf(),
        work_dir: work_dir.path().to_path_buf(),
        auth_policy: nomifun_app::AuthPolicy::NoAuth,
        ..Default::default()
    };
    let services = AppServices::from_config(db, &config).await.unwrap();
    let state = build_conversation_state(&services, None);

    let request = CreateConversationRequest {
        r#type: AgentType::Acp,
        name: Some("test".to_string()),
        model: None,
        source: None,
        channel_chat_id: None,
        extra: serde_json::json!({
            "workspace": custom_workspace.path().to_str().unwrap()
        }),
    };
    let response = state.service.create("system_default_user", request).await.unwrap();

    let workspace = response.extra.get("workspace").and_then(|v| v.as_str()).unwrap();
    assert!(
        workspace.starts_with(custom_workspace.path().to_str().unwrap()),
        "workspace should use user-specified path, got: {workspace}"
    );
}

#[tokio::test]
async fn workspace_defaults_to_data_dir_when_work_dir_equals_data_dir() {
    let data_dir = tempfile::TempDir::new().unwrap();

    let db = nomifun_db::init_database_memory().await.unwrap();
    let config = AppConfig {
        data_dir: data_dir.path().to_path_buf(),
        work_dir: data_dir.path().to_path_buf(),
        auth_policy: nomifun_app::AuthPolicy::NoAuth,
        ..Default::default()
    };
    let services = AppServices::from_config(db, &config).await.unwrap();
    let state = build_conversation_state(&services, None);

    let request = CreateConversationRequest {
        r#type: AgentType::Acp,
        name: Some("test".to_string()),
        model: None,
        source: None,
        channel_chat_id: None,
        extra: serde_json::json!({}),
    };
    let response = state.service.create("system_default_user", request).await.unwrap();

    let workspace = response.extra.get("workspace").and_then(|v| v.as_str()).unwrap();
    assert!(
        workspace.starts_with(data_dir.path().to_str().unwrap()),
        "workspace should be under data_dir when work_dir == data_dir, got: {workspace}"
    );
}

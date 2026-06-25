use nomifun_ai_agent::cc_switch::{
    CcSwitchPaths, build_model_info_from_env, read_claude_model_info_with_paths, read_claude_provider_env_with_paths,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

fn create_test_db(dir: &std::path::Path, provider_id: &str, settings_config: &str) {
    let db_path = dir.join("cc-switch.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL,
            PRIMARY KEY (id, app_type)
        );
        CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO providers (id, app_type, name, settings_config) VALUES (?1, 'claude', 'Test Provider', ?2)",
        [provider_id, settings_config],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO model_pricing (model_id, display_name) VALUES (?1, ?2)",
        ["deepseek-v4-pro", "DeepSeek V4 Pro"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO model_pricing (model_id, display_name) VALUES (?1, ?2)",
        ["deepseek-v4-max", "DeepSeek V4 Max"],
    )
    .unwrap();
}

#[test]
fn reads_provider_env_from_fixture_db() {
    let tmp = TempDir::new().unwrap();
    let cc_switch_dir = tmp.path().join(".cc-switch");
    fs::create_dir_all(&cc_switch_dir).unwrap();

    let settings = r#"{"currentProviderClaude": "deepseek-relay"}"#;
    fs::write(cc_switch_dir.join("settings.json"), settings).unwrap();

    let config = serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": "https://relay.example.com/v1",
            "ANTHROPIC_API_KEY": "sk-relay-test-key",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "deepseek-v4-pro",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "deepseek-v4-max"
        },
        "model": "default"
    });
    create_test_db(&cc_switch_dir, "deepseek-relay", &config.to_string());

    let paths = CcSwitchPaths::from_home(tmp.path());
    let env = read_claude_provider_env_with_paths(&paths);

    assert_eq!(env.get("ANTHROPIC_BASE_URL").unwrap(), "https://relay.example.com/v1");
    assert_eq!(env.get("ANTHROPIC_API_KEY").unwrap(), "sk-relay-test-key");
    assert_eq!(env.get("ANTHROPIC_DEFAULT_SONNET_MODEL").unwrap(), "deepseek-v4-pro");
    assert_eq!(env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").unwrap(), "deepseek-v4-max");
}

#[test]
fn reads_model_info_from_fixture_db() {
    let tmp = TempDir::new().unwrap();
    let cc_switch_dir = tmp.path().join(".cc-switch");
    fs::create_dir_all(&cc_switch_dir).unwrap();

    let settings = r#"{"currentProviderClaude": "deepseek-relay"}"#;
    fs::write(cc_switch_dir.join("settings.json"), settings).unwrap();

    let config = serde_json::json!({
        "env": {
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "deepseek-v4-pro",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "deepseek-v4-max"
        },
        "model": "default"
    });
    create_test_db(&cc_switch_dir, "deepseek-relay", &config.to_string());

    let paths = CcSwitchPaths::from_home(tmp.path());
    let info = read_claude_model_info_with_paths(&paths);

    assert!(info.is_some());
    let payload = info.unwrap();
    assert_eq!(payload.available_models.len(), 2);
    assert_eq!(payload.current_model_id.as_deref(), Some("default"));
    assert_eq!(payload.current_model_label.as_deref(), Some("DeepSeek V4 Pro"));
    assert_eq!(payload.available_models[0].label, "DeepSeek V4 Pro");
    assert_eq!(payload.available_models[1].label, "DeepSeek V4 Max");
}

#[test]
fn gracefully_handles_missing_cc_switch() {
    let tmp = TempDir::new().unwrap();
    let paths = CcSwitchPaths::from_home(tmp.path());

    let env = read_claude_provider_env_with_paths(&paths);
    assert!(env.is_empty());

    let info = read_claude_model_info_with_paths(&paths);
    assert!(info.is_none());
}

#[test]
fn gracefully_handles_empty_provider_id() {
    let tmp = TempDir::new().unwrap();
    let cc_switch_dir = tmp.path().join(".cc-switch");
    fs::create_dir_all(&cc_switch_dir).unwrap();

    fs::write(cc_switch_dir.join("settings.json"), r#"{"currentProviderClaude": ""}"#).unwrap();

    let paths = CcSwitchPaths::from_home(tmp.path());
    let env = read_claude_provider_env_with_paths(&paths);
    assert!(env.is_empty());
}

#[test]
fn default_provider_returns_empty_env_when_no_env_configured() {
    let tmp = TempDir::new().unwrap();
    let cc_switch_dir = tmp.path().join(".cc-switch");
    fs::create_dir_all(&cc_switch_dir).unwrap();

    let settings = r#"{"currentProviderClaude": "default"}"#;
    fs::write(cc_switch_dir.join("settings.json"), settings).unwrap();

    let config = serde_json::json!({
        "env": {}
    });
    create_test_db(&cc_switch_dir, "default", &config.to_string());

    let paths = CcSwitchPaths::from_home(tmp.path());
    let env = read_claude_provider_env_with_paths(&paths);
    assert!(env.is_empty());
}

#[test]
fn build_model_info_from_env_works_standalone() {
    let mut env = HashMap::new();
    env.insert("ANTHROPIC_DEFAULT_SONNET_MODEL".into(), "test-model".into());

    let labels = HashMap::from([("test-model".to_owned(), "Test Model Display".to_owned())]);

    let info = build_model_info_from_env(&env, &labels, None);
    assert!(info.is_some());
    let payload = info.unwrap();
    assert_eq!(payload.available_models[0].label, "Test Model Display");
}

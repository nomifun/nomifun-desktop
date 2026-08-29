//! Integration tests for agent type implementations and auxiliary features.
//!
//! These tests validate:
//! - Each agent manager implements AgentRuntimeControl correctly
//! - Agent factory can build all agent types
//! - Workspace browsing works with real filesystem
//! - Nomi stub returns appropriate errors

use nomifun_ai_agent::manager::nomi::NomiAgentManager;
use nomifun_ai_agent::types::NomiResolvedConfig;
use nomifun_ai_agent::*;
use nomifun_common::{AgentKillReason, AgentType, ConversationStatus};

const NOMI_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000221";

// ---------------------------------------------------------------------------
// Nomi agent tests (real implementation with AgentEngine)
// ---------------------------------------------------------------------------

fn make_nomi_config() -> NomiResolvedConfig {
    NomiResolvedConfig {
        provider: "anthropic".into(),
        api_key: "sk-test-key".into(),
        model: "claude-sonnet-4-20250514".into(),
        base_url: None,
        system_prompt: None,
        output_ceiling: Some(4096),
        max_turns: None,
        context_limit: None,
        compat_overrides: Default::default(),
        session_directory: std::env::temp_dir().join("nomi-test-sessions"),
        extra_mcp_servers: Default::default(),
        loopback_capability_leases: Default::default(),
        bedrock_config: None,
        computer_use: false,
        browser_use: false,
        browser_source: "managed".to_owned(),
        browser_full_power: false,
        browser_persistent_login: false,
        browser_site_memory: false,
        browser_visual_fallback: false,
        goal: None,
        persistent_login_key: None,
        owner_token: None,
        install_embedded_agent_execution: true,
        allowed_tools: Vec::new(),
        write_root: None,
    }
}

#[tokio::test]
async fn nomi_agent_kill_succeeds() {
    let agent = NomiAgentManager::new(NOMI_CONVERSATION_ID.into(), "/proj".into(), make_nomi_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
        .await
        .unwrap();
    assert!(agent.kill(None).is_ok());
    assert!(agent.kill(Some(AgentKillReason::IdleTimeout)).is_ok());
}

#[tokio::test]
async fn nomi_agent_metadata() {
    let agent = NomiAgentManager::new(NOMI_CONVERSATION_ID.into(), "/work".into(), make_nomi_config(), None, None, None, None, Vec::new(), None, None, Vec::new(), None)
        .await
        .unwrap();
    assert_eq!(agent.agent_type(), AgentType::Nomi);
    assert_eq!(agent.workspace(), "/work");
    assert_eq!(agent.conversation_id(), NOMI_CONVERSATION_ID);
    assert_eq!(agent.status(), Some(ConversationStatus::Pending));
}

// ---------------------------------------------------------------------------
// Workspace browsing (uses real filesystem via tempdir)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workspace_browse_reads_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path();

    // Create test files and dirs
    std::fs::create_dir(base.join("src")).unwrap();
    std::fs::create_dir(base.join("tests")).unwrap();
    std::fs::write(base.join("Cargo.toml"), "# test").unwrap();
    std::fs::write(base.join("README.md"), "# readme").unwrap();

    let mut entries = Vec::new();
    let mut dir_reader = tokio::fs::read_dir(base).await.unwrap();
    while let Ok(Some(entry)) = dir_reader.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = entry.file_type().await.unwrap();
        let entry_type = if ft.is_dir() { "directory" } else { "file" };
        entries.push((name, entry_type.to_string()));
    }

    assert_eq!(entries.len(), 4);

    // Check that directories exist
    let dir_names: Vec<&str> = entries
        .iter()
        .filter(|(_, t)| t == "directory")
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(dir_names.contains(&"src"));
    assert!(dir_names.contains(&"tests"));

    // Check that files exist
    let file_names: Vec<&str> = entries
        .iter()
        .filter(|(_, t)| t == "file")
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(file_names.contains(&"Cargo.toml"));
    assert!(file_names.contains(&"README.md"));
}

// ---------------------------------------------------------------------------
// Agent type metadata validation
// ---------------------------------------------------------------------------

#[test]
fn agent_type_serde_all_variants() {
    // Verify that all AgentType variants serialize/deserialize correctly
    for (variant, expected_json) in [(AgentType::Nomi, "\"nomi\"")] {
        let json = serde_json::to_string(&variant).unwrap();
        assert_eq!(json, expected_json, "Failed for {:?}", variant);
        let parsed: AgentType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, variant);
    }
}

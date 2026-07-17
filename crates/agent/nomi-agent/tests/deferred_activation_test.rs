mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use nomi_agent::engine::AgentEngine;
use nomi_agent::output::null_sink::NullSink;
use nomi_agent::session::{Session, SessionManager};
use nomi_protocol::events::ToolCategory;
use nomi_providers::{LlmProvider, ProviderError};
use nomi_tools::registry::ToolRegistry;
use nomi_tools::{Tool, tool_search::ToolSearchTool};
use nomi_types::llm::{LlmEvent, LlmRequest};
use nomi_types::message::{StopReason, TokenUsage};
use nomi_types::tool::ToolResult;
use serde_json::{Value, json};
use tokio::sync::mpsc;

const DEFERRED_TOOL: &str = "nomi_knowledge_update_base";

struct DeferredKnowledgeTool;

#[async_trait]
impl Tool for DeferredKnowledgeTool {
    fn name(&self) -> &str {
        DEFERRED_TOOL
    }

    fn description(&self) -> &str {
        "Update a managed knowledge base"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kb_id": {"type": "string"},
                "description": {"type": "string"}
            },
            "required": ["kb_id"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    async fn execute(&self, _input: Value) -> ToolResult {
        ToolResult::text("updated")
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Exec
    }

    fn is_deferred(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct RecordingProvider {
    requests: Mutex<Vec<LlmRequest>>,
    search_on_first_turn: bool,
}

impl RecordingProvider {
    fn with_tool_search() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            search_on_first_turn: true,
        }
    }

    fn requests(&self) -> Vec<LlmRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<mpsc::Receiver<LlmEvent>, ProviderError> {
        let turn = {
            let mut requests = self.requests.lock().unwrap();
            let turn = requests.len();
            requests.push(request.clone());
            turn
        };
        let (tx, rx) = mpsc::channel(4);
        if self.search_on_first_turn && turn == 0 {
            tx.send(LlmEvent::ToolUse {
                id: "search-1".into(),
                name: "ToolSearch".into(),
                input: json!({"query": DEFERRED_TOOL}),
                extra: None,
            })
            .await
            .unwrap();
            tx.send(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: TokenUsage::default(),
            })
            .await
            .unwrap();
        } else {
            tx.send(LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .await
            .unwrap();
        }
        Ok(rx)
    }
}

fn registry_with_deferred_tool() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(DeferredKnowledgeTool));
    let state = registry.deferred_state();
    registry.register(Box::new(ToolSearchTool::new(state)));
    registry
}

fn registry_with_tool_search_only() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    let state = registry.deferred_state();
    registry.register(Box::new(ToolSearchTool::new(state)));
    registry
}

fn resumed_session(id: &str) -> Session {
    let now = Utc::now();
    Session {
        id: id.into(),
        created_at: now,
        updated_at: now,
        provider: "anthropic".into(),
        model: "test-model".into(),
        cwd: std::env::current_dir().unwrap().to_string_lossy().into_owned(),
        total_usage: TokenUsage::default(),
        messages: Vec::new(),
        owner_token: None,
        activated_deferred_tools: vec![DEFERRED_TOOL.into()],
    }
}

fn find_tool<'a>(request: &'a LlmRequest, name: &str) -> &'a nomi_types::tool::ToolDef {
    request
        .tools
        .iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("missing tool {name}"))
}

#[tokio::test]
async fn tool_search_activates_full_schema_on_next_provider_turn() {
    let provider = Arc::new(RecordingProvider::with_tool_search());
    let mut engine = AgentEngine::new_with_provider(
        provider.clone(),
        common::test_config(),
        registry_with_deferred_tool(),
        Arc::new(NullSink),
        std::env::current_dir().unwrap(),
    );

    engine.execute_turn("update the base", "msg-1").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let first = find_tool(&requests[0], DEFERRED_TOOL);
    assert!(first.deferred, "first provider turn must receive a deferred stub");
    let first_wire = nomi_providers::anthropic_shared::build_tools(&[first.clone()]);
    assert!(first_wire[0]["input_schema"]["properties"]
        .as_object()
        .unwrap()
        .is_empty());

    let second = find_tool(&requests[1], DEFERRED_TOOL);
    assert!(!second.deferred, "ToolSearch must activate the next provider definition");
    assert_eq!(second.input_schema["required"][0], "kb_id");
    assert_eq!(second.input_schema["properties"]["kb_id"]["type"], "string");
    let second_wire = nomi_providers::anthropic_shared::build_tools(&[second.clone()]);
    assert_eq!(
        second_wire[0]["input_schema"]["properties"]["kb_id"]["type"],
        "string"
    );
}

#[tokio::test]
async fn resumed_session_restores_deferred_activations_before_first_provider_turn() {
    let provider = Arc::new(RecordingProvider::default());
    let mut engine = AgentEngine::resume_with_provider(
        provider.clone(),
        common::test_config(),
        registry_with_deferred_tool(),
        Arc::new(NullSink),
        resumed_session("resume-deferred"),
        std::env::current_dir().unwrap(),
    );

    engine.execute_turn("continue", "msg-resume").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let restored = find_tool(&requests[0], DEFERRED_TOOL);
    assert!(!restored.deferred);
    assert_eq!(restored.input_schema["required"][0], "kb_id");
    assert_eq!(restored.input_schema["properties"]["kb_id"]["type"], "string");
}

#[tokio::test]
async fn resumed_activation_applies_when_add_mcp_registers_before_first_provider_request() {
    let provider = Arc::new(RecordingProvider::default());
    let mut engine = AgentEngine::resume_with_provider(
        provider.clone(),
        common::test_config(),
        registry_with_tool_search_only(),
        Arc::new(NullSink),
        resumed_session("resume-before-add-mcp"),
        std::env::current_dir().unwrap(),
    );

    // Mirrors nomi-cli's pre-message AddMcpServer phase: the engine is already
    // resumed, then the dynamic deferred proxy is registered before Message.
    engine.registry_mut().register(Box::new(DeferredKnowledgeTool));
    engine.execute_turn("continue", "msg-late-mcp").await.unwrap();

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let restored = find_tool(&requests[0], DEFERRED_TOOL);
    assert!(!restored.deferred, "late MCP registration must consume pending activation");
    assert_eq!(restored.input_schema["required"][0], "kb_id");
    assert_eq!(restored.input_schema["properties"]["kb_id"]["type"], "string");
}

#[tokio::test]
async fn pending_resumed_activation_is_not_erased_by_session_save() {
    let directory = tempfile::tempdir().unwrap();
    let manager = SessionManager::new(directory.path().to_path_buf(), 5);
    let session = resumed_session("persist-pending-deferred");
    manager.save(&session).unwrap();
    manager.update_index_for(&session).unwrap();

    let provider = Arc::new(RecordingProvider::default());
    let mut config = common::test_config();
    config.session.enabled = true;
    config.session.directory = directory.path().to_string_lossy().into_owned();
    let mut engine = AgentEngine::resume_with_provider(
        provider,
        config,
        registry_with_tool_search_only(),
        Arc::new(NullSink),
        session,
        std::env::current_dir().unwrap(),
    );

    engine.execute_turn("save pending state", "msg-save-pending").await.unwrap();

    let saved = manager.load("persist-pending-deferred").unwrap();
    assert_eq!(saved.activated_deferred_tools, vec![DEFERRED_TOOL.to_string()]);
}

#[tokio::test]
async fn activated_deferred_tools_are_saved_with_the_session() {
    let directory = tempfile::tempdir().unwrap();
    let provider = Arc::new(RecordingProvider::with_tool_search());
    let mut config = common::test_config();
    config.session.enabled = true;
    config.session.directory = directory.path().to_string_lossy().into_owned();
    let mut engine = AgentEngine::new_with_provider(
        provider,
        config,
        registry_with_deferred_tool(),
        Arc::new(NullSink),
        std::env::current_dir().unwrap(),
    );
    engine
        .init_session("anthropic", ".", Some("persist-deferred"))
        .unwrap();

    engine.execute_turn("load the tool", "msg-save").await.unwrap();

    let manager = SessionManager::new(directory.path().to_path_buf(), 5);
    let saved = manager.load("persist-deferred").unwrap();
    assert_eq!(saved.activated_deferred_tools, vec![DEFERRED_TOOL.to_string()]);
}

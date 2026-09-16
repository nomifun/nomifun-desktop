use super::*;
use crate::tool_middleware::{BeforeToolDecision, BeforeToolInput, ToolCallMiddleware};
use std::sync::atomic::AtomicUsize;

struct Provider {
    calls: Arc<AtomicUsize>,
    path: PathBuf,
}
#[async_trait::async_trait]
impl LlmProvider for Provider {
    async fn stream(
        &self,
        _: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let round = self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(3);
        if round > 0 {
            tx.try_send(LlmEvent::TextDelta("Recorded.".into()))
                .unwrap();
            tx.try_send(LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            })
            .unwrap();
            return Ok(rx);
        }
        for name in ["Write", "second"] {
            tx.try_send(LlmEvent::ToolUse {
                id: name.into(),
                name: name.into(),
                input: if name == "Write" {
                    serde_json::json!({"file_path":self.path,"content":"completed"})
                } else {
                    serde_json::json!({})
                },
                extra: None,
            })
            .unwrap();
        }
        tx.try_send(LlmEvent::Done {
            stop_reason: StopReason::ToolUse,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}
struct Probe {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl nomi_tools::Tool for Probe {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "before_tool probe"
    }
    fn input_schema(&self) -> Value {
        serde_json::json!({"type":"object"})
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        false
    }
    async fn preflight_hook(
        &self,
        _: &Value,
        _: &nomi_tools::ToolExecutionContext,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn execute(&self, input: Value) -> nomi_types::tool::ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.name == "Write" {
            std::fs::write(input["file_path"].as_str().unwrap(), "completed").unwrap();
        }
        nomi_types::tool::ToolResult::text("completed original result")
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Edit
    }
}
struct FailSecond {
    deny: bool,
}
#[async_trait::async_trait]
impl ToolCallMiddleware for FailSecond {
    async fn before_tool(&self, input: BeforeToolInput) -> Result<BeforeToolDecision, String> {
        if input.tool_name == "second" {
            if self.deny {
                Ok(BeforeToolDecision::Deny {
                    reason: "business denied".into(),
                })
            } else {
                Err("service failure".into())
            }
        } else {
            Ok(BeforeToolDecision::Allow {})
        }
    }
    fn label(&self) -> &str {
        "fail-second"
    }
}
#[tokio::test]
async fn before_tool_deny_or_fatal_preserves_completed_mutation_evidence() {
    for deny in [false, true] {
        let workspace = tempfile::tempdir().unwrap();
        let mut config = Config::resolve(&nomi_config::config::CliArgs {
            provider: Some("openai".into()),
            api_key: Some("test-only".into()),
            base_url: None,
            model: Some("test".into()),
            max_tokens: None,
            max_turns: Some(3),
            system_prompt: Some("base".into()),
            profile: None,
            project_dir: Some(workspace.path().to_path_buf()),
        })
        .unwrap();
        config.session.enabled = false;
        config.compact.enabled = false;
        config.hooks = Default::default();
        let root = workspace.path().canonicalize().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let executions = Arc::new(AtomicUsize::new(0));
        let mut tools = ToolRegistry::new();
        for name in ["Write", "second"] {
            tools.register(Box::new(Probe {
                name,
                calls: executions.clone(),
            }));
        }
        let mut engine = AgentEngine::new_with_provider(
            Arc::new(Provider {
                calls: requests.clone(),
                path: root.join("output.txt"),
            }),
            config,
            tools,
            Arc::new(crate::output::null_sink::NullSink),
            root.clone(),
        );
        engine.register_tool_middleware(Arc::new(FailSecond { deny }));
        let content = vec![ContentBlock::Text {
            text: "run both probes".into(),
        }];
        let mut context = CompletionEvidenceContext::new(content.clone());
        let result = engine
            .execute_turn_with_completion_evidence_context(
                content,
                "hook-turn",
                "hook-turn",
                None,
                Some(&mut context),
                None,
            )
            .await;
        if deny {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(matches!(result, Err(AgentError::ToolMiddleware(_))));
        }
        assert_eq!(requests.load(Ordering::SeqCst), if deny { 2 } else { 1 });
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read_to_string(root.join("output.txt")).unwrap(),
            "completed"
        );
        assert_eq!(context.terminal_exact_receipts, ["output.txt"]);
        assert_eq!(context.prior_durable_effect_targets, ["output.txt"]);
        assert!(engine.messages.iter().flat_map(|m| &m.content).any(|block| matches!(block, ContentBlock::ToolResult {tool_use_id, content, is_error:false, ..} if tool_use_id=="Write" && content=="completed original result")));
    }
}

use super::*;
use crate::model_middleware::{BeforeModelInput, ModelRequestMiddleware, ModelRequestPatch};

struct Replace;
#[async_trait::async_trait]
impl ModelRequestMiddleware for Replace {
    async fn before_model(&self, input: BeforeModelInput) -> Result<ModelRequestPatch, String> {
        assert!(
            !input
                .system
                .contains(&plan_prompt::plan_mode_instructions())
        );
        Ok(ModelRequestPatch {
            system: Some("replacement".into()),
            tool_names: Some(Vec::new()),
        })
    }
    fn label(&self) -> &str {
        "replace"
    }
}

struct Capture(Arc<Mutex<Vec<LlmRequest>>>);
#[async_trait::async_trait]
impl LlmProvider for Capture {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        self.0.lock().unwrap().push(request.clone());
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.try_send(LlmEvent::TextDelta("ok".into())).unwrap();
        tx.try_send(LlmEvent::Done {
            stop_reason: StopReason::EndTurn,
            usage: Default::default(),
        })
        .unwrap();
        Ok(rx)
    }
}

#[tokio::test]
async fn before_model_preserves_trusted_plan_and_resource_rules_and_no_middleware_prompt() {
    let workspace = tempfile::tempdir().unwrap();
    let mut config = Config::resolve(&nomi_config::config::CliArgs {
        provider: Some("openai".into()),
        api_key: Some("test-only".into()),
        base_url: None,
        model: Some("test".into()),
        max_tokens: None,
        max_turns: Some(1),
        system_prompt: Some("base".into()),
        profile: None,
        project_dir: Some(workspace.path().to_path_buf()),
    })
    .unwrap();
    config.session.enabled = false;
    config.compact.enabled = false;
    config.hooks = Default::default();
    for enabled in [false, true] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut engine = AgentEngine::new_with_provider(
            Arc::new(Capture(requests.clone())),
            config.clone(),
            ToolRegistry::new(),
            Arc::new(crate::output::null_sink::NullSink),
            workspace.path().to_path_buf(),
        );
        engine.system_prompt = "base".into();
        engine.plan_state.is_active = true;
        engine.set_system_resource_inbox(Some(Arc::new(Mutex::new(
            std::collections::VecDeque::from(["host-resource-notice".to_owned()]),
        ))));
        if enabled {
            engine.register_model_middleware(Arc::new(Replace));
        }
        engine.execute_turn("question", "message").await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let expected_start = format!(
            "{}\n\n{}",
            if enabled { "replacement" } else { "base" },
            plan_prompt::plan_mode_instructions()
        );
        assert!(
            requests[0].system.starts_with(&expected_start),
            "{}",
            requests[0].system
        );
        assert!(requests[0].system.contains("host-resource-notice"));
        assert_eq!(engine.system_prompt, "base");
        assert!(engine.plan_state.is_active);
    }
}

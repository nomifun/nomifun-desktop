use super::*;
use nomi_agent::{
    engine::AgentEngine,
    model_middleware::{BeforeModelInput, ModelRequestMiddleware, ModelRequestPatch},
};
use nomi_config::config::{Config, ProviderType};
use nomi_providers::{LlmProvider, ProviderError};
use nomi_types::{
    llm::{LlmEvent, LlmRequest},
    message::StopReason,
};
use nomifun_ai_agent::model_middleware as contract;

struct Product {
    expected: nomifun_agent_contracts::ResolvedCapability,
    inputs: Mutex<Vec<Value>>,
    finished: Arc<AtomicUsize>,
    release: tokio::sync::Notify,
}

struct DropEvidence(Arc<AtomicUsize>);
impl Drop for DropEvidence {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl NomiPluginProductToolInvoker for Product {
    async fn invoke(
        &self,
        request: NomiPluginProductToolInvocation,
    ) -> Result<StrictJsonValue, NomiPluginToolError> {
        assert_eq!(request.capability(), &self.expected);
        assert_eq!(request.action(), &contract::action());
        assert_eq!(
            request.operation_id().as_ref(),
            request.idempotency_key().as_ref()
        );
        assert_eq!(
            request.operation_id().as_ref(),
            request.correlation_id().as_ref()
        );
        let input = &request.input().0;
        assert_eq!(input["phase"], "before_model");
        assert_eq!(input.as_object().unwrap().len(), 4);
        for tool in input["tools"].as_array().unwrap() {
            assert_eq!(tool.as_object().unwrap().len(), 2);
            assert!(tool.get("name").is_some());
            assert!(tool.get("description").is_some());
        }
        self.inputs.lock().unwrap().push(input.clone());
        let text = input["turn"]["text"].as_str().unwrap();
        let value = match text {
            "outside" => json!({"system":"must not commit", "tool_names":["Missing"]}),
            "duplicate" => json!({"tool_names":["Allowed", "Allowed"]}),
            "extra" => json!({"engine":{"authority":"all"}}),
            "oversize" => json!({"system":"x".repeat(65 * 1024)}),
            "reject" => {
                return Err(NomiPluginToolError::Contract(
                    "private product diagnostic".into(),
                ));
            }
            "wait" => {
                let _evidence = DropEvidence(self.finished.clone());
                self.release.notified().await;
                json!({})
            }
            _ => json!({"system":format!("transformed:{text}"), "tool_names":["Allowed"]}),
        };
        Ok(StrictJsonValue(value))
    }
}

struct Candidate {
    name: &'static str,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl Tool for Candidate {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "candidate"
    }
    fn input_schema(&self) -> JsonSchema {
        json!({"type":"object"})
    }
    fn is_concurrency_safe(&self, _: &Value) -> bool {
        true
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
    async fn execute(&self, _: Value) -> ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ToolResult::text("observed")
    }
}

struct Capture {
    requests: Mutex<Vec<LlmRequest>>,
    attack: bool,
}
#[async_trait]
impl LlmProvider for Capture {
    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
        let mut requests = self.requests.lock().unwrap();
        let first = requests.is_empty();
        requests.push(request.clone());
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        if first {
            tx.try_send(LlmEvent::ToolUse {
                id: "test-call".into(),
                name: if self.attack { "Blocked" } else { "Allowed" }.into(),
                input: json!({}),
                extra: None,
            })
            .unwrap();
            tx.try_send(LlmEvent::Done {
                stop_reason: StopReason::ToolUse,
                usage: Default::default(),
            })
            .unwrap();
        } else {
            tx.try_send(LlmEvent::TextDelta("ok".into())).unwrap();
            tx.try_send(LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            })
            .unwrap();
        }
        Ok(rx)
    }
}

fn engine(
    workspace: &std::path::Path,
    capture: Arc<Capture>,
    calls: Arc<AtomicUsize>,
) -> AgentEngine {
    let mut config = Config {
        provider_label: "test".into(),
        provider: ProviderType::OpenAI,
        api_key: "test".into(),
        base_url: "http://localhost:0".into(),
        model: "test".into(),
        output_max_tokens: Some(1024),
        max_turns: Some(3),
        system_prompt: Some("base".into()),
        project_instructions: Default::default(),
        thinking: None,
        prompt_caching: false,
        compat: nomi_config::compat::ProviderCompat::openai_defaults(),
        tools: Default::default(),
        session: Default::default(),
        compact: Default::default(),
        plan: Default::default(),
        file_cache: Default::default(),
        hooks: Default::default(),
        bedrock: None,
        vertex: None,
        mcp: Default::default(),
        logging: Default::default(),
    };
    config.session.enabled = false;
    config.compact.enabled = false;
    let mut tools = ToolRegistry::new();
    for name in ["Allowed", "Blocked"] {
        tools.register(Box::new(Candidate {
            name,
            calls: calls.clone(),
        }));
    }
    AgentEngine::new_with_provider(
        capture,
        config,
        tools,
        Arc::new(nomi_agent::output::null_sink::NullSink),
        workspace.to_path_buf(),
    )
}

async fn setup() -> (nomifun_ai_agent::NomiPluginToolSession, Arc<Product>) {
    let (mut capability, _) = plugin_product_fixture();
    capability.actions = vec![contract::action()];
    let compiled = compile_plugin_product_fixture(&capability);
    let product = Arc::new(Product {
        expected: compiled.content().contributions().next().unwrap().clone(),
        inputs: Mutex::new(Vec::new()),
        finished: Arc::new(AtomicUsize::new(0)),
        release: tokio::sync::Notify::new(),
    });
    let kernel = Arc::new(
        KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
    );
    let restricted = restricted_session(kernel.clone(), compiled.clone(), Arc::new(SchemaMap::default())).await;
    assert!(restricted.model_middleware().unwrap().is_empty());
    restricted.register_into(&mut ToolRegistry::new()).unwrap();
    assert!(product.inputs.lock().unwrap().is_empty());
    let base = KernelNomiPluginToolSession::materialize(
        kernel,
        Arc::new(compiled.clone()),
        owner(),
        SESSION.into(),
        format!("session:{SESSION}").into(),
        Arc::new(SchemaMap::default()),
    )
    .await
    .unwrap();
    assert!(base.register_into(&mut ToolRegistry::new()).is_err());
    assert!(base.model_middleware().is_err());
    assert!(
        base.clone()
            .bind_hosted_execution(product_bindings(Vec::new(), product.clone()))
            .is_err()
    );
    let schemas = contract::schemas();
    let mut invalid_schemas = schemas.clone();
    invalid_schemas.insert(
        contract::action().output_schema,
        StrictJsonValue(json!({"type":"string"})),
    );
    assert!(
        KernelNomiPluginToolSession::materialize_plugin_product_actions(
            &compiled,
            &owner(),
            &SESSION.into(),
            &format!("session:{SESSION}").into(),
            Arc::new(PluginProductSchemaMap {
                schemas: invalid_schemas
            })
        )
        .await
        .is_err()
    );
    let actions = KernelNomiPluginToolSession::materialize_plugin_product_actions(
        &compiled,
        &owner(),
        &SESSION.into(),
        &format!("session:{SESSION}").into(),
        Arc::new(PluginProductSchemaMap { schemas }),
    )
    .await
    .unwrap();
    let loaded = base
        .bind_hosted_execution(product_bindings(actions, product.clone()))
        .unwrap();
    let mut tools = ToolRegistry::new();
    loaded.register_into(&mut tools).unwrap();
    assert!(
        tools.tool_names().is_empty(),
        "middleware must not be a model Tool"
    );
    (loaded, product)
}

#[tokio::test]
async fn product_before_model_changes_real_requests_without_mutating_history_or_authority() {
    let (loaded, product) = setup().await;
    let capture = Arc::new(Capture {
        requests: Mutex::new(Vec::new()),
        attack: false,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let workspace = tempfile::tempdir().unwrap();
    let mut engine = engine(workspace.path(), capture.clone(), calls.clone());
    for middleware in loaded.model_middleware().unwrap() {
        engine.register_model_middleware(middleware);
    }
    engine.execute_turn("first", "message-first").await.unwrap();
    engine
        .execute_turn("second", "message-second")
        .await
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the offered tool still executes through the original owner"
    );
    let requests = capture.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        3,
        "middleware must run for every provider pass, not only once per turn"
    );
    for (index, request) in requests.iter().enumerate() {
        assert!(request.system.contains(if index < 2 {
            "transformed:first"
        } else {
            "transformed:second"
        }));
        assert_eq!(
            request
                .tools
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Allowed"]
        );
        assert!(
            !serde_json::to_string(&request.messages)
                .unwrap()
                .contains("transformed:")
        );
    }
    let inputs = product.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 3);
    assert!(
        inputs.iter().all(|input| input["system"] == "base"),
        "request patches must not persist in source prompt"
    );
}

#[tokio::test]
async fn before_model_hidden_tool_is_rejected_even_if_provider_requests_it() {
    let (loaded, _) = setup().await;
    let capture = Arc::new(Capture {
        requests: Mutex::new(Vec::new()),
        attack: true,
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let workspace = tempfile::tempdir().unwrap();
    let mut engine = engine(workspace.path(), capture.clone(), calls.clone());
    for middleware in loaded.model_middleware().unwrap() {
        engine.register_model_middleware(middleware);
    }
    let error = engine
        .execute_turn("attack", "attack")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("not advertised"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(capture.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn product_before_model_failure_blocks_provider_and_timeout_retains_invocation() {
    let (loaded, product) = setup().await;
    let workspace = tempfile::tempdir().unwrap();
    for text in [
        "outside",
        "duplicate",
        "extra",
        "oversize",
        "reject",
        "wait",
    ] {
        let capture = Arc::new(Capture {
            requests: Mutex::new(Vec::new()),
            attack: false,
        });
        let mut engine = engine(
            workspace.path(),
            capture.clone(),
            Arc::new(AtomicUsize::new(0)),
        );
        for middleware in loaded.model_middleware().unwrap() {
            engine.register_model_middleware(middleware);
        }
        let error = engine
            .execute_turn(text, text)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private product diagnostic"));
        assert!(
            capture.requests.lock().unwrap().is_empty(),
            "{text}: {error}"
        );
    }
    settle_retained_wait(&loaded, &product).await;
}

async fn settle_retained_wait(loaded: &nomifun_ai_agent::NomiPluginToolSession, product: &Product) {
    let scope = loaded.effect_scope().unwrap();
    assert_eq!(product.finished.load(Ordering::SeqCst), 0,
        "caller cancellation must not discard the hosted operation");
    assert!(scope.ensure_turn_open().is_err(), "cancellation must close further dispatch");
    assert!(scope.begin_turn().is_err(), "a pending operation is not proof of settlement");
    product.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), scope.settle_turn()).await.unwrap().unwrap();
    assert_eq!(product.finished.load(Ordering::SeqCst), 1,
        "settlement must await the original operation, not spawn a retry");
    scope.begin_turn().unwrap();
    scope.settle_turn().await.unwrap();
}

struct Filter {
    name: &'static str,
    seen: Arc<Mutex<Vec<Vec<String>>>>,
}

#[tokio::test]
async fn dropping_before_model_turn_retains_the_product_invocation_without_provider_request() {
    let (loaded, product) = setup().await;
    let capture = Arc::new(Capture {
        requests: Mutex::new(Vec::new()),
        attack: false,
    });
    let workspace = tempfile::tempdir().unwrap();
    let mut engine = engine(
        workspace.path(),
        capture.clone(),
        Arc::new(AtomicUsize::new(0)),
    );
    for middleware in loaded.model_middleware().unwrap() {
        engine.register_model_middleware(middleware);
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            engine.execute_turn("wait", "cancelled-message")
        )
        .await
        .is_err()
    );
    settle_retained_wait(&loaded, &product).await;
    assert_eq!(product.inputs.lock().unwrap().len(), 1);
    assert!(capture.requests.lock().unwrap().is_empty());
}
#[async_trait]
impl ModelRequestMiddleware for Filter {
    async fn before_model(&self, input: BeforeModelInput) -> Result<ModelRequestPatch, String> {
        self.seen
            .lock()
            .unwrap()
            .push(input.tools.iter().map(|t| t.name.clone()).collect());
        Ok(ModelRequestPatch {
            system: None,
            tool_names: Some(vec![self.name.into()]),
        })
    }
    fn label(&self) -> &str {
        self.name
    }
}

#[tokio::test]
async fn before_model_chain_cannot_reintroduce_tools_removed_by_an_earlier_stage() {
    let capture = Arc::new(Capture {
        requests: Mutex::new(Vec::new()),
        attack: false,
    });
    let workspace = tempfile::tempdir().unwrap();
    let mut engine = engine(
        workspace.path(),
        capture.clone(),
        Arc::new(AtomicUsize::new(0)),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    for name in ["Allowed", "Blocked"] {
        engine.register_model_middleware(Arc::new(Filter {
            name,
            seen: seen.clone(),
        }));
    }
    assert!(engine.execute_turn("question", "message").await.is_err());
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            vec!["Allowed".to_owned(), "Blocked".to_owned()],
            vec!["Allowed".to_owned()]
        ]
    );
    assert!(capture.requests.lock().unwrap().is_empty());
}

//! Atomic hosted binding must retain Hidden calls and fence unknown effects.
use super::*;
use nomifun_ai_agent::{NomiPluginToolSession, engine_effect_scope::EngineEffectScope};
use tokio::sync::Notify;

#[derive(Default)]
struct ControlledProduct {
    calls: AtomicUsize,
    finished: Arc<AtomicUsize>,
    entered: Notify,
    release: Notify,
    unknown: bool,
}

struct Finished(Arc<AtomicUsize>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl NomiPluginProductToolInvoker for ControlledProduct {
    async fn invoke(&self, request: NomiPluginProductToolInvocation) -> Result<StrictJsonValue, NomiPluginToolError> {
        let _finished = Finished(self.finished.clone());
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        self.release.notified().await;
        if self.unknown {
            return Err(NomiPluginToolError::OutcomeUnknown("test owner outcome unknown".into()));
        }
        Ok(StrictJsonValue(if request.action() == &nomifun_ai_agent::model_middleware::action() {
            json!({})
        } else {
            assert_eq!(request.action(), &nomifun_ai_agent::tool_discovery::action());
            json!({"names":[]})
        }))
    }
}

async fn setup(
    middleware: bool,
    product: Arc<ControlledProduct>,
) -> (NomiPluginToolSession, Arc<EngineEffectScope>) {
    let (mut capability, _) = plugin_product_fixture();
    let (action, schemas) = if middleware {
        (nomifun_ai_agent::model_middleware::action(), nomifun_ai_agent::model_middleware::schemas())
    } else {
        (nomifun_ai_agent::tool_discovery::action(), nomifun_ai_agent::tool_discovery::schemas())
    };
    capability.actions = vec![action];
    let compiled = compile_plugin_product_fixture(&capability);
    let actions = KernelNomiPluginToolSession::materialize_plugin_product_actions(
        &compiled, &owner(), &SESSION.into(), &format!("session:{SESSION}").into(),
        Arc::new(PluginProductSchemaMap { schemas }),
    ).await.unwrap();
    let kernel = Arc::new(KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap());
    let base = KernelNomiPluginToolSession::materialize(
        kernel, Arc::new(compiled), owner(), SESSION.into(), format!("session:{SESSION}").into(),
        Arc::new(SchemaMap::default()),
    ).await.unwrap();
    let scope = Arc::new(EngineEffectScope::new(Vec::new()).unwrap());
    let loaded = base.bind_hosted_execution(nomifun_ai_agent::NomiHostedSessionBindings {
        product: Some((actions, product)),
        ..nomifun_ai_agent::NomiHostedSessionBindings::new(scope.clone())
    }).unwrap();
    assert!(Arc::ptr_eq(&loaded.effect_scope().unwrap(), &scope));
    assert!(loaded.plugin_product_actions().is_empty(), "Hidden actions must not become model tools");
    (loaded, scope)
}

async fn invoke_hidden(loaded: &NomiPluginToolSession, middleware: bool) -> bool {
    if middleware {
        loaded.model_middleware().unwrap()[0].before_model(nomi_agent::model_middleware::BeforeModelInput {
            phase: "before_model",
            turn: nomi_agent::context_contributor::TurnContext {
                turn_id: "hosted-hidden-turn".into(),
                source_message_id: "hosted-hidden-source".into(), text: "question".into(), ..Default::default()
            },
            system: "base".into(), tools: Vec::new(),
        }).await.is_ok()
    } else {
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(nomi_tools::tool_search::ToolSearchTool::new(tools.deferred_state())));
        loaded.register_into(&mut tools).unwrap();
        !tools.get("ToolSearch").unwrap().execute(json!({"query":"question"})).await.is_error
    }
}

#[tokio::test]
async fn hidden_consumers_retain_cancelled_calls_after_atomic_binding() {
    for middleware in [true, false] {
        {
            let product = Arc::new(ControlledProduct::default());
            let (loaded, scope) = setup(middleware, product.clone()).await;
            scope.begin_turn().unwrap();
            let mut waiter = Box::pin(invoke_hidden(&loaded, middleware));
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::select! {
                    _ = product.entered.notified() => {},
                    result = &mut waiter => panic!("call returned before owner release: {result}"),
                }
            }).await.expect("Hidden consumer did not reach its owner");
            drop(waiter);
            assert_eq!(product.finished.load(Ordering::SeqCst), 0, "caller cancellation must retain owner work");
            assert!(scope.ensure_turn_open().is_err(), "cancelled Hidden waiter must close dispatch");
            assert!(!invoke_hidden(&loaded, middleware).await);
            assert_eq!(product.calls.load(Ordering::SeqCst), 1);
            assert!(scope.begin_turn().is_err(), "unsettled owner cannot admit a new turn");

            product.release.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(2), scope.settle_turn()).await.unwrap().unwrap();
            assert_eq!(product.finished.load(Ordering::SeqCst), 1);
            // A proved settlement permits reuse; no second scheduling path.
            scope.begin_turn().unwrap();
            product.release.notify_one();
            assert!(invoke_hidden(&loaded, middleware).await);
            scope.settle_turn().await.unwrap();
            assert_eq!(product.calls.load(Ordering::SeqCst), 2);
        }
    }
}

#[tokio::test]
async fn hidden_consumers_fence_unknown_outcomes_after_atomic_binding() {
    for middleware in [true, false] {
        {
            let product = Arc::new(ControlledProduct { unknown: true, ..Default::default() });
            let (loaded, scope) = setup(middleware, product.clone()).await;
            scope.begin_turn().unwrap();
            product.release.notify_one();
            assert!(!invoke_hidden(&loaded, middleware).await);
            assert!(scope.ensure_turn_open().is_err(), "unknown outcome must retire the same scope");
            assert!(!invoke_hidden(&loaded, middleware).await);
            assert_eq!(product.calls.load(Ordering::SeqCst), 1);
            scope.settle_turn().await.unwrap();
            assert!(scope.begin_turn().is_err(), "task completion cannot erase unknown outcome");
        }
    }
}

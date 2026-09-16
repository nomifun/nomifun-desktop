use super::*;
use async_trait::async_trait;
use nomifun_agent_contracts::{PluginDependencyCall, PluginHostResponseBody, PluginHostSuccess};
use nomifun_js_host::{ExtensionHostDemandPort, ExtensionHostDependencyCaller};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
struct Calls {
    entered: AtomicUsize,
    dropped: AtomicUsize,
    started: Notify,
    released: Notify,
}

struct DropCall(Arc<Calls>);
impl Drop for DropCall {
    fn drop(&mut self) {
        self.0.dropped.fetch_add(1, Ordering::SeqCst);
        self.0.released.notify_one();
    }
}

struct Caller {
    calls: Arc<Calls>,
    pause: bool,
}
#[async_trait]
impl ExtensionHostDependencyCaller for Caller {
    async fn invoke(&self, call: PluginDependencyCall) -> PluginHostResponseBody {
        let _guard = DropCall(self.calls.clone());
        self.calls.entered.fetch_add(1, Ordering::SeqCst);
        self.calls.started.notify_one();
        if self.pause {
            std::future::pending::<()>().await;
        }
        PluginHostResponseBody::Success(PluginHostSuccess::Value(StrictJsonValue(json!({
            "target": call.capability_id, "action": call.action_id,
            "key": call.call_key, "input": call.input,
        }))))
    }
}

fn input() -> StrictJsonValue {
    StrictJsonValue(
        json!({"capabilityId":"fixture.child","actionId":"run","callKey":"one","input":{"value":7}}),
    )
}

async fn demand(temp: &TempDir) -> MountLoadDemand {
    let context = context(temp.path(), "mount-a", 'a');
    MountLoadDemand {
        module: module(context.target.clone()).await,
        context,
    }
}

fn context_input(
    action: &str,
    input: StrictJsonValue,
) -> nomifun_agent_contracts::ContextContributionInput {
    nomifun_agent_contracts::ContextContributionInput::BeforeTurn {
        turn: nomifun_agent_contracts::ContextTurnInput {
            source_message_id: "context-parent".into(),
            text: json!({"action": action, "input": input}).to_string(),
            image_media_types: Vec::new(),
        },
    }
}

#[tokio::test]
async fn context_dependencies_close_on_success_failure_and_abandonment() {
    let host = Arc::new(supervisor(Duration::from_secs(5)).await);
    let temp = TempDir::new().unwrap();
    let demand = demand(&temp).await;
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    let caller = || {
        Some(Arc::new(Caller {
            calls: calls.clone(),
            pause: false,
        }) as Arc<dyn ExtensionHostDependencyCaller>)
    };
    let result = host
        .contribute_context_demand(
            demand.clone(),
            reference.clone(),
            "schema://fixture/dependencies".into(),
            context_input("dependency", input()),
            caller(),
        )
        .await
        .unwrap();
    assert_eq!(result.0["input"]["value"], 7);
    let result = host
        .contribute_context_demand(
            demand.clone(),
            reference.clone(),
            "schema://fixture/dependencies".into(),
            context_input("dependency", input()),
            None,
        )
        .await;
    assert!(
        matches!(result, Err(JavaScriptHostError::RequestFailed { message, .. }) if message.contains("DEPENDENCY_PARENT_CLOSED"))
    );
    for reject in [false, true] {
        let result = host
            .contribute_context_demand(
                demand.clone(),
                reference.clone(),
                "schema://fixture/dependencies".into(),
                context_input(
                    "retain_dependency",
                    StrictJsonValue(json!({"reject":reject})),
                ),
                caller(),
            )
            .await;
        assert_eq!(result.is_err(), reject);
        let result = host
            .contribute_context_demand(
                demand.clone(),
                reference.clone(),
                "schema://fixture/dependencies".into(),
                context_input("late_dependency", input()),
                caller(),
            )
            .await
            .unwrap();
        assert!(
            result.0["error"]
                .as_str()
                .unwrap()
                .contains("DEPENDENCY_PARENT_CLOSED")
        );
    }
    assert_eq!(calls.entered.load(Ordering::SeqCst), 1);
    let paused = Arc::new(Calls::default());
    let pending = tokio::spawn({
        let host = host.clone();
        let demand = demand.clone();
        let reference = reference.clone();
        let calls = paused.clone();
        async move {
            host.contribute_context_demand(
                demand,
                reference,
                "schema://fixture/dependencies".into(),
                context_input("dependency", input()),
                Some(Arc::new(Caller { calls, pause: true })),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), paused.started.notified())
        .await
        .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), paused.released.notified())
        .await
        .unwrap();
    assert_eq!(paused.dropped.load(Ordering::SeqCst), 1);

    // A Context may finish without awaiting a child; Host still owns the child.
    let detached = Arc::new(Calls::default());
    host.contribute_context_demand(
        demand,
        reference.clone(),
        "schema://fixture/dependencies".into(),
        context_input("detach_dependency", input()),
        Some(Arc::new(Caller {
            calls: detached.clone(),
            pause: true,
        })),
    )
    .await
    .unwrap();
    let result = host
        .invoke(
            reference,
            "await_service".into(),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert!(
        result.0["error"]
            .as_str()
            .unwrap()
            .contains("DEPENDENCY_PARENT_CLOSED")
    );
    assert_eq!(
        detached.entered.load(Ordering::SeqCst),
        detached.dropped.load(Ordering::SeqCst)
    );
    let generation = match host.state() {
        JavaScriptHostState::Running { generation, .. } => generation,
        _ => panic!(),
    };
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn dependencies_are_request_scoped_and_plain_invocations_have_no_caller() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let demand = demand(&temp).await;
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    let value = host
        .invoke_with_dependencies(
            demand.clone(),
            reference.clone(),
            "dependency".into(),
            input(),
            Arc::new(Caller {
                calls: calls.clone(),
                pause: false,
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        value.0,
        json!({"target":"fixture.child","action":"run","key":"one","input":{"value":7}})
    );
    let rejected = host
        .invoke_demand(demand, reference.clone(), "dependency".into(), input())
        .await;
    assert!(
        matches!(rejected, Err(JavaScriptHostError::RequestFailed { message, .. }) if message.contains("DEPENDENCY_PARENT_CLOSED"))
    );
    assert_eq!(calls.entered.load(Ordering::SeqCst), 1);
    let generation = match host.state() {
        JavaScriptHostState::Running { generation, .. } => generation,
        _ => panic!(),
    };
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn retained_sdk_closures_cannot_rebind_after_success_or_failure() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let demand = demand(&temp).await;
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    for reject in [false, true] {
        let result = host
            .invoke_with_dependencies(
                demand.clone(),
                reference.clone(),
                "retain_dependency".into(),
                StrictJsonValue(json!({"reject":reject})),
                Arc::new(Caller {
                    calls: calls.clone(),
                    pause: false,
                }),
            )
            .await;
        assert_eq!(result.is_err(), reject);
        let result = host
            .invoke_with_dependencies(
                demand.clone(),
                reference.clone(),
                "late_dependency".into(),
                input(),
                Arc::new(Caller {
                    calls: calls.clone(),
                    pause: false,
                }),
            )
            .await
            .unwrap();
        assert!(
            result.0["error"]
                .as_str()
                .unwrap()
                .contains("DEPENDENCY_PARENT_CLOSED")
        );
    }
    assert_eq!(calls.entered.load(Ordering::SeqCst), 0);
    let generation = match host.state() {
        JavaScriptHostState::Running { generation, .. } => generation,
        _ => panic!(),
    };
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn abandoning_parent_drops_its_child_and_keeps_the_generation_usable() {
    let host = Arc::new(supervisor(Duration::from_secs(5)).await);
    let temp = TempDir::new().unwrap();
    let demand = demand(&temp).await;
    // Measure abandoning an admitted invocation, not lazy Node startup. Lazy
    // demand admission is covered by dependencies_are_request_scoped above.
    host.load_mount(demand.clone()).await.unwrap();
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    let pending = tokio::spawn({
        let host = host.clone();
        let calls = calls.clone();
        let reference = reference.clone();
        async move {
            host.invoke_with_dependencies(
                demand,
                reference,
                "dependency".into(),
                input(),
                Arc::new(Caller { calls, pause: true }),
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(3), calls.started.notified())
        .await
        .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), calls.released.notified())
        .await
        .unwrap();
    assert_eq!(calls.dropped.load(Ordering::SeqCst), 1);
    host.invoke(reference, "echo".into(), StrictJsonValue(json!({})))
        .await
        .unwrap();
    let generation = match host.state() {
        JavaScriptHostState::Running { generation, .. } => generation,
        _ => panic!(),
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match host.stop_generation(generation).await {
                Ok(_) => break,
                Err(JavaScriptHostError::NotQuiescent { .. }) => tokio::task::yield_now().await,
                Err(error) => panic!("{error}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn parent_completion_cancels_unawaited_dependency_work() {
    let host = supervisor(Duration::from_secs(5)).await;
    let temp = TempDir::new().unwrap();
    let demand = demand(&temp).await;
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    host.invoke_with_dependencies(
        demand,
        reference.clone(),
        "detach_dependency".into(),
        input(),
        Arc::new(Caller {
            calls: calls.clone(),
            pause: true,
        }),
    )
    .await
    .unwrap();
    let result = host
        .invoke(
            reference,
            "await_service".into(),
            StrictJsonValue(json!({})),
        )
        .await
        .unwrap();
    assert!(
        result.0["error"]
            .as_str()
            .unwrap()
            .contains("DEPENDENCY_PARENT_CLOSED")
    );
    assert_eq!(
        calls.entered.load(Ordering::SeqCst),
        calls.dropped.load(Ordering::SeqCst)
    );
    let generation = match host.state() {
        JavaScriptHostState::Running { generation, .. } => generation,
        _ => panic!(),
    };
    host.stop_generation(generation).await.unwrap();
}

#[tokio::test]
async fn wire_parent_identity_is_checked_even_when_the_js_sdk_is_bypassed() {
    run_wire_parent_identity(false).await;
}

#[tokio::test]
async fn wire_context_parent_identity_is_checked_even_when_the_js_sdk_is_bypassed() {
    run_wire_parent_identity(true).await;
}

async fn run_wire_parent_identity(context: bool) {
    let host = supervisor_with_host(
        Duration::from_secs(5),
        fixture("dependency-protocol-host.mjs")
            .canonicalize()
            .unwrap(),
    )
    .await;
    let temp = TempDir::new().unwrap();
    load(&host, &temp, "mount-b", 'b').await;
    let demand = demand(&temp).await;
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    for mode in ["valid", "stale-parent", "unknown-parent", "wrong-mount"] {
        let result = if context {
            host.contribute_context_demand(
                demand.clone(),
                reference.clone(),
                "schema://fixture/dependencies".into(),
                context_input("relay", StrictJsonValue(json!({"mode":mode}))),
                Some(Arc::new(Caller {
                    calls: calls.clone(),
                    pause: false,
                })),
            )
            .await
        } else {
            host.invoke_with_dependencies(
                demand.clone(),
                reference.clone(),
                "relay".into(),
                StrictJsonValue(json!({"mode":mode})),
                Arc::new(Caller {
                    calls: calls.clone(),
                    pause: false,
                }),
            )
            .await
        }
        .unwrap();
        if mode == "valid" {
            assert_eq!(result.0["outcome"], "success");
        } else {
            assert_eq!(
                result.0["value"]["code"], "DEPENDENCY_PARENT_CLOSED",
                "{mode}: {result:?}"
            );
        }
    }
    assert_eq!(calls.entered.load(Ordering::SeqCst), 1);
    let result = if context {
        host.contribute_context_demand(
            demand,
            reference,
            "schema://fixture/dependencies".into(),
            context_input("relay", StrictJsonValue(json!({"mode":"wrong-generation"}))),
            Some(Arc::new(Caller {
                calls: calls.clone(),
                pause: false,
            })),
        )
        .await
    } else {
        host.invoke_with_dependencies(
            demand,
            reference,
            "relay".into(),
            StrictJsonValue(json!({"mode":"wrong-generation"})),
            Arc::new(Caller {
                calls: calls.clone(),
                pause: false,
            }),
        )
        .await
    };
    assert!(result.is_err());
    wait_until_stopped(&host).await;
    assert_eq!(calls.entered.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dependency_callback_cannot_outlive_the_parent_watchdog_deadline() {
    run_watchdog_case(false).await;
}

#[tokio::test]
async fn context_dependency_callback_cannot_outlive_the_parent_watchdog_deadline() {
    run_watchdog_case(true).await;
}

async fn run_watchdog_case(context: bool) {
    let host = supervisor(Duration::from_millis(500)).await;
    let temp = TempDir::new().unwrap();
    let demand = demand(&temp).await;
    // The parent watchdog starts with the invocation, after Host hello and
    // activation. Keep their independent budgets outside this measurement.
    host.load_mount(demand.clone()).await.unwrap();
    let reference = contribution(demand.context.target.clone());
    let calls = Arc::new(Calls::default());
    let result = tokio::time::timeout(Duration::from_secs(4), async {
        if context {
            return host
                .contribute_context_demand(
                    demand,
                    reference,
                    "schema://fixture/dependencies".into(),
                    context_input("dependency", input()),
                    Some(Arc::new(Caller {
                        calls: calls.clone(),
                        pause: true,
                    })),
                )
                .await;
        }
        host.invoke_with_dependencies(
            demand,
            reference,
            "dependency".into(),
            input(),
            Arc::new(Caller {
                calls: calls.clone(),
                pause: true,
            }),
        )
        .await
    })
    .await
    .unwrap();
    assert!(result.is_err());
    wait_until_stopped(&host).await;
    assert_eq!(calls.entered.load(Ordering::SeqCst), 1);
    assert_eq!(calls.dropped.load(Ordering::SeqCst), 1);
}

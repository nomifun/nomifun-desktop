use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_plugin_platform::{
    AgentPluginBindings, AutomationPluginBindings, BindingFailureSemantics,
    BindingInvocationAdapter, BindingMultiplicity, BindingPointContract, DesktopPluginBindings,
    InMemoryPluginBindingRegistry, PassthroughBindingAdapter, PluginActionAvailability,
    PluginActionCallError, PluginActionInvocation, PluginActionRegistration, PluginCancellation,
    PluginActionRuntimePort, PluginActionUnavailableReason, PluginBindingError,
    PluginDispatchOptions,
};
use nomifun_agent_contracts::{
    DigestHex, PluginActionEffect, PluginActionManifest, PluginActionPublication,
    PluginArtifactRef, PluginBindingPoint, PluginId, StrictJsonValue,
};
use serde_json::json;
use tokio::sync::Notify;

fn digest(character: char) -> DigestHex {
    DigestHex::from(character.to_string().repeat(64))
}

fn plugin(number: u8) -> PluginId {
    PluginId::from(format!(
        "019b0000-0000-7000-8000-{number:012}"
    ))
}

fn publication(
    plugin_id: &PluginId,
    artifact_digest: &DigestHex,
    action_id: &str,
    points: impl IntoIterator<Item = PluginBindingPoint>,
) -> PluginActionPublication {
    PluginActionPublication {
        plugin_id: plugin_id.clone(),
        artifact: PluginArtifactRef {
            digest: artifact_digest.clone(),
            package_id: "local.binding-fixture".into(),
            version: "1.0.0".into(),
        },
        action_id: action_id.into(),
        action: PluginActionManifest {
            name: action_id.into(),
            description: format!("{action_id} test Action"),
            input: StrictJsonValue(json!({"type": "object"})),
            output: StrictJsonValue(json!({"type": "object"})),
            effect: PluginActionEffect::Read,
        },
        bindings: points.into_iter().collect(),
    }
}

fn registration(
    plugin_id: &PluginId,
    artifact_digest: &DigestHex,
    action_id: &str,
    points: impl IntoIterator<Item = PluginBindingPoint>,
) -> PluginActionRegistration {
    publication(plugin_id, artifact_digest, action_id, points).into()
}

fn owner(
    point: PluginBindingPoint,
    failure: BindingFailureSemantics,
    timeout: Duration,
) -> BindingPointContract {
    BindingPointContract {
        point,
        input_schema: StrictJsonValue(json!({"type": "object"})),
        output_schema: StrictJsonValue(json!({"type": "object"})),
        multiplicity: BindingMultiplicity::Multiple,
        failure,
        timeout,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct CapturedCall {
    stable_action_id: String,
    binding_point: Option<PluginBindingPoint>,
    artifact_digest: DigestHex,
    call_chain: Vec<String>,
}

#[derive(Default)]
struct RecordingRuntime {
    calls: Mutex<Vec<CapturedCall>>,
    failures: Mutex<HashSet<String>>,
}

impl RecordingRuntime {
    fn fail(&self, stable_action_id: &str) {
        self.failures
            .lock()
            .unwrap()
            .insert(stable_action_id.into());
    }

    fn calls(&self) -> Vec<CapturedCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl PluginActionRuntimePort for RecordingRuntime {
    async fn invoke(
        &self,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError> {
        self.calls.lock().unwrap().push(CapturedCall {
            stable_action_id: invocation.stable_action_id.clone(),
            binding_point: invocation.binding_point,
            artifact_digest: invocation.publication.artifact.digest.clone(),
            call_chain: invocation.call_chain.clone(),
        });
        if self
            .failures
            .lock()
            .unwrap()
            .contains(&invocation.stable_action_id)
        {
            return Err(PluginActionCallError::new("FIXTURE_FAILED", "fixture failure"));
        }
        Ok(StrictJsonValue(json!({
            "stable_action_id": invocation.stable_action_id,
            "input": invocation.input.0
        })))
    }
}

#[derive(Default)]
struct RecordingAdapter {
    points: Mutex<Vec<PluginBindingPoint>>,
}

#[async_trait]
impl BindingInvocationAdapter for RecordingAdapter {
    async fn invoke(
        &self,
        runtime: Arc<dyn PluginActionRuntimePort>,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError> {
        self.points
            .lock()
            .unwrap()
            .push(invocation.binding_point.expect("bound invocation"));
        runtime.invoke(invocation).await
    }
}

fn register_owner(
    registry: &InMemoryPluginBindingRegistry,
    point: PluginBindingPoint,
) {
    registry
        .register_binding_owner(
            owner(
                point,
                BindingFailureSemantics::FailClosed,
                Duration::from_secs(1),
            ),
            Arc::new(PassthroughBindingAdapter),
        )
        .unwrap();
}

#[tokio::test]
async fn replace_is_atomic_keeps_stable_identity_and_enforces_artifact_fence() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime.clone());
    register_owner(&registry, PluginBindingPoint::DesktopCommand);
    let plugin_id = plugin(1);
    let first_digest = digest('a');
    let second_digest = digest('b');
    let stable = format!("plugin:{}/run", plugin_id.as_ref());

    registry
        .replace_plugin(
            plugin_id.clone(),
            first_digest.clone(),
            true,
            vec![registration(
                &plugin_id,
                &first_digest,
                "run",
                [PluginBindingPoint::DesktopCommand],
            )],
        )
        .unwrap();
    let original = registry.resolve_action(&stable).unwrap();
    registry
        .dispatch_binding(
            PluginBindingPoint::DesktopCommand,
            &stable,
            StrictJsonValue(json!({})),
            PluginDispatchOptions {
                expected_artifact_digest: Some(first_digest.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    registry
        .replace_plugin(
            plugin_id.clone(),
            second_digest.clone(),
            true,
            vec![registration(
                &plugin_id,
                &second_digest,
                "run",
                [PluginBindingPoint::DesktopCommand],
            )],
        )
        .unwrap();
    let updated = registry.resolve_action(&stable).unwrap();
    assert_eq!(updated.stable_action_id, stable);
    assert_eq!(updated.order, original.order);
    assert_eq!(updated.publication.artifact.digest, second_digest);
    assert!(matches!(
        registry
            .dispatch_binding(
                PluginBindingPoint::DesktopCommand,
                &stable,
                StrictJsonValue(json!({})),
                PluginDispatchOptions {
                    expected_artifact_digest: Some(first_digest),
                    ..Default::default()
                },
            )
            .await,
        Err(PluginBindingError::ArtifactFence(_))
    ));

    let revision = registry.revision().unwrap();
    assert!(matches!(
        registry.replace_plugin(
            plugin_id.clone(),
            digest('c'),
            true,
            vec![registration(
                &plugin_id,
                &digest('c'),
                "broken",
                [PluginBindingPoint::AgentTool],
            )],
        ),
        Err(PluginBindingError::UnsupportedBinding(_))
    ));
    assert_eq!(registry.revision().unwrap(), revision);
    assert_eq!(
        registry.resolve_action(&stable).unwrap().publication.artifact.digest,
        digest('b'),
    );
    assert_eq!(runtime.calls().len(), 1);
}

#[tokio::test]
async fn removed_and_disabled_actions_remain_visible_as_unavailable() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime);
    register_owner(&registry, PluginBindingPoint::DesktopCommand);
    let desktop = DesktopPluginBindings::new(registry.clone());
    let plugin_id = plugin(2);
    let first_digest = digest('a');
    let stable = format!("plugin:{}/open", plugin_id.as_ref());
    registry
        .replace_plugin(
            plugin_id.clone(),
            first_digest.clone(),
            true,
            vec![registration(
                &plugin_id,
                &first_digest,
                "open",
                [PluginBindingPoint::DesktopCommand],
            )],
        )
        .unwrap();

    registry.set_plugin_enabled(&plugin_id, false).unwrap();
    assert_eq!(
        registry.resolve_action(&stable).unwrap().availability,
        PluginActionAvailability::Unavailable(PluginActionUnavailableReason::Disabled),
    );
    assert!(matches!(
        desktop.select(&stable),
        Err(PluginBindingError::ActionUnavailable {
            reason: PluginActionUnavailableReason::Disabled,
            ..
        })
    ));
    registry.set_plugin_enabled(&plugin_id, true).unwrap();

    registry
        .replace_plugin(plugin_id.clone(), digest('b'), true, Vec::new())
        .unwrap();
    let removed = registry.resolve_action(&stable).unwrap();
    assert_eq!(
        removed.availability,
        PluginActionAvailability::Unavailable(PluginActionUnavailableReason::ActionRemoved),
    );
    assert_eq!(desktop.commands().unwrap(), vec![removed.clone()]);

    registry
        .replace_plugin(
            plugin_id.clone(),
            digest('c'),
            true,
            vec![registration(
                &plugin_id,
                &digest('c'),
                "open",
                [PluginBindingPoint::DesktopCommand],
            )],
        )
        .unwrap();
    assert_eq!(
        registry.resolve_action(&stable).unwrap().availability,
        PluginActionAvailability::Available,
    );
    registry.remove_plugin(&plugin_id).unwrap();
    assert_eq!(
        registry.resolve_action(&stable).unwrap().availability,
        PluginActionAvailability::Unavailable(PluginActionUnavailableReason::PluginRemoved),
    );
}

#[test]
fn optional_unsupported_binding_is_dormant_until_owner_registration() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime);
    let plugin_id = plugin(3);
    let artifact_digest = digest('a');
    let mut optional = registration(
        &plugin_id,
        &artifact_digest,
        "scheduled",
        [PluginBindingPoint::AutomationAction],
    );
    optional
        .optional_bindings
        .insert(PluginBindingPoint::AutomationAction);
    registry
        .replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![optional],
        )
        .unwrap();
    assert!(matches!(
        registry.list_binding(PluginBindingPoint::AutomationAction),
        Err(PluginBindingError::UnsupportedBinding(_))
    ));

    register_owner(&registry, PluginBindingPoint::AutomationAction);
    assert_eq!(
        registry
            .list_binding(PluginBindingPoint::AutomationAction)
            .unwrap()
            .len(),
        1,
    );

    let unsupported = InMemoryPluginBindingRegistry::new(Arc::new(RecordingRuntime::default()));
    assert!(matches!(
        unsupported.replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![registration(
                &plugin_id,
                &artifact_digest,
                "required",
                [PluginBindingPoint::AutomationAction],
            )],
        ),
        Err(PluginBindingError::UnsupportedBinding(_))
    ));
}

#[test]
fn single_multiplicity_rejects_a_second_enabled_action_atomically() {
    let registry = InMemoryPluginBindingRegistry::new(Arc::new(RecordingRuntime::default()));
    registry
        .register_binding_owner(
            BindingPointContract {
                point: PluginBindingPoint::AgentBeforeModel,
                input_schema: StrictJsonValue(json!({"type": "object"})),
                output_schema: StrictJsonValue(json!({"type": "object"})),
                multiplicity: BindingMultiplicity::Single,
                failure: BindingFailureSemantics::FailClosed,
                timeout: Duration::from_secs(1),
            },
            Arc::new(PassthroughBindingAdapter),
        )
        .unwrap();
    let first = plugin(30);
    let second = plugin(31);
    let artifact_digest = digest('a');
    registry
        .replace_plugin(
            first.clone(),
            artifact_digest.clone(),
            true,
            vec![registration(
                &first,
                &artifact_digest,
                "check",
                [PluginBindingPoint::AgentBeforeModel],
            )],
        )
        .unwrap();
    let revision = registry.revision().unwrap();
    assert!(matches!(
        registry.validate_plugin_replacement(
            &second,
            &artifact_digest,
            true,
            vec![registration(
                &second,
                &artifact_digest,
                "check",
                [PluginBindingPoint::AgentBeforeModel],
            )],
        ),
        Err(PluginBindingError::MultiplicityConflict(_))
    ));
    assert_eq!(registry.revision().unwrap(), revision);
    assert!(matches!(
        registry.replace_plugin(
            second.clone(),
            artifact_digest.clone(),
            true,
            vec![registration(
                &second,
                &artifact_digest,
                "check",
                [PluginBindingPoint::AgentBeforeModel],
            )],
        ),
        Err(PluginBindingError::MultiplicityConflict(_))
    ));
    assert_eq!(registry.revision().unwrap(), revision);
    assert_eq!(
        registry
            .list_binding(PluginBindingPoint::AgentBeforeModel)
            .unwrap()
            .len(),
        1,
    );
}

#[tokio::test]
async fn desktop_events_commands_and_automation_use_registered_adapters() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime.clone());
    let adapter = Arc::new(RecordingAdapter::default());
    for point in [
        PluginBindingPoint::DesktopCommand,
        PluginBindingPoint::DesktopEvent,
        PluginBindingPoint::AutomationAction,
    ] {
        registry
            .register_binding_owner(
                owner(
                    point,
                    BindingFailureSemantics::Continue,
                    Duration::from_secs(1),
                ),
                adapter.clone(),
            )
            .unwrap();
    }
    let plugin_id = plugin(4);
    let artifact_digest = digest('a');
    registry
        .replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "command",
                    [PluginBindingPoint::DesktopCommand],
                ),
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "event_first",
                    [PluginBindingPoint::DesktopEvent],
                ),
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "event_second",
                    [PluginBindingPoint::DesktopEvent],
                ),
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "automate",
                    [PluginBindingPoint::AutomationAction],
                ),
            ],
        )
        .unwrap();

    let desktop = DesktopPluginBindings::new(registry.clone());
    let command = desktop.commands().unwrap()[0].stable_action_id.clone();
    let selected = desktop.select(&command).unwrap();
    desktop
        .trigger(
            &selected,
            StrictJsonValue(json!({"source": "desktop"})),
            Default::default(),
        )
        .await
        .unwrap();
    let events = desktop
        .emit_event(
            StrictJsonValue(json!({"event": "wake"})),
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        events
            .outputs
            .iter()
            .map(|output| output.stable_action_id.as_str())
            .collect::<Vec<_>>(),
        [
            format!("plugin:{}/event_first", plugin_id.as_ref()),
            format!("plugin:{}/event_second", plugin_id.as_ref()),
        ]
    );

    let automation = AutomationPluginBindings::new(registry);
    let action = automation.actions().unwrap()[0].stable_action_id.clone();
    automation
        .trigger(
            &action,
            StrictJsonValue(json!({"source": "automation"})),
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        adapter.points.lock().unwrap().as_slice(),
        &[
            PluginBindingPoint::DesktopCommand,
            PluginBindingPoint::DesktopEvent,
            PluginBindingPoint::DesktopEvent,
            PluginBindingPoint::AutomationAction,
        ],
    );
    assert_eq!(runtime.calls().len(), 4);
}

#[tokio::test]
async fn agent_tools_context_and_hooks_keep_activation_order() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime);
    for point in [
        PluginBindingPoint::AgentTool,
        PluginBindingPoint::AgentContext,
        PluginBindingPoint::AgentBeforeModel,
        PluginBindingPoint::AgentBeforeTool,
    ] {
        register_owner(&registry, point);
    }
    let plugin_id = plugin(5);
    let artifact_digest = digest('a');
    registry
        .replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "second",
                    [
                        PluginBindingPoint::AgentContext,
                        PluginBindingPoint::AgentBeforeModel,
                        PluginBindingPoint::AgentBeforeTool,
                    ],
                ),
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "first",
                    [
                        PluginBindingPoint::AgentTool,
                        PluginBindingPoint::AgentContext,
                        PluginBindingPoint::AgentBeforeModel,
                    ],
                ),
            ],
        )
        .unwrap();
    let agent = AgentPluginBindings::new(registry);
    let expected = [
        format!("plugin:{}/second", plugin_id.as_ref()),
        format!("plugin:{}/first", plugin_id.as_ref()),
    ];
    assert_eq!(
        agent
            .contexts()
            .unwrap()
            .iter()
            .map(|action| action.stable_action_id.clone())
            .collect::<Vec<_>>(),
        expected,
    );
    assert_eq!(
        agent
            .before_model()
            .unwrap()
            .iter()
            .map(|action| action.stable_action_id.clone())
            .collect::<Vec<_>>(),
        expected,
    );
    assert_eq!(agent.before_tool().unwrap()[0].stable_action_id, expected[0]);
    let snapshot = agent.snapshot().unwrap();
    assert!(snapshot.revision > 0);
    assert_eq!(
        snapshot
            .contexts
            .iter()
            .map(|action| action.stable_action_id.clone())
            .collect::<Vec<_>>(),
        expected,
    );
    assert_eq!(snapshot.tools.len(), 1);
    assert_eq!(snapshot.before_model.len(), 2);
    assert_eq!(snapshot.before_tool.len(), 1);
    let context = agent
        .contribute_context(StrictJsonValue(json!({})), Default::default())
        .await
        .unwrap();
    assert_eq!(
        context
            .outputs
            .iter()
            .map(|output| output.stable_action_id.clone())
            .collect::<Vec<_>>(),
        expected,
    );
    assert_eq!(
        agent
            .run_before_model(StrictJsonValue(json!({})), Default::default())
            .await
            .unwrap()
            .outputs
            .len(),
        2,
    );
    assert_eq!(
        agent
            .run_before_tool(StrictJsonValue(json!({})), Default::default())
            .await
            .unwrap()
            .outputs
            .len(),
        1,
    );
    let tool = agent.tools().unwrap()[0].stable_action_id.clone();
    agent
        .invoke_tool(&tool, StrictJsonValue(json!({})), Default::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn continued_failure_is_reported_without_reordering_later_actions() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime.clone());
    registry
        .register_binding_owner(
            owner(
                PluginBindingPoint::DesktopEvent,
                BindingFailureSemantics::Continue,
                Duration::from_secs(1),
            ),
            Arc::new(PassthroughBindingAdapter),
        )
        .unwrap();
    let plugin_id = plugin(6);
    let artifact_digest = digest('a');
    let failing = format!("plugin:{}/fails", plugin_id.as_ref());
    runtime.fail(&failing);
    registry
        .replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "fails",
                    [PluginBindingPoint::DesktopEvent],
                ),
                registration(
                    &plugin_id,
                    &artifact_digest,
                    "runs",
                    [PluginBindingPoint::DesktopEvent],
                ),
            ],
        )
        .unwrap();
    let report = DesktopPluginBindings::new(registry)
        .emit_event(StrictJsonValue(json!({})), Default::default())
        .await
        .unwrap();
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].stable_action_id, failing);
    assert_eq!(
        report.outputs[0].stable_action_id,
        format!("plugin:{}/runs", plugin_id.as_ref()),
    );
}

#[tokio::test]
async fn recursion_and_call_depth_are_rejected_before_runtime_dispatch() {
    let runtime = Arc::new(RecordingRuntime::default());
    let registry = InMemoryPluginBindingRegistry::with_limits(
        runtime.clone(),
        2,
        Duration::from_secs(1),
    )
    .unwrap();
    let plugin_id = plugin(7);
    let artifact_digest = digest('a');
    let stable = format!("plugin:{}/loop", plugin_id.as_ref());
    registry
        .replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![registration(
                &plugin_id,
                &artifact_digest,
                "loop",
                [],
            )],
        )
        .unwrap();
    let nested_probe = PluginActionInvocation {
        stable_action_id: stable.clone(),
        publication: publication(&plugin_id, &artifact_digest, "loop", []),
        binding_point: None,
        input: StrictJsonValue(json!({})),
        call_chain: vec![stable.clone()],
        cancellation: PluginCancellation::new(),
    };
    assert_eq!(
        PluginDispatchOptions::nested(&nested_probe).call_chain,
        vec![stable.clone()],
    );
    assert!(matches!(
        registry
            .dispatch_action(
                &stable,
                StrictJsonValue(json!({})),
                PluginDispatchOptions {
                    call_chain: vec![stable.clone()],
                    ..Default::default()
                },
            )
            .await,
        Err(PluginBindingError::RecursiveCall(_))
    ));
    assert!(matches!(
        registry
            .dispatch_action(
                &stable,
                StrictJsonValue(json!({})),
                PluginDispatchOptions {
                    call_chain: vec!["plugin:a/one".into(), "plugin:b/two".into()],
                    ..Default::default()
                },
            )
            .await,
        Err(PluginBindingError::CallDepthExceeded)
    ));
    assert!(runtime.calls().is_empty());
}

#[derive(Default)]
struct SlowRuntime {
    started: Notify,
    cancellations: AtomicUsize,
}

#[async_trait]
impl PluginActionRuntimePort for SlowRuntime {
    async fn invoke(
        &self,
        invocation: PluginActionInvocation,
    ) -> Result<StrictJsonValue, PluginActionCallError> {
        self.started.notify_one();
        invocation.cancellation.cancelled().await;
        self.cancellations.fetch_add(1, Ordering::AcqRel);
        Err(PluginActionCallError::new("CANCELED", "canceled"))
    }
}

async fn wait_for_cancellations(runtime: &SlowRuntime, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while runtime.cancellations.load(Ordering::Acquire) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancellation_and_timeout_cancel_the_runtime_call() {
    let runtime = Arc::new(SlowRuntime::default());
    let registry = InMemoryPluginBindingRegistry::new(runtime.clone());
    registry
        .register_binding_owner(
            owner(
                PluginBindingPoint::DesktopCommand,
                BindingFailureSemantics::FailClosed,
                Duration::from_millis(20),
            ),
            Arc::new(PassthroughBindingAdapter),
        )
        .unwrap();
    let plugin_id = plugin(8);
    let artifact_digest = digest('a');
    let stable = format!("plugin:{}/slow", plugin_id.as_ref());
    registry
        .replace_plugin(
            plugin_id.clone(),
            artifact_digest.clone(),
            true,
            vec![registration(
                &plugin_id,
                &artifact_digest,
                "slow",
                [PluginBindingPoint::DesktopCommand],
            )],
        )
        .unwrap();

    let timed_out = tokio::spawn({
        let registry = registry.clone();
        let stable = stable.clone();
        async move {
            registry
                .dispatch_binding(
                    PluginBindingPoint::DesktopCommand,
                    &stable,
                    StrictJsonValue(json!({})),
                    Default::default(),
                )
                .await
        }
    });
    runtime.started.notified().await;
    assert!(matches!(
        timed_out.await.unwrap(),
        Err(PluginBindingError::Timeout)
    ));
    wait_for_cancellations(&runtime, 1).await;

    let cancellation = PluginCancellation::new();
    let dispatched = tokio::spawn({
        let registry = registry.clone();
        let stable = stable.clone();
        let cancellation = cancellation.clone();
        async move {
            registry
                .dispatch_binding(
                    PluginBindingPoint::DesktopCommand,
                    &stable,
                    StrictJsonValue(json!({})),
                    PluginDispatchOptions {
                        cancellation,
                        ..Default::default()
                    },
                )
                .await
        }
    });
    runtime.started.notified().await;
    cancellation.cancel();
    assert!(matches!(
        dispatched.await.unwrap(),
        Err(PluginBindingError::Canceled)
    ));
    wait_for_cancellations(&runtime, 2).await;
}

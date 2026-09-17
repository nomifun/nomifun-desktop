//! Actual Nomi ToolRegistry consumption of the Kernel subcall seam. These
//! in-process handlers are not evidence that the JS SDK can issue subcalls.
use super::*;
use nomifun_agent_kernel::{CapabilityDependencyCall, CapabilityDependencyCaller};

const CHILD: &str = "example.dynamic.dependency";

struct Relay {
    retained: Arc<Mutex<Option<CapabilityDependencyCaller>>>,
}

#[async_trait]
impl CapabilityContextContributionFactory for Relay {
    async fn contribute(
        &self,
        request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        *self.retained.lock().unwrap() = Some(request.dependencies.clone());
        let message = match request.input {
            nomifun_agent_contracts::ContextContributionInput::SessionStart => "startup".into(),
            nomifun_agent_contracts::ContextContributionInput::BeforeTurn { turn } => turn.text,
        };
        let value = request
            .dependencies
            .invoke(CapabilityDependencyCall {
                capability_id: CHILD.into(),
                action_id: AGENT_ACTION.into(),
                call_key: "context-result".into(),
                input: StrictJsonValue(json!({"message":message})),
            })
            .await?;
        Ok(ContextContributionResult { value: Some(value) })
    }
}

#[tokio::test]
async fn initial_context_rejects_tool_dependencies_and_turn_contexts_use_distinct_evaluation_ids()
{
    for phase in [
        nomifun_agent_contracts::ContextContributionPhase::SessionStart,
        nomifun_agent_contracts::ContextContributionPhase::BeforeTurn,
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let evidence = Arc::new(Mutex::new(Vec::new()));
        let retained = Arc::new(Mutex::new(None));
        let (base, schemas) = registration('a', "unused:", calls.clone(), evidence.clone());
        let mut registration = PluginRegistration::new(base.metadata);
        let capabilities = &mut registration
            .metadata
            .manifest
            .payload
            .contributions
            .capabilities;
        let mut child = capabilities
            .iter()
            .find(|value| value.id.as_ref() == AGENT_TOOL)
            .unwrap()
            .clone();
        child.id = CHILD.into();
        child.contribution_id = "capability:example.dynamic.dependency".into();
        child
            .contributions
            .actions
            .retain(|action| action.action_id.as_ref() == AGENT_ACTION);
        let context = capabilities
            .iter_mut()
            .find(|value| value.id.as_ref() == CONTEXT_CAPABILITY)
            .unwrap();
        context.contributions.context_phase = phase;
        context.requires.push(CapabilityRef {
            id: CHILD.into(),
            version: VERSION.into(),
        });
        capabilities.push(child);
        registration.metadata.manifest =
            ArtifactEnvelope::new(registration.metadata.manifest.payload.clone()).unwrap();
        for id in [AGENT_TOOL, UI_ONLY_TOOL, CHILD] {
            registration
                .add_capability_handler(
                    id.into(),
                    Arc::new(CapturingHandler {
                        prefix: "context-dependency:",
                        calls: calls.clone(),
                        evidence: evidence.clone(),
                    }),
                )
                .unwrap();
        }
        registration
            .add_capability_context_factory(
                CONTEXT_CAPABILITY.into(),
                Arc::new(Relay {
                    retained: retained.clone(),
                }),
            )
            .unwrap();
        let kernel = Arc::new(
            KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
        );
        let registry = kernel.replace_all(vec![registration]).unwrap();
        let compiled = compile(&registry);
        // Rebuild the same Session twice; a restarted in-memory sequence must
        // not alias two evaluations to the same dependency effect identity.
        for _ in 0..2 {
            if phase == nomifun_agent_contracts::ContextContributionPhase::SessionStart {
                let error = match try_session(kernel.clone(), compiled.clone(), schemas.clone()).await {
                    Ok(_) => panic!("SessionStart Context must not invoke a Tool dependency"),
                    Err(error) => error,
                };
                let NomiPluginToolError::Kernel(error) = error else {
                    panic!("SessionStart Tool dependency returned a non-Kernel error")
                };
                assert_eq!(error.canonical_code().as_ref(), "DEPENDENCY_TURN_REQUIRED");
                continue;
            }
            let loaded = session(kernel.clone(), compiled.clone(), schemas.clone()).await;
            assert!(
                !loaded
                    .actions()
                    .iter()
                    .any(|action| action.capability_id().as_ref() == CHILD)
            );
            let value = loaded.context_contributors()[0]
                .pre_turn_context_for_turn_result(
                    &nomi_agent::context_contributor::TurnContext {
                        turn_id: "turn-same-source".into(),
                        source_message_id: "same-source".into(),
                        text: "turn".into(),
                        image_media_types: Vec::new(),
                        cs_dialogue_id: None,
                    },
                )
                .await
                .unwrap()
                .unwrap();
            assert!(value.contains("context-dependency:turn"));
            let caller = retained.lock().unwrap().take().unwrap();
            assert_eq!(
                caller
                    .invoke(CapabilityDependencyCall {
                        capability_id: CHILD.into(),
                        action_id: AGENT_ACTION.into(),
                        call_key: "late".into(),
                        input: StrictJsonValue(json!({})),
                    })
                    .await
                    .unwrap_err()
                    .canonical_code()
                    .as_ref(),
                "DEPENDENCY_PARENT_CLOSED"
            );
        }
        let expected_calls = usize::from(
            phase == nomifun_agent_contracts::ContextContributionPhase::BeforeTurn,
        ) * 2;
        assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
        let evidence = evidence.lock().unwrap();
        if phase == nomifun_agent_contracts::ContextContributionPhase::BeforeTurn {
            assert_ne!(evidence[0].idempotency_key, evidence[1].idempotency_key);
        } else {
            assert!(evidence.is_empty());
        }
        for item in evidence.iter() {
            assert_eq!(item.agent_session_id, SESSION);
            assert_eq!(item.capability_id, CHILD);
        }
    }
}

#[async_trait]
impl CapabilityHandler for Relay {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        *self.retained.lock().unwrap() = Some(context.dependencies.clone());
        context
            .dependencies
            .invoke(CapabilityDependencyCall {
                capability_id: CHILD.into(),
                action_id: AGENT_ACTION.into(),
                call_key: "render-result".into(),
                input,
            })
            .await
    }
}

#[tokio::test]
async fn nomi_tool_execution_consumes_a_managed_dependency_without_another_session() {
    let calls = Arc::new(AtomicUsize::new(0));
    let evidence = Arc::new(Mutex::new(Vec::new()));
    let retained = Arc::new(Mutex::new(None));
    let (base, schemas) = registration('a', "unused:", calls.clone(), evidence.clone());
    let mut registration = PluginRegistration::new(base.metadata);
    let capabilities = &mut registration
        .metadata
        .manifest
        .payload
        .contributions
        .capabilities;
    let root = capabilities
        .iter_mut()
        .find(|capability| capability.id.as_ref() == AGENT_TOOL)
        .unwrap();
    let mut child = root.clone();
    child.id = CHILD.into();
    child.contribution_id = "capability:example.dynamic.dependency".into();
    child
        .contributions
        .actions
        .retain(|action| action.action_id.as_ref() == AGENT_ACTION);
    root.requires.push(CapabilityRef {
        id: CHILD.into(),
        version: VERSION.into(),
    });
    capabilities.push(child);
    registration.metadata.manifest =
        ArtifactEnvelope::new(registration.metadata.manifest.payload.clone()).unwrap();
    registration
        .add_capability_handler(
            AGENT_TOOL.into(),
            Arc::new(Relay {
                retained: retained.clone(),
            }),
        )
        .unwrap();
    for id in [UI_ONLY_TOOL, CHILD] {
        registration
            .add_capability_handler(
                id.into(),
                Arc::new(CapturingHandler {
                    prefix: "dependency:",
                    calls: calls.clone(),
                    evidence: evidence.clone(),
                }),
            )
            .unwrap();
    }
    registration
        .add_capability_context_factory(CONTEXT_CAPABILITY.into(), Arc::new(EmptyContextFactory))
        .unwrap();
    let kernel = Arc::new(
        KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let session = session(kernel, compile(&materialized), schemas).await;
    assert!(
        !session
            .actions()
            .iter()
            .any(|action| action.capability_id().as_ref() == CHILD)
    );
    let action = session
        .actions()
        .iter()
        .find(|action| action.capability_id().as_ref() == AGENT_TOOL)
        .unwrap();
    let mut tools = ToolRegistry::new();
    session.register_into(&mut tools).unwrap();
    let result = tools
        .get(action.provider_name())
        .unwrap()
        .execute_with_context(
            json!({"message":"hello"}),
            &ToolExecutionContext::from_scoped_tool_call("turn-dependency", "root-call"),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<Value>(&result.content).unwrap()["echo"],
        "dependency:hello"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    {
        let evidence = evidence.lock().unwrap();
        assert_eq!(evidence[0].agent_session_id, SESSION);
        assert_eq!(evidence[0].capability_id, CHILD);
        assert!(evidence[0].operation_id.starts_with("dependency-"));
        assert!(
            evidence[0]
                .correlation_id
                .starts_with("nomi-plugin:tool-call-v1-")
        );
    }
    let caller = retained.lock().unwrap().take().unwrap();
    let error = caller
        .invoke(CapabilityDependencyCall {
            capability_id: CHILD.into(),
            action_id: AGENT_ACTION.into(),
            call_key: "late-effect".into(),
            input: StrictJsonValue(json!({"message":"late"})),
        })
        .await
        .unwrap_err();
    assert_eq!(error.canonical_code().as_ref(), "DEPENDENCY_PARENT_CLOSED");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

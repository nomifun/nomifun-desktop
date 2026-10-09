//! Canonical initial and per-turn Context consumption of the Kernel subcall seam.
use super::*;
use nomifun_agent_kernel::{CapabilityDependencyCall, CapabilityDependencyCaller, SessionCapabilityState};
use nomifun_ai_agent::{NomiTurnContextContributor, context_contributor::ContextContributor};

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
        let base = registration('a', "unused:", calls.clone(), evidence.clone());
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
        for rebuild in 0..2 {
            if phase == nomifun_agent_contracts::ContextContributionPhase::SessionStart {
                let error = match initial_context(&kernel, &compiled, None).await {
                    Ok(_) => panic!("SessionStart Context must not invoke a Tool dependency"),
                    Err(error) => error,
                };
                let NomiPluginToolError::Kernel(error) = error else {
                    panic!("SessionStart Tool dependency returned a non-Kernel error")
                };
                assert_eq!(error.canonical_code().as_ref(), "DEPENDENCY_TURN_REQUIRED");
                continue;
            }
            let inactive = Arc::new(SessionCapabilityState::from_committed(
                &compiled,
                1,
                compiled.content().capability_allowlist.iter()
                    .filter(|id| id.as_ref() != CONTEXT_CAPABILITY)
                    .cloned(),
            ).unwrap());
            let inactive_context = NomiTurnContextContributor::new(
                kernel.clone(), Arc::new(compiled.clone()), inactive,
                owner(), AgentSessionId::from(SESSION),
                ScopeKey::from(format!("session:{SESSION}")),
                vec![CapabilityId::from(CONTEXT_CAPABILITY)],
            );
            let before = calls.load(Ordering::SeqCst);
            assert!(inactive_context.pre_turn_context_for_turn_result(
                &nomifun_ai_agent::context_contributor::TurnContext {
                    turn_id: "turn-inactive".into(),
                    source_message_id: "same-source".into(),
                    text: "inactive persona".into(),
                    image_media_types: Vec::new(),
                    cs_dialogue_id: None,
                },
            ).await.unwrap().is_none());
            assert_eq!(calls.load(Ordering::SeqCst), before,
                "inactive Context must not dispatch its dependency");
            assert!(retained.lock().unwrap().is_none());
            let (_, context_ids) = initial_context(&kernel, &compiled, None).await.unwrap();
            let loaded = NomiTurnContextContributor::new(kernel.clone(), Arc::new(compiled.clone()),
                Arc::new(SessionCapabilityState::new(&compiled)), owner(), AgentSessionId::from(SESSION),
                ScopeKey::from(format!("session:{SESSION}")), context_ids);
            let refreshed = format!("turn-{rebuild}");
            let value = loaded
                .pre_turn_context_for_turn_result(
                    &nomifun_ai_agent::context_contributor::TurnContext {
                        turn_id: "turn-same-source".into(),
                        source_message_id: "same-source".into(),
                        text: refreshed.clone(),
                        image_media_types: Vec::new(),
                        cs_dialogue_id: None,
                    },
                )
                .await
                .unwrap()
                .unwrap();
            assert!(value.contains(&format!("context-dependency:{refreshed}")),
                "a fresh active context must use this turn's persona text");
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

use super::*;
use nomi_agent::context_contributor::TurnContext;
use nomifun_agent_contracts::{
    ContextContributionInput, ContextContributionPhase, ContextTurnInput,
};

struct TurnFactory(Arc<Mutex<Vec<ContextContributionInput>>>);

struct OrderedFactory {
    id: &'static str,
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl CapabilityContextContributionFactory for OrderedFactory {
    async fn contribute(
        &self,
        _: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        self.seen.lock().unwrap().push(self.id.into());
        Ok(ContextContributionResult {
            value: Some(StrictJsonValue(json!({"id": self.id}))),
        })
    }
}

const INITIAL_A: &str = "example.order.initial-a";
const INITIAL_Z: &str = "example.order.initial-z";
const TURN_A: &str = "example.order.turn-a";
const TURN_Z: &str = "example.order.turn-z";

async fn ordered_setup() -> (
    Arc<KernelRegistry>,
    Arc<SchemaMap>,
    Arc<Mutex<Vec<String>>>,
    AgentPresetRevision,
) {
    let (mut registration, schemas) = registration(
        'a',
        "tool:",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut manifest = registration.metadata.manifest.payload.clone();
    let base = manifest
        .contributions
        .capabilities
        .iter()
        .find(|value| value.id.as_ref() == CONTEXT_CAPABILITY)
        .unwrap()
        .clone();
    for (id, phase) in [
        (TURN_Z, ContextContributionPhase::BeforeTurn),
        (INITIAL_Z, ContextContributionPhase::SessionStart),
        (TURN_A, ContextContributionPhase::BeforeTurn),
        (INITIAL_A, ContextContributionPhase::SessionStart),
    ] {
        let mut capability = base.clone();
        capability.id = id.into();
        capability.contribution_id = format!("capability:{id}").into();
        capability.contributions.context_phase = phase;
        if id == TURN_Z {
            capability.requires = [INITIAL_A, INITIAL_Z, TURN_A]
                .into_iter()
                .map(|id| CapabilityRef {
                    id: id.into(),
                    version: VERSION.into(),
                })
                .collect();
        }
        manifest.contributions.capabilities.push(capability);
        registration
            .metadata
            .registrar
            .declared_capability_ids
            .insert(id.into());
        registration
            .add_capability_context_factory(
                id.into(),
                Arc::new(OrderedFactory {
                    id,
                    seen: seen.clone(),
                }),
            )
            .unwrap();
    }
    registration.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
    let kernel = Arc::new(
        KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
    );
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let mut revision = revision_with_capabilities(&registry);
    revision.payload.enabled_capabilities = [TURN_Z, INITIAL_A, TURN_A, INITIAL_Z]
        .into_iter()
        .map(|id| selection(id, &[]))
        .collect();
    revision.contribution_locks = revision
        .payload
        .enabled_capabilities
        .iter()
        .map(|value| {
            registry
                .capability(&value.capability.id)
                .unwrap()
                .contribution_lock
                .clone()
        })
        .collect();
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    (kernel, schemas, seen, revision)
}

#[tokio::test]
async fn restricted_session_does_not_execute_plugin_context_at_either_phase() {
    let (kernel, schemas, seen, revision) = ordered_setup().await;
    let compiled = compile_revision(&kernel.snapshot().unwrap(), revision);
    let loaded = restricted_session(kernel, compiled, schemas).await;
    assert!(seen.lock().unwrap().is_empty());
    assert!(loaded.initial_context_contributions().is_empty());
    assert!(loaded.context_contributors().is_empty());
}

#[tokio::test]
async fn private_context_dependencies_are_not_consumed_at_start_or_before_turn() {
    let (kernel, schemas, seen, mut revision) = ordered_setup().await;
    let registry = kernel.snapshot().unwrap();
    revision
        .payload
        .enabled_capabilities
        .retain(|value| value.capability.id.as_ref() == TURN_Z);
    revision.contribution_locks = vec![
        registry
            .capability(&TURN_Z.into())
            .unwrap()
            .contribution_lock
            .clone(),
    ];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let compiled = compile_revision(&registry, revision);
    assert_eq!(compiled.content().enabled_capabilities.len(), 4);
    assert_eq!(compiled.content().contributions().count(), 1);
    let active = nomifun_agent_kernel::SessionCapabilityState::new(&compiled)
        .snapshot()
        .unwrap();
    let request = nomifun_agent_kernel::CapabilityAccessRequest {
        principal: owner(),
        session_owner: owner(),
        agent_session_id: SESSION.into(),
        turn_id: None,
        operation_id: "private-context".into(),
        correlation_id: "private-context".into(),
        resolved_snapshot_ref: compiled.snapshot_ref().clone(),
        active_set_generation: active.generation,
        capability_id: INITIAL_A.into(),
        resource_binding_ids: BTreeSet::new(),
        state_scope_key: format!("session:{SESSION}").into(),
    };
    assert!(matches!(
        kernel.contribute_context(&compiled, &active, request).await,
        Err(KernelError::CapabilityNotInPreset { .. })
    ));
    let loaded = session(kernel, compiled, schemas).await;
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(
        loaded
            .system_prompt_with_initial_context(Some("base"))
            .unwrap()
            .as_deref(),
        Some("base")
    );
    loaded.context_contributors()[0]
        .pre_turn_context_for_turn_result(&turn("one"))
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![TURN_Z]);
}

#[tokio::test]
async fn frozen_context_order_controls_calls_and_prompt_data_within_each_phase() {
    let (kernel, schemas, seen, mut revision) = ordered_setup().await;
    let registry = kernel.snapshot().unwrap();
    let legacy = compile_revision(&registry, revision.clone());
    // Interleaved phases are filtered, never moved across their declared timing.
    revision.payload.context_order = vec![TURN_Z.into(), INITIAL_Z.into()];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let ordered = compile_revision(&registry, revision.clone());
    assert_ne!(legacy.snapshot_ref(), ordered.snapshot_ref());
    assert_ne!(
        legacy.content().compiled_runtime_profile_digest,
        ordered.content().compiled_runtime_profile_digest
    );
    assert_eq!(
        legacy.content().capability_allowlist,
        ordered.content().capability_allowlist
    );
    assert_eq!(legacy.authority_policies, ordered.authority_policies);
    let loaded = session(kernel.clone(), ordered, schemas.clone()).await;
    assert_eq!(*seen.lock().unwrap(), vec![INITIAL_Z, INITIAL_A]);
    let prompt = loaded
        .system_prompt_with_initial_context(Some("base"))
        .unwrap()
        .unwrap();
    assert!(prompt.find(INITIAL_Z).unwrap() < prompt.find(INITIAL_A).unwrap());
    assert!(!prompt.contains(TURN_Z));
    for text in ["one", "two"] {
        let output = loaded.context_contributors()[0]
            .pre_turn_context_for_turn_result(&turn(text))
            .await
            .unwrap()
            .unwrap();
        assert!(output.find(TURN_Z).unwrap() < output.find(TURN_A).unwrap());
    }
    assert_eq!(
        *seen.lock().unwrap(),
        vec![INITIAL_Z, INITIAL_A, TURN_Z, TURN_A, TURN_Z, TURN_A]
    );
    seen.lock().unwrap().clear();
    let old = session(kernel.clone(), legacy, schemas).await;
    assert_eq!(*seen.lock().unwrap(), vec![INITIAL_A, INITIAL_Z]);
    old.context_contributors()[0]
        .pre_turn_context_for_turn_result(&turn("old"))
        .await
        .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![INITIAL_A, INITIAL_Z, TURN_A, TURN_Z]
    );
    kernel.replace_all(Vec::new()).unwrap();
    assert!(
        loaded.context_contributors()[0]
            .pre_turn_context_for_turn_result(&turn("withdrawn"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn context_order_cannot_select_tools_or_unselected_capabilities() {
    let (kernel, _, _, mut revision) = ordered_setup().await;
    revision.payload.context_order = vec!["missing".into()];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    assert!(
        revision
            .validate()
            .unwrap_err()
            .message
            .contains("context_order")
    );
    revision.payload.context_order = vec![INITIAL_Z.into(), INITIAL_Z.into()];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    assert!(
        revision
            .validate()
            .unwrap_err()
            .message
            .contains("context_order")
    );
    let mut tool_revision = revision_with_capabilities(&kernel.snapshot().unwrap());
    tool_revision.payload.context_order = vec![AGENT_TOOL.into()];
    tool_revision.reference.revision_digest = tool_revision.revision_digest().unwrap();
    let error = try_compile_revision(&kernel.snapshot().unwrap(), tool_revision).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must publish Agent Context")
    );
}
#[async_trait]
impl CapabilityContextContributionFactory for TurnFactory {
    async fn contribute(
        &self,
        request: CapabilityContextContributionRequest,
    ) -> Result<ContextContributionResult, KernelError> {
        self.0.lock().unwrap().push(request.input.clone());
        let ContextContributionInput::BeforeTurn { turn } = request.input else {
            panic!("before-turn context must not run at Session startup");
        };
        if turn.text == "fail" {
            return Err(KernelError::CapabilityExecution {
                reason: "context unavailable".into(),
            });
        }
        if turn.text == "hang" {
            std::future::pending::<()>().await;
        }
        if turn.text == "oversize" {
            return Ok(ContextContributionResult {
                value: Some(StrictJsonValue(json!("x".repeat(65536)))),
            });
        }
        Ok(ContextContributionResult {
            value: Some(StrictJsonValue(json!({
                "guidance": format!("guidance:{}", turn.text),
                "source": turn.source_message_id,
                "media": turn.image_media_types,
            }))),
        })
    }
}

#[tokio::test]
async fn explicitly_ordered_but_unadmitted_builtin_is_not_silently_skipped() {
    let (registration, schemas) = registration_with_source(
        'a',
        "tool:",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
        PluginSourceKind::Bundled,
        false,
    );
    let kernel = Arc::new(
        KernelRegistry::new(
            policy_for(PluginSourceKind::Bundled),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let mut revision = revision_with_capabilities(&registry);
    revision.payload.context_order = vec![CONTEXT_CAPABILITY.into()];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let result = KernelNomiPluginToolSession::materialize(
        kernel,
        Arc::new(compile_revision(&registry, revision)),
        owner(),
        AgentSessionId::from(SESSION),
        ScopeKey::from(format!("session:{SESSION}")),
        schemas,
    )
    .await;
    let error = match result {
        Ok(_) => panic!("unadmitted Context silently ignored"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("not admitted by this runtime"));
}

async fn setup() -> (
    Arc<KernelRegistry>,
    nomifun_ai_agent::NomiPluginToolSession,
    Arc<Mutex<Vec<ContextContributionInput>>>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (mut registration, schemas) = registration_with_context_factory(
        'a',
        "tool:",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
        PluginSourceKind::ManagedLocal,
        false,
        Arc::new(TurnFactory(seen.clone())),
    );
    let mut manifest = registration.metadata.manifest.payload.clone();
    manifest
        .contributions
        .capabilities
        .iter_mut()
        .find(|capability| capability.id.as_ref() == CONTEXT_CAPABILITY)
        .unwrap()
        .contributions
        .context_phase = ContextContributionPhase::BeforeTurn;
    registration.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
    let kernel = Arc::new(
        KernelRegistry::new(policy(), Arc::new(InMemoryPluginStatePersistence::new())).unwrap(),
    );
    let materialized = kernel.replace_all(vec![registration]).unwrap();
    let loaded = session(kernel.clone(), compile(&materialized), schemas).await;
    (kernel, loaded, seen)
}

fn turn(text: &str) -> TurnContext {
    TurnContext {
        turn_id: format!("turn-{text}"),
        source_message_id: format!("message-{text}"),
        text: text.into(),
        image_media_types: vec!["image/png".into()],
        cs_dialogue_id: Some("private-host-only".into()),
    }
}

#[tokio::test]
async fn selected_context_receives_fresh_turn_facts_without_initial_or_tool_exposure() {
    let (kernel, loaded, seen) = setup().await;
    assert!(seen.lock().unwrap().is_empty());
    assert!(loaded.initial_context_contributions().is_empty());
    assert!(
        loaded
            .provider_names_for(CONTEXT_CAPABILITY, false)
            .is_empty()
    );
    assert_eq!(loaded.context_contributors().len(), 1);
    let contributor = &loaded.context_contributors()[0];
    for text in ["first", "second"] {
        let output = contributor
            .pre_turn_context_for_turn_result(&turn(text))
            .await
            .unwrap()
            .unwrap();
        assert!(output.contains(&format!("guidance:{text}")));
        assert!(!output.contains("private-host-only"));
    }
    let observed = seen.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert!(observed.iter().all(|input| matches!(
        input,
        ContextContributionInput::BeforeTurn {
            turn: ContextTurnInput {
                cs_dialogue_id: None,
                ..
            }
        }
    )));
    drop(observed);
    kernel.replace_all(Vec::new()).unwrap();
    assert!(
        contributor
            .pre_turn_context_for_turn_result(&turn("withdrawn"))
            .await
            .is_err()
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn dynamic_context_errors_and_input_output_limits_are_not_silently_ignored() {
    let (_, loaded, seen) = setup().await;
    let contributor = &loaded.context_contributors()[0];
    assert!(
        contributor
            .pre_turn_context_for_turn_result(&turn("fail"))
            .await
            .unwrap_err()
            .contains("context unavailable")
    );
    assert!(
        contributor
            .pre_turn_context_for_turn_result(&turn("oversize"))
            .await
            .unwrap_err()
            .contains("64 KiB")
    );
    assert!(
        contributor
            .pre_turn_context_for_turn_result(&turn(&"x".repeat(256 * 1024)))
            .await
            .unwrap_err()
            .contains("256 KiB")
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "oversized input never reaches the factory"
    );
}

#[tokio::test]
async fn dynamic_context_deadline_is_bounded_and_reports_the_selected_capability() {
    let (_, loaded, _) = setup().await;
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(7),
        loaded.context_contributors()[0].pre_turn_context_for_turn_result(&turn("hang")),
    )
    .await
    .expect("Context deadline failed to bound the wait")
    .unwrap_err();
    assert!(error.contains(CONTEXT_CAPABILITY));
    assert!(error.contains("shared 5 second deadline"));
}

#[tokio::test]
async fn real_engine_consumes_dynamic_context_on_successive_turns_and_stops_after_withdrawal() {
    use nomi_config::config::{Config, ProviderType};
    use nomi_providers::{LlmProvider, ProviderError};
    use nomi_types::llm::{LlmEvent, LlmRequest};
    use nomi_types::message::StopReason;
    struct Capture(Arc<Mutex<Vec<LlmRequest>>>);
    #[async_trait]
    impl LlmProvider for Capture {
        async fn stream(
            &self,
            request: &LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<LlmEvent>, ProviderError> {
            self.0.lock().unwrap().push(request.clone());
            let (tx, rx) = tokio::sync::mpsc::channel(2);
            tx.send(LlmEvent::TextDelta("ok".into())).await.unwrap();
            tx.send(LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
            Ok(rx)
        }
    }
    let (kernel, loaded, _) = setup().await;
    let mut config = Config {
        provider_label: "test".into(),
        provider: ProviderType::OpenAI,
        api_key: "test".into(),
        base_url: "http://localhost:0".into(),
        model: "test".into(),
        output_max_tokens: Some(1024),
        max_turns: Some(2),
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
    config.tools.enforce_builtin_allowlist = true;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let workspace = tempfile::tempdir().unwrap();
    let mut built = nomi_agent::bootstrap::AgentBootstrap::new(
        config.clone(),
        workspace.path().to_str().unwrap(),
        Arc::new(nomi_agent::output::null_sink::NullSink),
    )
    .provider(Arc::new(Capture(requests.clone())))
    .build()
    .await
    .unwrap();
    for contributor in loaded.context_contributors() {
        built
            .engine
            .register_context_contributor(contributor.clone());
    }
    for text in ["first", "second"] {
        built
            .engine
            .execute_turn(text, &format!("message-{text}"))
            .await
            .unwrap();
    }
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    let first = &captured[0].system;
    let second = &captured[1].system;
    assert!(first.contains("guidance:first"));
    assert!(second.contains("guidance:second"));
    assert!(
        !second.contains("guidance:first"),
        "dynamic context must not accumulate in persisted messages"
    );
    assert!(
        !serde_json::to_string(&captured[1].messages)
            .unwrap()
            .contains("guidance:first")
    );
    drop(captured);
    kernel.replace_all(Vec::new()).unwrap();
    assert!(
        built
            .engine
            .execute_turn("withdrawn", "message-withdrawn")
            .await
            .is_err()
    );
    assert_eq!(requests.lock().unwrap().len(), 2);

    // Verify ordering at the actual Engine's model request boundary as well.
    let (kernel, schemas, _, mut revision) = ordered_setup().await;
    revision.payload.context_order = vec![TURN_Z.into(), INITIAL_Z.into()];
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    let loaded = session(
        kernel.clone(),
        compile_revision(&kernel.snapshot().unwrap(), revision),
        schemas,
    )
    .await;
    config.system_prompt = loaded
        .system_prompt_with_initial_context(Some("base"))
        .unwrap();
    let ordered_requests = Arc::new(Mutex::new(Vec::new()));
    let mut ordered_engine = nomi_agent::bootstrap::AgentBootstrap::new(
        config,
        workspace.path().to_str().unwrap(),
        Arc::new(nomi_agent::output::null_sink::NullSink),
    )
    .provider(Arc::new(Capture(ordered_requests.clone())))
    .build()
    .await
    .unwrap();
    for contributor in loaded.context_contributors() {
        ordered_engine
            .engine
            .register_context_contributor(contributor.clone());
    }
    for text in ["ordered-one", "ordered-two"] {
        ordered_engine
            .engine
            .execute_turn(text, text)
            .await
            .unwrap();
    }
    let captured = ordered_requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    for request in captured.iter() {
        assert!(request.system.find(INITIAL_Z).unwrap() < request.system.find(INITIAL_A).unwrap());
        assert!(request.system.find(TURN_Z).unwrap() < request.system.find(TURN_A).unwrap());
        assert!(
            !serde_json::to_string(&request.messages)
                .unwrap()
                .contains(TURN_Z)
        );
    }
}

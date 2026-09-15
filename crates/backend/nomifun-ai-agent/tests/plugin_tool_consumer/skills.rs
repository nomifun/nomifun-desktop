use super::*;
use nomifun_agent_contracts::{LogicalArtifactRef, SkillDefinition, SkillRef, digest_bytes};
use nomifun_ai_agent::{NomiPluginSkillArtifact, NomiPluginSkillArtifactResolver};

const SKILL: &str = "example.dynamic.guide";
const BODY: &str = "Package guidance $ARGUMENTS";

#[tokio::test]
async fn verified_skill_descriptors_keep_body_identity_and_cannot_authorize_cold_reads() {
    use nomifun_ai_agent::plugin_skills::{NomiVerifiedSkillCommand, verified_skill_commands};
    let (registration, _, _) = fixture('a');
    let kernel = Arc::new(KernelRegistry::new(
        policy_for(PluginSourceKind::ManagedLocal), Arc::new(InMemoryPluginStatePersistence::new()),
    ).unwrap());
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let compiled = Arc::new(compiled(&registry));
    let command = || NomiVerifiedSkillCommand {
        lock: compiled.content().skill_locks[0].clone(), markdown: BODY.into(), description: "Guide".into(),
    };
    let descriptions = verified_skill_commands(kernel.clone(), compiled.clone(), None, vec![command()]).unwrap();
    assert_eq!(descriptions[0].command_name(), format!("skill:{SKILL}"));
    assert!(descriptions[0].metadata().disable_model_invocation);
    assert!(descriptions[0].read(None, None, None).await.is_err());
    let mut changed = command();
    changed.markdown.push_str("changed");
    assert!(verified_skill_commands(kernel.clone(), compiled.clone(), None, vec![changed]).is_err());
    assert!(verified_skill_commands(kernel.clone(), compiled.clone(), None, vec![command(), command()]).is_err());
    kernel.replace_all(Vec::new()).unwrap();
    assert!(verified_skill_commands(kernel.clone(), compiled.clone(), None, vec![command()]).is_err());
}

#[tokio::test]
async fn cold_skill_discovery_uses_exact_artifacts_without_materializing_a_session() {
    use nomifun_ai_agent::plugin_skills::discover_package_skill_commands;
    for scenario in ["valid", "hidden", "unsupported", "bad_digest", "withdraw"] {
        let (mut registration, _, mut resolver) = fixture('a');
        let body = match scenario {
            "hidden" => "---\nuser-invocable: false\n---\nHidden guidance",
            "unsupported" => "---\ncontext: fork\n---\nMust not run",
            _ => BODY,
        };
        resolver.definition.body_ref.digest = digest_bytes(body.as_bytes());
        resolver
            .files
            .insert("resources/guide.md".into(), body.as_bytes().to_vec());
        let mut manifest = registration.metadata.manifest.payload.clone();
        manifest.contributions.skills[0] = resolver.definition.clone();
        registration.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
        let kernel = Arc::new(
            KernelRegistry::new(
                policy_for(PluginSourceKind::ManagedLocal),
                Arc::new(InMemoryPluginStatePersistence::new()),
            )
            .unwrap(),
        );
        let registry = kernel.replace_all(vec![registration]).unwrap();
        let compiled = Arc::new(compiled(&registry));
        if scenario == "bad_digest" {
            resolver
                .files
                .insert("resources/guide.md".into(), b"changed".to_vec());
        }
        if scenario == "withdraw" {
            resolver.withdraw = Some(kernel.clone());
        }
        // No NomiPluginToolSession, Context assembly, active set or LLM is created.
        let commands = discover_package_skill_commands(kernel, compiled, Arc::new(resolver)).await;
        match scenario {
            "valid" => {
                let commands = commands.unwrap();
                assert_eq!(commands.len(), 1);
                assert_eq!(commands[0].command, format!("skill:{SKILL}"));
            }
            "hidden" => assert!(commands.unwrap().is_empty()),
            _ => assert!(commands.is_err(), "{scenario}"),
        }
    }
}

#[test]
fn skill_locks_require_provenance_and_do_not_bypass_consumer_or_dependency_selection() {
    let (registration, _, _) = fixture('a');
    let kernel = KernelRegistry::new(
        policy_for(PluginSourceKind::ManagedLocal),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap();
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let compiled = compiled(&registry);
    compiled.envelope.validate().unwrap();
    let mut old_wire = serde_json::to_value(&compiled.content().skill_locks[0]).unwrap();
    old_wire
        .as_object_mut()
        .unwrap()
        .remove("contribution_lock");
    assert!(
        serde_json::from_value::<nomifun_agent_contracts::ResolvedSkillLock>(old_wire).is_err()
    );
    for field in ["mount", "dependency", "duplicate"] {
        let mut envelope = compiled.envelope.clone();
        match field {
            "mount" => envelope.content.skill_locks[0].resolved_mount_id = "wrong".into(),
            "dependency" => {
                envelope
                    .content
                    .capability_allowlist
                    .remove(&AGENT_TOOL.into());
            }
            "duplicate" => envelope
                .content
                .skill_locks
                .push(envelope.content.skill_locks[0].clone()),
            _ => unreachable!(),
        }
        envelope.snapshot_ref.snapshot_digest = digest_payload(&envelope.content).unwrap();
        assert!(envelope.validate().is_err(), "{field}");
    }
    let mut revision = revision_with_capabilities(&registry);
    revision.payload.skill_bindings.push(SkillRef {
        id: SKILL.into(),
        version: VERSION.into(),
    });
    assert!(AgentPresetCompiler::skills_unchanged(
        &registry,
        &revision,
        &compiled.envelope
    ));
    revision
        .payload
        .enabled_capabilities
        .retain(|item| item.capability.id.as_ref() != AGENT_TOOL);
    assert!(!AgentPresetCompiler::skills_unchanged(
        &registry,
        &revision,
        &compiled.envelope
    ));
    let mut registry = (*registry).clone();
    let mut revision = revision_with_capabilities(&registry);
    revision.payload.skill_bindings.push(SkillRef {
        id: SKILL.into(),
        version: VERSION.into(),
    });
    registry
        .skills
        .get_mut(&SKILL.into())
        .unwrap()
        .definition
        .supported_surfaces =
        capability_surface_declarations(["desktop"], [CapabilityConsumer::Ui]);
    assert!(!AgentPresetCompiler::skills_unchanged(
        &registry,
        &revision,
        &compiled.envelope
    ));
}

struct Resolver {
    definition: SkillDefinition,
    files: BTreeMap<String, Vec<u8>>,
    withdraw: Option<Arc<KernelRegistry>>,
}
#[async_trait]
impl NomiPluginSkillArtifactResolver for Resolver {
    async fn resolve(
        &self,
        _: &nomifun_agent_contracts::ResolvedSkillLock,
    ) -> Result<NomiPluginSkillArtifact, String> {
        if let Some(kernel) = &self.withdraw {
            kernel.replace_all(Vec::new()).unwrap();
        }
        Ok(NomiPluginSkillArtifact {
            definition: self.definition.clone(),
            files: self.files.clone(),
        })
    }
}

fn fixture(artifact: char) -> (PluginRegistration, Arc<SchemaMap>, Resolver) {
    let (mut registration, schemas) = registration(
        artifact,
        "echo:",
        Arc::new(AtomicUsize::new(0)),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut manifest = registration.metadata.manifest.payload.clone();
    let definition = SkillDefinition {
        id: SKILL.into(),
        version: VERSION.into(),
        package: PackageRef {
            id: PACKAGE.into(),
            version: VERSION.into(),
        },
        display: display("Guide"),
        body_ref: LogicalArtifactRef {
            artifact_id: "body".into(),
            normalized_relative_path: "resources/guide.md".into(),
            digest: digest_bytes(BODY.as_bytes()),
        },
        resources: Vec::new(),
        requires_capabilities: vec![CapabilityRef {
            id: AGENT_TOOL.into(),
            version: VERSION.into(),
        }],
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent],
        ),
    };
    manifest.contributions.skills.push(definition.clone());
    registration.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
    registration
        .metadata
        .registrar
        .allowed_operations
        .insert(PluginRegistrarOperation::ContributeSkill);
    registration
        .metadata
        .registrar
        .declared_skill_ids
        .insert(SKILL.into());
    let resolver = Resolver {
        definition,
        files: BTreeMap::from([("resources/guide.md".into(), BODY.as_bytes().to_vec())]),
        withdraw: None,
    };
    (registration, schemas, resolver)
}

fn compiled(
    registry: &nomifun_agent_kernel::MaterializedRegistry,
) -> nomifun_agent_kernel::CompiledSnapshot {
    let mut revision = revision_with_capabilities(registry);
    revision.payload.skill_bindings.push(SkillRef {
        id: SKILL.into(),
        version: VERSION.into(),
    });
    revision.contribution_locks.push(
        registry
            .skill(&SKILL.into())
            .unwrap()
            .contribution_lock
            .clone(),
    );
    revision.contribution_locks.sort();
    revision.reference.revision_digest = revision.revision_digest().unwrap();
    compile_revision(registry, revision)
}

#[tokio::test]
async fn snapshot_skill_command_uses_kernel_guard_in_real_bootstrap() {
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
            tx.send(LlmEvent::TextDelta("Applied package guidance".into()))
                .await
                .unwrap();
            tx.send(LlmEvent::Done {
                stop_reason: StopReason::EndTurn,
                usage: Default::default(),
            })
            .await
            .unwrap();
            Ok(rx)
        }
    }
    let (registration, schemas, _) = fixture('a');
    let kernel = Arc::new(
        KernelRegistry::new(
            policy_for(PluginSourceKind::ManagedLocal),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let compiled = Arc::new(compiled(&registry));
    let commands = vec![nomifun_ai_agent::plugin_skills::NomiVerifiedSkillCommand {
        lock: compiled.content().skill_locks[0].clone(),
        markdown: BODY.into(), description: "Guide".into(),
    }];
    let loaded = session(kernel.clone(), (*compiled).clone(), schemas)
        .await
        .with_verified_skill_commands(kernel.clone(), compiled, commands)
        .unwrap();
    assert!(loaded.package_skills()[0].metadata().disable_model_invocation);
    let mut config = Config {
        provider_label: "test".into(),
        provider: ProviderType::OpenAI,
        api_key: "test".into(),
        base_url: "http://localhost:0".into(),
        model: "test".into(),
        output_max_tokens: Some(1024),
        max_turns: Some(2),
        system_prompt: None,
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
    loaded.extend_tool_policy(
        &mut config.tools.builtin_allowlist,
        &mut config.tools.deferred_allowlist,
    );
    let requests = Arc::new(Mutex::new(Vec::new()));
    let workspace = tempfile::tempdir().unwrap();
    let mut built = nomi_agent::bootstrap::AgentBootstrap::new(
        config,
        workspace.path().to_str().unwrap(),
        Arc::new(nomi_agent::output::null_sink::NullSink),
    )
    .provider(Arc::new(Capture(requests.clone())))
    .host_skills(loaded.package_skills().to_vec())
    .build()
    .await
    .unwrap();
    assert!(
        built
            .engine
            .slash_command_list()
            .iter()
            .any(|(name, _)| name == &format!("skill:{SKILL}"))
    );
    built
        .engine
        .execute_turn(&format!("/skill:{SKILL} selected"), "turn-1")
        .await
        .unwrap();
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].messages.iter().flat_map(|message| &message.content).any(|block| {
        matches!(block, ContentBlock::Text { text } if text.contains("Package guidance selected"))
    }));
    drop(captured);
    kernel.replace_all(Vec::new()).unwrap();
    let error = built
        .engine
        .execute_turn(&format!("/skill:{SKILL} withdrawn"), "turn-2")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no longer available"), "{error}");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "withdrawn source cannot reach model"
    );
}

#[tokio::test]
async fn frozen_skill_detects_same_body_artifact_upgrade_and_withdrawal() {
    let (registration, schemas, resolver) = fixture('a');
    let kernel = Arc::new(
        KernelRegistry::new(
            policy_for(PluginSourceKind::ManagedLocal),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let compiled = Arc::new(compiled(&registry));
    let old_lock = compiled.content().skill_locks[0].clone();
    let loaded = session(kernel.clone(), (*compiled).clone(), schemas.clone())
        .await
        .with_package_skills(kernel.clone(), compiled.clone(), Arc::new(resolver))
        .await
        .unwrap();
    assert_eq!(loaded.package_skills().len(), 1);
    assert_eq!(
        loaded.package_skills()[0]
            .read(Some("yes"), None, None)
            .await
            .unwrap(),
        "Package guidance yes"
    );
    let (replacement, _, resolver) = fixture('b');
    kernel.replace_all(vec![replacement]).unwrap();
    assert!(
        loaded.package_skills()[0]
            .read(None, None, None)
            .await
            .unwrap_err()
            .contains("frozen source")
    );
    assert_eq!(
        compiled.content().skill_locks[0],
        old_lock,
        "old Snapshot must remain immutable"
    );
    let retry = loaded.clone();
    assert!(
        retry
            .with_package_skills(kernel.clone(), compiled, Arc::new(resolver))
            .await
            .is_err()
    );
    kernel.replace_all(Vec::new()).unwrap();
    assert!(
        loaded.package_skills()[0]
            .read(None, None, None)
            .await
            .unwrap_err()
            .contains("no longer available")
    );
}

#[tokio::test]
async fn bad_body_missing_files_and_withdrawal_during_io_fail_closed() {
    for scenario in ["digest", "missing", "extra", "definition", "withdraw"] {
        let (registration, schemas, mut resolver) = fixture('a');
        let kernel = Arc::new(
            KernelRegistry::new(
                policy_for(PluginSourceKind::ManagedLocal),
                Arc::new(InMemoryPluginStatePersistence::new()),
            )
            .unwrap(),
        );
        let registry = kernel.replace_all(vec![registration]).unwrap();
        let compiled = Arc::new(compiled(&registry));
        let session = session(kernel.clone(), (*compiled).clone(), schemas).await;
        match scenario {
            "digest" => {
                resolver
                    .files
                    .get_mut("resources/guide.md")
                    .unwrap()
                    .push(b'!');
            }
            "missing" => resolver.files.clear(),
            "extra" => {
                resolver.files.insert("secret".into(), vec![]);
            }
            "definition" => resolver.definition.display.description = "changed".into(),
            "withdraw" => resolver.withdraw = Some(kernel.clone()),
            _ => unreachable!(),
        }
        assert!(
            session
                .with_package_skills(kernel, compiled, Arc::new(resolver))
                .await
                .is_err(),
            "{scenario}"
        );
    }
}

#[tokio::test]
async fn skill_descriptor_cannot_attach_to_another_snapshot() {
    let (registration, schemas, resolver) = fixture('a');
    let kernel = Arc::new(
        KernelRegistry::new(
            policy_for(PluginSourceKind::ManagedLocal),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .unwrap(),
    );
    let registry = kernel.replace_all(vec![registration]).unwrap();
    let mut compiled = compiled(&registry);
    let session = session(kernel.clone(), compiled.clone(), schemas).await;
    compiled.envelope.snapshot_ref.snapshot_id = "another-snapshot".into();
    assert!(
        session
            .with_package_skills(kernel, Arc::new(compiled), Arc::new(resolver))
            .await
            .unwrap_err()
            .to_string()
            .contains("differs from its Session")
    );
}

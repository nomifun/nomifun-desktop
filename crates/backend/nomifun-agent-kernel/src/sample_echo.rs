use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload, ArtifactEnvelope,
    CapabilityActionDescriptor, CapabilityContributions, CapabilityExposure, CapabilityId,
    CapabilityKind, CapabilityManifest, CapabilityRef, CancellationDescriptor, CanonicalSchemaRef,
    DeclaredServiceViewDescriptor, DigestHex, EffectClass, HostPortId, HostPortRef,
    InProcessEntrypointMetadata, LocalizedMetadata, LogicalArtifactRef, ManagedTaskRegistrationDescriptor,
    McpServerId, McpToolCapabilityMapping, McpToolKey, OperationId, PackageContributions,
    PackageId, PackageManifest, PackageRef, PlatformConstraint, PluginBootCriticality,
    PluginBootState, PluginContextDescriptor, PluginDesiredState, PluginEffectiveState,
    PluginIdentityDescriptor, PluginMountId, PluginRegistrarDescriptor, PluginRegistrarOperation,
    PluginRegistrationMetadata, PluginSourceKind, PluginSourceMetadata,
    PluginStateCompareAndSwapOutcome, PluginStateHandleDescriptor, PluginStateMethod,
    PresetRevisionRef, PrincipalRef, ResourceBindingId, ResourceId, ResourceKind,
    RuntimeProfileKind, RuntimeTarget, ScopeKey, ServiceHandleDescriptor, ServiceKeyRef,
    ServiceProvision, ServiceRequirement, SkillDefinition, SkillId, SkillRef, StrictJsonValue,
    ToolPresentationKind, TypedResourceBinding, UserId, ValidatedPluginConfig, VersionString,
    digest_bytes, digest_payload,
};
use serde_json::json;

use crate::{
    ActivationOutcome, AgentPresetCompiler, CapabilityHandler, CapabilityInvocationContext,
    CapabilityInvocationRequest, CompileRequest, CompilerEnvironment, CompletedTurnBoundary,
    HostPluginStateApi, InMemoryPluginStatePersistence, KernelError, KernelRegistry,
    MaterializationPolicy, PluginRegistration, PluginStatePersistence, ServiceKey,
    SessionCapabilityState,
};

const SAMPLE_PACKAGE: &str = "sample.echo";
const SAMPLE_MOUNT: &str = "sample-echo";
const SAMPLE_CAPABILITY: &str = "sample.echo";
const SAMPLE_ACTION: &str = "sample.echo.invoke";
const SAMPLE_SKILL: &str = "sample.echo-guidance";
const SAMPLE_RESOURCE_KIND: &str = "sample.echo.target";
const VERSION: &str = "1.0.0";

fn package_ref(package_id: &str) -> PackageRef {
    PackageRef {
        id: PackageId::from(package_id),
        version: VersionString::from(VERSION),
    }
}

fn host_port(id: &str) -> HostPortRef {
    HostPortRef {
        id: HostPortId::from(id),
        version: VersionString::from(VERSION),
    }
}

fn display(name: &str, description: &str) -> LocalizedMetadata {
    LocalizedMetadata {
        name: name.to_owned(),
        description: description.to_owned(),
        localized_names: BTreeMap::new(),
        localized_descriptions: BTreeMap::new(),
    }
}

fn registration_for(
    package_id: &str,
    mount_id: &str,
    capability_id: &str,
    skill_id: &str,
    server_id: &str,
    prefix: &str,
) -> PluginRegistration {
    let package = package_ref(package_id);
    let capability_ref = CapabilityRef {
        id: CapabilityId::from(capability_id),
        version: VersionString::from(VERSION),
    };
    let config_schema = StrictJsonValue(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "prefix": {
                "type": "string",
                "maxLength": 32
            }
        },
        "required": ["prefix"]
    }));
    let action_input_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "message": {"type": "string", "maxLength": 256}
        },
        "required": ["message"]
    });
    let action_output_schema = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "echo": {"type": "string"},
            "count": {"type": "integer", "minimum": 1}
        },
        "required": ["echo", "count"]
    });
    let action_input_schema_digest = digest_payload(&action_input_schema).unwrap();
    let action_output_schema_digest = digest_payload(&action_output_schema).unwrap();
    let capability = CapabilityManifest {
        id: capability_ref.id.clone(),
        version: capability_ref.version.clone(),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: display("Sample Echo", "Echo a message through the capability host."),
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: BTreeSet::from(["test".to_owned()]),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({
            "type": "object",
            "additionalProperties": false
        })),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from(SAMPLE_ACTION),
                input_schema: CanonicalSchemaRef::from(format!(
                    "schema://{capability_id}/input@1#{}",
                    action_input_schema_digest.as_ref()
                )),
                output_schema: CanonicalSchemaRef::from(format!(
                    "schema://{capability_id}/output@1#{}",
                    action_output_schema_digest.as_ref()
                )),
                effect_class: EffectClass::WriteReversible,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            context_schema_refs: Vec::new(),
            event_schema_refs: Vec::new(),
            resource_kinds: BTreeSet::from([ResourceKind::from(
                SAMPLE_RESOURCE_KIND,
            )]),
            host_ports: Vec::new(),
        },
    };
    let skill = SkillDefinition {
        id: SkillId::from(skill_id),
        version: VersionString::from(VERSION),
        package: package.clone(),
        display: display(
            "Sample Echo Guidance",
            "Use sample.echo to return the exact requested message.",
        ),
        body_ref: LogicalArtifactRef {
            artifact_id: nomifun_agent_contracts::ArtifactId::from(format!(
                "{skill_id}.body"
            )),
            normalized_relative_path: "skills/sample-echo/SKILL.md".to_owned(),
            digest: digest_bytes(b"Use sample.echo with one message."),
        },
        resources: Vec::new(),
        requires_capabilities: vec![capability_ref.clone()],
        supported_surfaces: BTreeSet::from(["test".to_owned()]),
    };
    let mcp_mapping = McpToolCapabilityMapping {
        package: package.clone(),
        server_id: McpServerId::from(server_id),
        canonical_tool_key: McpToolKey::from(format!("{server_id}.echo")),
        schema_digest: action_input_schema_digest,
        capability: capability_ref.clone(),
        materialization_version: VersionString::from(VERSION),
    };
    let manifest = PackageManifest {
        schema_version: VersionString::from(VERSION),
        host_contract_version: VersionString::from(VERSION),
        package_id: package.id.clone(),
        package_version: package.version.clone(),
        display: display("Sample Echo Package", "CI-only source-neutral fixture."),
        package_dependencies: Vec::new(),
        requires_runtime_features: Vec::new(),
        config_schema: config_schema.clone(),
        provides_services: Vec::new(),
        requires_services: Vec::new(),
        entrypoint: InProcessEntrypointMetadata {
            entrypoint_profile: "trusted-in-process".to_owned(),
            entrypoint_id: format!("{package_id}.entrypoint"),
            contract_version: VersionString::from(VERSION),
        },
        contributions: PackageContributions {
            capabilities: vec![capability],
            skills: vec![skill],
            mcp_tools: vec![mcp_mapping],
        },
    };
    let source = PluginSourceMetadata {
        source_kind: PluginSourceKind::TestFixture,
        source_identity: package_id.to_owned(),
        source_digest: None,
    };
    let cancellation_port = host_port("host.plugin.cancel");
    let task_port = host_port("host.plugin.tasks");
    let identity = PluginIdentityDescriptor {
        package: package.clone(),
        mount_id: PluginMountId::from(mount_id),
    };
    let metadata = PluginRegistrationMetadata {
        manifest: ArtifactEnvelope::new(manifest).unwrap(),
        mount_id: identity.mount_id.clone(),
        source: source.clone(),
        boot_state: PluginBootState {
            criticality: PluginBootCriticality::Required,
            desired_state: PluginDesiredState::Enabled,
            effective_state: PluginEffectiveState::Active,
            diagnostic_code: None,
        },
        registrar: PluginRegistrarDescriptor {
            identity: identity.clone(),
            allowed_operations: BTreeSet::from([
                PluginRegistrarOperation::ContributeCapability,
                PluginRegistrarOperation::ContributeSkill,
                PluginRegistrarOperation::ContributeMcpToolMapping,
                PluginRegistrarOperation::BindHostPort,
            ]),
            declared_capability_ids: BTreeSet::from([capability_ref.id.clone()]),
            declared_skill_ids: BTreeSet::from([SkillId::from(skill_id)]),
            declared_mcp_tool_keys: BTreeSet::from([McpToolKey::from(format!(
                "{server_id}.echo"
            ))]),
            declared_service_keys: BTreeSet::new(),
            declared_host_ports: BTreeSet::from([
                cancellation_port.id.clone(),
                task_port.id.clone(),
            ]),
        },
        context: PluginContextDescriptor {
            identity,
            source,
            validated_config: ValidatedPluginConfig {
                schema_digest: digest_payload(&config_schema).unwrap(),
                config_revision: 1,
                value: StrictJsonValue(json!({"prefix": prefix})),
            },
            state: PluginStateHandleDescriptor {
                package_id: package.id,
                mount_id: PluginMountId::from(mount_id),
                methods: PluginStateMethod::REQUIRED.into_iter().collect(),
            },
            declared_services: DeclaredServiceViewDescriptor::default(),
            host_ports: Vec::new(),
            typed_command_ports: Vec::new(),
            domain_outbox_ports: Vec::new(),
            cancellation: CancellationDescriptor {
                cancellation_port,
                scope_key: ScopeKey::from(format!("mount:{mount_id}")),
            },
            managed_task_registration: ManagedTaskRegistrationDescriptor {
                registrar_port: task_port,
                scope_key: ScopeKey::from(format!("mount:{mount_id}")),
            },
        },
    };
    let mut registration = PluginRegistration::new(metadata);
    registration
        .add_capability_handler(
            capability_ref.id,
            Arc::new(EchoHandler {
                prefix: prefix.to_owned(),
            }),
        )
        .unwrap();
    registration
}

fn sample_registration(prefix: &str) -> PluginRegistration {
    registration_for(
        SAMPLE_PACKAGE,
        SAMPLE_MOUNT,
        SAMPLE_CAPABILITY,
        SAMPLE_SKILL,
        "sample.echo.server",
        prefix,
    )
}

struct EchoHandler {
    prefix: String,
}

#[async_trait]
impl CapabilityHandler for EchoHandler {
    async fn invoke(
        &self,
        context: CapabilityInvocationContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, KernelError> {
        if context.action_id.as_ref() != SAMPLE_ACTION {
            return Err(KernelError::ActionNotDeclared {
                capability_id: context.capability_id,
                action_id: context.action_id,
            });
        }
        let object = input
            .0
            .as_object()
            .ok_or_else(|| KernelError::CapabilityExecution {
                reason: "sample.echo input must be an object".to_owned(),
            })?;
        if object.len() != 1 {
            return Err(KernelError::CapabilityExecution {
                reason: "sample.echo input accepts only `message`".to_owned(),
            });
        }
        let message = object
            .get("message")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| KernelError::CapabilityExecution {
                reason: "sample.echo requires string `message`".to_owned(),
            })?;
        let state_key = nomifun_agent_contracts::StateKey::from("invoke-count");
        let format_version = VersionString::from(VERSION);
        for _ in 0..8 {
            let current = context
                .state
                .get(&context.state_scope_key, &state_key)
                .await
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })?;
            let expected_revision =
                current.as_ref().map(|entry| entry.revision).unwrap_or(0);
            let count = current
                .as_ref()
                .and_then(|entry| entry.value.0.as_u64())
                .unwrap_or(0)
                + 1;
            match context
                .state
                .compare_and_swap(
                    &context.state_scope_key,
                    &state_key,
                    expected_revision,
                    &format_version,
                    Some(StrictJsonValue(json!(count))),
                )
                .await
                .map_err(|error| KernelError::CapabilityExecution {
                    reason: error.to_string(),
                })?
            {
                PluginStateCompareAndSwapOutcome::Applied { .. } => {
                    return Ok(StrictJsonValue(json!({
                        "echo": format!("{}{}", self.prefix, message),
                        "count": count
                    })));
                }
                PluginStateCompareAndSwapOutcome::Conflict { .. } => continue,
            }
        }
        Err(KernelError::CapabilityExecution {
            reason: "sample.echo state remained contended".to_owned(),
        })
    }
}

fn principal(id: &str) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: id.to_owned(),
    }
}

fn resource_binding(owner_id: &str) -> TypedResourceBinding {
    TypedResourceBinding {
        binding_id: ResourceBindingId::from("sample-echo-target"),
        resource_kind: ResourceKind::from(SAMPLE_RESOURCE_KIND),
        resource_id: ResourceId::from("echo-target-1"),
        owner_id: owner_id.to_owned(),
        operations: BTreeSet::from(["invoke".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }
}

fn sample_revision(owner_id: &str) -> AgentPresetRevision {
    let payload = AgentPresetRevisionPayload {
        schema_version: VersionString::from(VERSION),
        surfaces: BTreeSet::from(["test".to_owned()]),
        model_route_refs: BTreeMap::new(),
        initial_capabilities: Vec::new(),
        on_demand_capabilities: vec![nomifun_agent_contracts::CapabilitySelection {
            capability: CapabilityRef {
                id: CapabilityId::from(SAMPLE_CAPABILITY),
                version: VersionString::from(VERSION),
            },
            required: true,
            exposure: CapabilityExposure::Discoverable,
            action_allowlist: BTreeSet::from([ActionId::from(SAMPLE_ACTION)]),
            resource_binding_refs: vec![ResourceBindingId::from(
                "sample-echo-target",
            )],
            destination_constraints: BTreeSet::new(),
            context_budget_override: None,
            tool_budget_override: None,
            config: StrictJsonValue(json!({})),
        }],
        skill_bindings: vec![SkillRef {
            id: SkillId::from(SAMPLE_SKILL),
            version: VersionString::from(VERSION),
        }],
        resource_bindings: vec![resource_binding(owner_id)],
        persona: "Echo fixture".to_owned(),
        instructions: "Use the selected echo capability.".to_owned(),
        context_policy: StrictJsonValue(json!({})),
        execution_constraints: StrictJsonValue(json!({})),
        runtime_budget: StrictJsonValue(json!({})),
    };
    let revision_digest = digest_payload(&payload).unwrap();
    AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: AgentPresetId::from("sample.echo.preset"),
            revision: 1,
            revision_digest,
        },
        payload,
        created_by: UserId::from(owner_id),
        created_at_ms: 1,
        reason: Some("sample fixture".to_owned()),
    }
}

fn compiler_environment(target_digest: DigestHex) -> CompilerEnvironment {
    CompilerEnvironment {
        resolver_version: VersionString::from(VERSION),
        required_runtime_protocol_version: VersionString::from(VERSION),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: DigestHex::from("runtime-features"),
        available_runtime_features: BTreeSet::new(),
        canonical_schema_manifest_digest: DigestHex::from("schema-manifest"),
        target_contribution_manifest_digest: target_digest,
        host_target: RuntimeTarget::from("windows-desktop-x64"),
        host_surface: "desktop".to_owned(),
        availability_evidence_revision: "sample-fixture".to_owned(),
    }
}

fn compile_request(revision: AgentPresetRevision, owner: PrincipalRef) -> CompileRequest {
    CompileRequest {
        revision,
        principal: owner,
        scene: "sample".to_owned(),
        surface: "test".to_owned(),
        audience: "test".to_owned(),
        created_at_ms: 2,
        resolver_run_id: OperationId::from("resolve-sample"),
    }
}

fn invocation(
    snapshot: &crate::CompiledSnapshot,
    owner: PrincipalRef,
    active_generation: u64,
    message: &str,
) -> CapabilityInvocationRequest {
    CapabilityInvocationRequest {
        principal: owner.clone(),
        session_owner: owner,
        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
        active_set_generation: active_generation,
        capability_id: CapabilityId::from(SAMPLE_CAPABILITY),
        action_id: ActionId::from(SAMPLE_ACTION),
        resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
            "sample-echo-target",
        )]),
        state_scope_key: ScopeKey::from("session:sample-1"),
        input: StrictJsonValue(json!({"message": message})),
    }
}

fn refresh_manifest(registration: &mut PluginRegistration) {
    let manifest = registration.metadata.manifest.payload.clone();
    registration.metadata.manifest = ArtifactEnvelope::new(manifest).unwrap();
}

trait TestEchoService: Send + Sync {
    fn echo(&self, value: &str) -> String;
}

struct TestEchoServiceImpl;

impl TestEchoService for TestEchoServiceImpl {
    fn echo(&self, value: &str) -> String {
        format!("service:{value}")
    }
}

fn add_service_provider<T>(
    registration: &mut PluginRegistration,
    key: &ServiceKey<T>,
    service: Arc<T>,
) where
    T: ?Sized + Send + Sync + 'static,
{
    registration
        .metadata
        .manifest
        .payload
        .provides_services
        .push(ServiceProvision {
            service: key.reference().clone(),
        });
    registration
        .metadata
        .registrar
        .allowed_operations
        .insert(PluginRegistrarOperation::ProvideService);
    registration
        .metadata
        .registrar
        .declared_service_keys
        .insert(key.reference().id.clone());
    registration
        .metadata
        .context
        .declared_services
        .provided_services
        .push(key.reference().clone());
    registration.provide_service(key, service).unwrap();
    refresh_manifest(registration);
}

fn add_service_requirement(
    registration: &mut PluginRegistration,
    service: ServiceKeyRef,
    provider_package: PackageRef,
    provider_mount_id: PluginMountId,
) {
    registration
        .metadata
        .manifest
        .payload
        .requires_services
        .push(ServiceRequirement {
            service: service.clone(),
        });
    registration
        .metadata
        .context
        .declared_services
        .required_service_handles
        .push(ServiceHandleDescriptor {
            service,
            provider_package,
            provider_mount_id,
        });
    refresh_manifest(registration);
}

#[tokio::test]
async fn sample_echo_uses_materialize_compile_activate_authorize_invoke_and_restart_chain() {
    let persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::clone(&persistence) as Arc<dyn PluginStatePersistence>,
    )
    .unwrap();
    let materialized = registry
        .replace_all(vec![sample_registration("prefix:")])
        .unwrap();
    assert!(materialized.packages.contains_key(&PackageId::from(SAMPLE_PACKAGE)));
    assert!(materialized
        .capabilities
        .contains_key(&CapabilityId::from(SAMPLE_CAPABILITY)));
    assert!(materialized.skills.contains_key(&SkillId::from(SAMPLE_SKILL)));
    assert!(materialized
        .mcp_for_capability(&CapabilityId::from(SAMPLE_CAPABILITY))
        .is_some());

    let owner = principal("user-a");
    let revision = sample_revision(&owner.principal_id);
    let compiled = AgentPresetCompiler::compile(
        &materialized,
        &compiler_environment(materialized.registry_digest.clone()),
        compile_request(revision.clone(), owner.clone()),
    )
    .unwrap();
    let replayed = AgentPresetCompiler::compile(
        &materialized,
        &compiler_environment(materialized.registry_digest.clone()),
        compile_request(revision, owner.clone()),
    )
    .unwrap();
    assert_eq!(
        compiled.snapshot_ref().snapshot_digest,
        replayed.snapshot_ref().snapshot_digest
    );
    assert_eq!(compiled.content().skill_locks.len(), 1);
    assert_eq!(compiled.content().mcp_tool_locks.len(), 1);
    assert_eq!(compiled.content().compact_on_demand_index.len(), 1);

    let active = SessionCapabilityState::new(&compiled);
    assert_eq!(active.search("echo", 8).unwrap().len(), 1);
    let inactive = active.snapshot().unwrap();
    assert!(matches!(
        registry
            .invoke(
                &compiled,
                &inactive,
                invocation(&compiled, owner.clone(), 0, "hello"),
            )
            .await,
        Err(KernelError::CapabilityNotActive { .. })
    ));
    assert_eq!(
        active
            .activate_at_boundary(
                0,
                &CapabilityId::from(SAMPLE_CAPABILITY),
                CompletedTurnBoundary::committed(OperationId::from("turn-1")),
            )
            .unwrap(),
        ActivationOutcome::Activated {
            generation: 1,
            activated_bundle: vec![CapabilityId::from(SAMPLE_CAPABILITY)],
        }
    );
    let active_snapshot = active.snapshot().unwrap();
    let first = registry
        .invoke(
            &compiled,
            &active_snapshot,
            invocation(&compiled, owner.clone(), 1, "hello"),
        )
        .await
        .unwrap();
    assert_eq!(first.0, json!({"echo": "prefix:hello", "count": 1}));

    let restarted = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::clone(&persistence) as Arc<dyn PluginStatePersistence>,
    )
    .unwrap();
    let restarted_materialized = restarted
        .replace_all(vec![sample_registration("prefix:")])
        .unwrap();
    let restarted_compiled = AgentPresetCompiler::compile(
        &restarted_materialized,
        &compiler_environment(restarted_materialized.registry_digest.clone()),
        compile_request(sample_revision(&owner.principal_id), owner.clone()),
    )
    .unwrap();
    let restarted_active = SessionCapabilityState::new(&restarted_compiled);
    restarted_active
        .activate_at_boundary(
            0,
            &CapabilityId::from(SAMPLE_CAPABILITY),
            CompletedTurnBoundary::committed(OperationId::from("turn-2")),
        )
        .unwrap();
    let second = restarted
        .invoke(
            &restarted_compiled,
            &restarted_active.snapshot().unwrap(),
            invocation(&restarted_compiled, owner, 1, "again"),
        )
        .await
        .unwrap();
    assert_eq!(second.0, json!({"echo": "prefix:again", "count": 2}));
}

#[tokio::test]
async fn authority_rejects_wrong_principal_and_resource_without_invoking() {
    let persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        persistence,
    )
    .unwrap();
    let materialized = registry
        .replace_all(vec![sample_registration("")])
        .unwrap();
    let owner = principal("owner");
    let compiled = AgentPresetCompiler::compile(
        &materialized,
        &compiler_environment(materialized.registry_digest.clone()),
        compile_request(sample_revision("owner"), owner.clone()),
    )
    .unwrap();
    let active = SessionCapabilityState::new(&compiled);
    active
        .activate_at_boundary(
            0,
            &CapabilityId::from(SAMPLE_CAPABILITY),
            CompletedTurnBoundary::committed(OperationId::from("turn")),
        )
        .unwrap();
    let active = active.snapshot().unwrap();

    let mut wrong_principal = invocation(&compiled, principal("other"), 1, "x");
    wrong_principal.session_owner = owner.clone();
    assert!(matches!(
        registry
            .invoke(&compiled, &active, wrong_principal)
            .await,
        Err(KernelError::ResourceOwnerMismatch { .. })
    ));

    let mut wrong_resource = invocation(&compiled, owner, 1, "x");
    wrong_resource.resource_binding_ids =
        BTreeSet::from([ResourceBindingId::from("wrong")]);
    assert!(matches!(
        registry
            .invoke(&compiled, &active, wrong_resource)
            .await,
        Err(KernelError::ResourceBindingMissing { .. })
    ));
}

#[test]
fn invalid_config_and_duplicate_capability_do_not_publish_partial_generation() {
    let persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        persistence,
    )
    .unwrap();
    let first = registry
        .replace_all(vec![sample_registration("ok:")])
        .unwrap();

    let mut bad_config = sample_registration("ok:");
    bad_config.metadata.context.validated_config.value =
        StrictJsonValue(json!({"prefix": "ok:", "unknown": true}));
    assert!(matches!(
        registry.replace_all(vec![bad_config]),
        Err(KernelError::InvalidPluginConfig { .. })
    ));
    let after_bad_config = registry.snapshot().unwrap();
    assert_eq!(after_bad_config.generation, first.generation);
    assert_eq!(after_bad_config.registry_digest, first.registry_digest);

    let duplicate = registration_for(
        "sample.echo.other",
        "sample-echo-other",
        SAMPLE_CAPABILITY,
        "sample.echo-other-guidance",
        "sample.echo.other.server",
        "other:",
    );
    assert!(matches!(
        registry.replace_all(vec![sample_registration("ok:"), duplicate]),
        Err(KernelError::DuplicateCapability { .. })
    ));
    assert_eq!(registry.snapshot().unwrap().generation, first.generation);
}

#[test]
fn dependency_skill_and_service_faults_fail_closed() {
    let persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        persistence,
    )
    .unwrap();

    let mut missing_capability = sample_registration("");
    missing_capability.metadata.manifest.payload.contributions.capabilities[0]
        .requires
        .push(CapabilityRef {
            id: CapabilityId::from("missing.capability"),
            version: VersionString::from(VERSION),
        });
    refresh_manifest(&mut missing_capability);
    assert!(matches!(
        registry.replace_all(vec![missing_capability]),
        Err(KernelError::MissingCapabilityDependency { .. })
    ));

    let mut missing_service = sample_registration("");
    add_service_requirement(
        &mut missing_service,
        ServiceKeyRef {
            id: nomifun_agent_contracts::ServiceKeyId::from("service.missing"),
            version: VersionString::from(VERSION),
        },
        package_ref("missing.provider"),
        PluginMountId::from("missing-provider"),
    );
    assert!(matches!(
        registry.replace_all(vec![missing_service]),
        Err(KernelError::MissingService { .. })
    ));

    let mut skill_without_capability = sample_revision("owner");
    skill_without_capability.payload.on_demand_capabilities.clear();
    skill_without_capability.reference.revision_digest =
        digest_payload(&skill_without_capability.payload).unwrap();
    let materialized = registry
        .replace_all(vec![sample_registration("")])
        .unwrap();
    assert!(matches!(
        AgentPresetCompiler::compile(
            &materialized,
            &compiler_environment(materialized.registry_digest.clone()),
            compile_request(skill_without_capability, principal("owner")),
        ),
        Err(KernelError::SkillRequiresCapability { .. })
    ));
}

#[test]
fn typed_service_wiring_is_exact_and_service_cycles_fail() {
    let service_key =
        ServiceKey::<dyn TestEchoService>::new("service.sample.echo", VERSION);
    let mut provider = registration_for(
        "sample.provider",
        "sample-provider",
        "sample.provider.capability",
        "sample.provider.skill",
        "sample.provider.server",
        "",
    );
    add_service_provider(
        &mut provider,
        &service_key,
        Arc::new(TestEchoServiceImpl) as Arc<dyn TestEchoService>,
    );
    let mut consumer = registration_for(
        "sample.consumer",
        "sample-consumer",
        "sample.consumer.capability",
        "sample.consumer.skill",
        "sample.consumer.server",
        "",
    );
    add_service_requirement(
        &mut consumer,
        service_key.reference().clone(),
        package_ref("sample.provider"),
        PluginMountId::from("sample-provider"),
    );
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap();
    registry
        .replace_all(vec![consumer, provider])
        .unwrap();
    let view = registry
        .declared_service_view(&PluginMountId::from("sample-consumer"))
        .unwrap()
        .unwrap();
    assert_eq!(
        view.require(&service_key).unwrap().echo("ok"),
        "service:ok"
    );

    let key_a = ServiceKey::<String>::new("service.a", VERSION);
    let key_b = ServiceKey::<String>::new("service.b", VERSION);
    let mut a = registration_for(
        "sample.a",
        "sample-a",
        "sample.a.capability",
        "sample.a.skill",
        "sample.a.server",
        "",
    );
    let mut b = registration_for(
        "sample.b",
        "sample-b",
        "sample.b.capability",
        "sample.b.skill",
        "sample.b.server",
        "",
    );
    add_service_provider(&mut a, &key_a, Arc::new("a".to_owned()));
    add_service_provider(&mut b, &key_b, Arc::new("b".to_owned()));
    add_service_requirement(
        &mut a,
        key_b.reference().clone(),
        package_ref("sample.b"),
        PluginMountId::from("sample-b"),
    );
    add_service_requirement(
        &mut b,
        key_a.reference().clone(),
        package_ref("sample.a"),
        PluginMountId::from("sample-a"),
    );
    assert!(matches!(
        registry.replace_all(vec![a, b]),
        Err(KernelError::ServiceDependencyCycle)
    ));
}

#[test]
fn materialization_is_order_independent_and_stable_policy_excludes_test_fixture() {
    let left = registration_for(
        "sample.left",
        "sample-left",
        "sample.left.capability",
        "sample.left.skill",
        "sample.left.server",
        "left:",
    );
    let right = registration_for(
        "sample.right",
        "sample-right",
        "sample.right.capability",
        "sample.right.skill",
        "sample.right.server",
        "right:",
    );
    let first = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap()
    .replace_all(vec![left.clone(), right.clone()])
    .unwrap();
    let second = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap()
    .replace_all(vec![right, left])
    .unwrap();
    assert_eq!(first.registry_digest, second.registry_digest);
    assert_eq!(first.package_start_order, second.package_start_order);
    assert_eq!(
        first.service_dag.topological_start_order,
        second.service_dag.topological_start_order
    );

    let production = KernelRegistry::new(
        MaterializationPolicy::stable(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap();
    assert!(matches!(
        production.replace_all(vec![sample_registration("")]),
        Err(KernelError::SourceNotAllowed { .. })
    ));
}

#[test]
fn package_cycles_and_duplicate_service_providers_fail_before_publish() {
    let mut left = registration_for(
        "sample.left",
        "sample-left",
        "sample.left.capability",
        "sample.left.skill",
        "sample.left.server",
        "",
    );
    let mut right = registration_for(
        "sample.right",
        "sample-right",
        "sample.right.capability",
        "sample.right.skill",
        "sample.right.server",
        "",
    );
    left.metadata
        .manifest
        .payload
        .package_dependencies
        .push(package_ref("sample.right"));
    right
        .metadata
        .manifest
        .payload
        .package_dependencies
        .push(package_ref("sample.left"));
    refresh_manifest(&mut left);
    refresh_manifest(&mut right);
    let registry = KernelRegistry::new(
        MaterializationPolicy::stable_with_test_fixtures(VERSION),
        Arc::new(InMemoryPluginStatePersistence::new()),
    )
    .unwrap();
    assert!(matches!(
        registry.replace_all(vec![left, right]),
        Err(KernelError::PackageDependencyCycle)
    ));
    assert_eq!(registry.snapshot().unwrap().generation, 0);

    let service = ServiceKey::<String>::new("service.duplicate", VERSION);
    let mut first = registration_for(
        "sample.provider-a",
        "sample-provider-a",
        "sample.provider-a.capability",
        "sample.provider-a.skill",
        "sample.provider-a.server",
        "",
    );
    let mut second = registration_for(
        "sample.provider-b",
        "sample-provider-b",
        "sample.provider-b.capability",
        "sample.provider-b.skill",
        "sample.provider-b.server",
        "",
    );
    add_service_provider(&mut first, &service, Arc::new("a".to_owned()));
    add_service_provider(&mut second, &service, Arc::new("b".to_owned()));
    assert!(matches!(
        registry.replace_all(vec![first, second]),
        Err(KernelError::DuplicateServiceProvider { .. })
    ));
    assert_eq!(registry.snapshot().unwrap().generation, 0);
}
